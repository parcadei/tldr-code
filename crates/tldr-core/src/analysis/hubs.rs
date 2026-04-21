//! Hub detection algorithms for call graph centrality analysis
//!
//! This module provides centrality measures to identify "hub" functions
//! that are critical to the codebase - changes to them affect many others.
//!
//! ## Implemented Measures
//!
//! - `compute_in_degree`: Normalized count of callers (who depends on this?)
//! - `compute_out_degree`: Normalized count of callees (what does this depend on?)
//! - `compute_pagerank`: Recursive importance via PageRank algorithm
//! - `compute_betweenness`: Bridge detection via betweenness centrality
//!
//! ## Normalization
//!
//! All centrality scores are normalized to [0, 1] using:
//! - `in_degree(v) = |callers| / (n - 1)` where n = total nodes
//! - `out_degree(v) = |callees| / (n - 1)`
//! - `pagerank`: Normalized by dividing by max value after convergence
//! - `betweenness`: Normalized by (n-1)(n-2) for directed graphs, then by max
//!
//! ## Risk Levels
//!
//! Based on composite score:
//! - Critical: >= 0.8
//! - High: >= 0.6
//! - Medium: >= 0.4
//! - Low: < 0.4
//!
//! ## Composite Score Weights
//!
//! Default weights (from spec):
//! - in_degree: 0.25
//! - out_degree: 0.25
//! - betweenness: 0.30
//! - pagerank: 0.20
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::analysis::hubs::{compute_in_degree, compute_out_degree, compute_pagerank, compute_betweenness, HubScore, RiskLevel, PageRankConfig, BetweennessConfig};
//! use tldr_core::callgraph::graph_utils::{build_forward_graph, build_reverse_graph, collect_nodes};
//!
//! let forward = build_forward_graph(&call_graph);
//! let reverse = build_reverse_graph(&call_graph);
//! let nodes = collect_nodes(&call_graph);
//!
//! let in_degrees = compute_in_degree(&nodes, &reverse);
//! let out_degrees = compute_out_degree(&nodes, &forward);
//!
//! // PageRank with default config
//! let pr_config = PageRankConfig::default();
//! let pagerank_result = compute_pagerank(&nodes, &reverse, &forward, &pr_config);
//!
//! // Betweenness with sampling for large graphs
//! let bc_config = BetweennessConfig::default();
//! let betweenness = compute_betweenness(&nodes, &forward, &bc_config);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

use crate::types::FunctionRef;

// =============================================================================
// Configuration Types
// =============================================================================

/// Configuration for PageRank computation (T2 mitigation)
///
/// Default values tuned for code call graphs:
/// - damping: 0.85 (standard)
/// - max_iterations: 150 (increased for deep chains)
/// - epsilon: 1e-5 (faster convergence with negligible accuracy loss)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRankConfig {
    /// Damping factor (probability of following edges vs random jump)
    /// Default: 0.85
    pub damping: f64,
    /// Maximum iterations before stopping
    /// Default: 150 (T2 mitigation: increased from 100)
    pub max_iterations: usize,
    /// Convergence threshold (stop when max delta < epsilon)
    /// Default: 1e-5 (T2 mitigation: larger for faster convergence)
    pub epsilon: f64,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            max_iterations: 150,
            epsilon: 1e-5,
        }
    }
}

/// Configuration for betweenness centrality (T4 mitigation)
///
/// For large graphs, betweenness is O(V*E) which can be prohibitive.
/// Sampling uses k random sources to approximate betweenness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BetweennessConfig {
    /// Sample size for approximation. None = compute from all sources
    /// For graphs > 1000 nodes, recommend Some(100) per Brandes 2008
    pub sample_size: Option<usize>,
    /// Maximum nodes before auto-skipping betweenness
    /// Default: 5000 (warn but still compute with sampling)
    pub max_nodes: usize,
}

impl Default for BetweennessConfig {
    fn default() -> Self {
        Self {
            sample_size: None,
            max_nodes: 5000,
        }
    }
}

impl BetweennessConfig {
    /// Create config with sampling enabled
    pub fn with_sampling(sample_size: usize) -> Self {
        Self {
            sample_size: Some(sample_size),
            max_nodes: 5000,
        }
    }
}

/// Result of PageRank computation with convergence info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRankResult {
    /// PageRank scores for each node, normalized to [0, 1]
    pub scores: HashMap<FunctionRef, f64>,
    /// Number of iterations used
    pub iterations_used: usize,
    /// Whether the algorithm converged (delta < epsilon)
    pub converged: bool,
}

// =============================================================================
// Types
// =============================================================================

/// Risk level classification for hub functions
///
/// Thresholds based on composite centrality score:
/// - Critical (>=0.8): Top ~5% - changes require extensive testing
/// - High (>=0.6): Top ~15% - changes need careful review
/// - Medium (>=0.4): Top ~30% - normal caution
/// - Low (<0.4): Safe to modify with standard practices
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Composite score >= 0.8: top ~5% of functions, changes require extensive testing.
    Critical,
    /// Composite score >= 0.6: top ~15% of functions, changes need careful review.
    High,
    /// Composite score >= 0.4: top ~30% of functions, normal caution advised.
    Medium,
    /// Composite score < 0.4: safe to modify with standard development practices.
    Low,
}

impl RiskLevel {
    /// Classify risk level from a composite score
    ///
    /// # Arguments
    /// * `score` - Composite centrality score in range [0, 1]
    ///
    /// # Returns
    /// Appropriate risk level based on thresholds
    pub fn from_score(score: f64) -> Self {
        if score >= 0.8 {
            RiskLevel::Critical
        } else if score >= 0.6 {
            RiskLevel::High
        } else if score >= 0.4 {
            RiskLevel::Medium
        } else {
            RiskLevel::Low
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Critical => write!(f, "critical"),
            RiskLevel::High => write!(f, "high"),
            RiskLevel::Medium => write!(f, "medium"),
            RiskLevel::Low => write!(f, "low"),
        }
    }
}

/// Hub score for a single function
///
/// Contains all centrality metrics and derived risk classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubScore {
    /// Reference to the function
    pub function_ref: FunctionRef,
    /// File path (convenience accessor)
    pub file: PathBuf,
    /// Function name (convenience accessor)
    pub name: String,
    /// Normalized in-degree [0, 1] - how many functions call this one
    pub in_degree: f64,
    /// Normalized out-degree [0, 1] - how many functions this one calls
    pub out_degree: f64,
    /// PageRank score [0, 1] - recursive importance based on callers
    /// None if PageRank was not computed
    pub pagerank: Option<f64>,
    /// Betweenness centrality [0, 1] - how often on shortest paths
    /// None if betweenness was not computed
    pub betweenness: Option<f64>,
    /// Raw count of callers
    pub callers_count: usize,
    /// Raw count of callees
    pub callees_count: usize,
    /// Composite score combining all measures [0, 1]
    pub composite_score: f64,
    /// Risk level based on composite score
    pub risk_level: RiskLevel,
}

/// Default weights for composite score calculation (from spec)
pub const WEIGHT_IN_DEGREE: f64 = 0.25;
/// Weight applied to normalized out-degree in the composite hub score formula.
pub const WEIGHT_OUT_DEGREE: f64 = 0.25;
/// Weight applied to betweenness centrality in the composite hub score formula.
pub const WEIGHT_BETWEENNESS: f64 = 0.30;
/// Weight applied to PageRank score in the composite hub score formula.
pub const WEIGHT_PAGERANK: f64 = 0.20;

impl HubScore {
    /// Create a new HubScore from centrality values (in/out degree only)
    ///
    /// # Arguments
    /// * `function_ref` - Reference to the function
    /// * `in_degree` - Normalized in-degree [0, 1]
    /// * `out_degree` - Normalized out-degree [0, 1]
    /// * `callers_count` - Raw count of callers
    /// * `callees_count` - Raw count of callees
    pub fn new(
        function_ref: FunctionRef,
        in_degree: f64,
        out_degree: f64,
        callers_count: usize,
        callees_count: usize,
    ) -> Self {
        // Simple composite: average of in_degree and out_degree (when no pagerank/betweenness)
        let composite_score = (in_degree + out_degree) / 2.0;
        let risk_level = RiskLevel::from_score(composite_score);

        Self {
            file: function_ref.file.clone(),
            name: function_ref.name.clone(),
            function_ref,
            in_degree,
            out_degree,
            pagerank: None,
            betweenness: None,
            callers_count,
            callees_count,
            composite_score,
            risk_level,
        }
    }

    /// Create HubScore with all four centrality measures
    ///
    /// Uses weighted composite:
    /// - in_degree: 0.25
    /// - out_degree: 0.25
    /// - betweenness: 0.30
    /// - pagerank: 0.20
    pub fn with_all_measures(
        function_ref: FunctionRef,
        in_degree: f64,
        out_degree: f64,
        pagerank: f64,
        betweenness: f64,
        callers_count: usize,
        callees_count: usize,
    ) -> Self {
        let composite_score =
            compute_composite_score(in_degree, out_degree, Some(pagerank), Some(betweenness));
        let risk_level = RiskLevel::from_score(composite_score);

        Self {
            file: function_ref.file.clone(),
            name: function_ref.name.clone(),
            function_ref,
            in_degree,
            out_degree,
            pagerank: Some(pagerank),
            betweenness: Some(betweenness),
            callers_count,
            callees_count,
            composite_score,
            risk_level,
        }
    }

    /// Create HubScore with explicit composite score
    ///
    /// Used when composite is computed with additional measures (PageRank, betweenness)
    pub fn with_composite(
        function_ref: FunctionRef,
        in_degree: f64,
        out_degree: f64,
        callers_count: usize,
        callees_count: usize,
        composite_score: f64,
    ) -> Self {
        let risk_level = RiskLevel::from_score(composite_score);

        Self {
            file: function_ref.file.clone(),
            name: function_ref.name.clone(),
            function_ref,
            in_degree,
            out_degree,
            pagerank: None,
            betweenness: None,
            callers_count,
            callees_count,
            composite_score,
            risk_level,
        }
    }

    /// Create HubScore with optional pagerank and betweenness
    pub fn with_optional_measures(
        function_ref: FunctionRef,
        in_degree: f64,
        out_degree: f64,
        pagerank: Option<f64>,
        betweenness: Option<f64>,
        callers_count: usize,
        callees_count: usize,
    ) -> Self {
        let composite_score = compute_composite_score(in_degree, out_degree, pagerank, betweenness);
        let risk_level = RiskLevel::from_score(composite_score);

        Self {
            file: function_ref.file.clone(),
            name: function_ref.name.clone(),
            function_ref,
            in_degree,
            out_degree,
            pagerank,
            betweenness,
            callers_count,
            callees_count,
            composite_score,
            risk_level,
        }
    }
}

/// Compute composite score from available measures
///
/// Uses weighted average with weights normalized to sum to 1.0 for available measures.
/// Default weights (from spec):
/// - in_degree: 0.25
/// - out_degree: 0.25
/// - betweenness: 0.30
/// - pagerank: 0.20
pub fn compute_composite_score(
    in_degree: f64,
    out_degree: f64,
    pagerank: Option<f64>,
    betweenness: Option<f64>,
) -> f64 {
    let mut total_weight = WEIGHT_IN_DEGREE + WEIGHT_OUT_DEGREE;
    let mut weighted_sum = WEIGHT_IN_DEGREE * in_degree + WEIGHT_OUT_DEGREE * out_degree;

    if let Some(pr) = pagerank {
        weighted_sum += WEIGHT_PAGERANK * pr;
        total_weight += WEIGHT_PAGERANK;
    }

    if let Some(bc) = betweenness {
        weighted_sum += WEIGHT_BETWEENNESS * bc;
        total_weight += WEIGHT_BETWEENNESS;
    }

    if total_weight > 0.0 {
        weighted_sum / total_weight
    } else {
        0.0
    }
}

// =============================================================================
// Degree Centrality Functions
// =============================================================================

/// Compute normalized in-degree for all nodes
///
/// In-degree measures how many functions call each function.
/// Higher in-degree means more functions depend on this one.
///
/// Formula: `in_degree(v) = |callers| / (n - 1)`
///
/// Where:
/// - `|callers|` = number of functions that call v
/// - `n` = total number of nodes in the graph
/// - `n - 1` = maximum possible in-degree (all other nodes)
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `reverse_graph` - Map from callee -> [callers]
///
/// # Returns
/// HashMap mapping each FunctionRef to its normalized in-degree [0, 1]
///
/// # Edge Cases
/// - Empty graph: returns empty map
/// - Single node: returns { node: 0.0 } (no possible callers)
/// - Node with no callers: returns 0.0 for that node
pub fn compute_in_degree(
    nodes: &HashSet<FunctionRef>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
) -> HashMap<FunctionRef, f64> {
    let n = nodes.len();

    // Handle edge cases
    if n == 0 {
        return HashMap::new();
    }

    // Single node has no possible callers (n-1 = 0)
    if n == 1 {
        return nodes.iter().map(|node| (node.clone(), 0.0)).collect();
    }

    let max_degree = (n - 1) as f64;

    nodes
        .iter()
        .map(|node| {
            let callers_count = reverse_graph
                .get(node)
                .map(|callers| callers.len())
                .unwrap_or(0);

            let normalized = callers_count as f64 / max_degree;
            (node.clone(), normalized)
        })
        .collect()
}

/// Compute normalized out-degree for all nodes
///
/// Out-degree measures how many functions each function calls.
/// Higher out-degree means this function orchestrates/coordinates many others.
///
/// Formula: `out_degree(v) = |callees| / (n - 1)`
///
/// Where:
/// - `|callees|` = number of functions that v calls
/// - `n` = total number of nodes in the graph
/// - `n - 1` = maximum possible out-degree (all other nodes)
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `forward_graph` - Map from caller -> [callees]
///
/// # Returns
/// HashMap mapping each FunctionRef to its normalized out-degree [0, 1]
///
/// # Edge Cases
/// - Empty graph: returns empty map
/// - Single node: returns { node: 0.0 } (no possible callees)
/// - Node with no callees: returns 0.0 for that node
pub fn compute_out_degree(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
) -> HashMap<FunctionRef, f64> {
    let n = nodes.len();

    // Handle edge cases
    if n == 0 {
        return HashMap::new();
    }

    // Single node has no possible callees (n-1 = 0)
    if n == 1 {
        return nodes.iter().map(|node| (node.clone(), 0.0)).collect();
    }

    let max_degree = (n - 1) as f64;

    nodes
        .iter()
        .map(|node| {
            let callees_count = forward_graph
                .get(node)
                .map(|callees| callees.len())
                .unwrap_or(0);

            let normalized = callees_count as f64 / max_degree;
            (node.clone(), normalized)
        })
        .collect()
}

/// Get raw caller counts for all nodes
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `reverse_graph` - Map from callee -> [callers]
///
/// # Returns
/// HashMap mapping each FunctionRef to its raw caller count
pub fn get_caller_counts(
    nodes: &HashSet<FunctionRef>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
) -> HashMap<FunctionRef, usize> {
    nodes
        .iter()
        .map(|node| {
            let count = reverse_graph
                .get(node)
                .map(|callers| callers.len())
                .unwrap_or(0);
            (node.clone(), count)
        })
        .collect()
}

// =============================================================================
// PageRank Algorithm (T1 mitigation - corrected formula)
// =============================================================================

/// Compute PageRank for all nodes (reverse PageRank for call graphs)
///
/// For call graph analysis, we use **reverse PageRank** to measure
/// "how many important functions depend on this one."
///
/// ## Algorithm (power iteration)
///
/// 1. Initialize all nodes with score 1/n
/// 2. Iterate until convergence:
///    - Compute dangling node contribution (nodes with no callers)
///    - For each node v, new_score = (1-d)/n + d*(incoming_contrib + dangling_contrib)
/// 3. Normalize to [0, 1] by dividing by max value
///
/// ## Formula (T1 mitigation - CORRECTED)
///
/// ```text
/// PR(v) = (1-d)/n + d * (sum(PR(u)/out_deg(u) for u in callers(v)) + dangling_sum/n)
/// ```
///
/// The key correction is that `dangling_sum/n` is INSIDE the damping term,
/// not added separately (which would double-apply damping).
///
/// ## Dangling Nodes (T1 mitigation)
///
/// Dangling nodes are nodes with no outgoing edges (in our reversed view,
/// these are entry points with no callers). Their PageRank is distributed
/// evenly to all nodes.
///
/// # Arguments
/// * `nodes` - Set of all function references
/// * `reverse_graph` - Map from callee -> [callers]
/// * `forward_graph` - Map from caller -> [callees]
/// * `config` - PageRank configuration (damping, max_iter, epsilon)
///
/// # Returns
/// PageRankResult containing normalized scores and convergence info
pub fn compute_pagerank(
    nodes: &HashSet<FunctionRef>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    config: &PageRankConfig,
) -> PageRankResult {
    let n = nodes.len();

    // Handle edge cases
    if n == 0 {
        return PageRankResult {
            scores: HashMap::new(),
            iterations_used: 0,
            converged: true,
        };
    }

    if n == 1 {
        return PageRankResult {
            scores: nodes.iter().map(|node| (node.clone(), 1.0)).collect(),
            iterations_used: 0,
            converged: true,
        };
    }

    let n_f64 = n as f64;
    let d = config.damping;
    let base_score = (1.0 - d) / n_f64;

    // Initialize scores uniformly
    let mut scores: HashMap<FunctionRef, f64> = nodes
        .iter()
        .map(|node| (node.clone(), 1.0 / n_f64))
        .collect();

    // Pre-compute out-degrees on reversed graph (= number of callers for each node)
    // For reverse PageRank, "out-degree" is the number of callees (who we point to in the original)
    // But we're computing importance based on who calls us, so we use the reverse graph
    let out_degrees: HashMap<FunctionRef, usize> = nodes
        .iter()
        .map(|node| {
            // Out-degree in the reverse graph = number of nodes this node points to in reverse
            // = number of functions this function calls (forward graph edges from this node)
            let deg = forward_graph.get(node).map_or(0, |v| v.len());
            (node.clone(), deg)
        })
        .collect();

    // Identify dangling nodes (nodes with no outgoing edges in the original graph)
    // These are leaf functions that don't call anything
    let dangling_nodes: Vec<FunctionRef> = nodes
        .iter()
        .filter(|node| out_degrees.get(*node).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();

    let mut iterations_used = 0;
    let mut converged = false;

    for _ in 0..config.max_iterations {
        iterations_used += 1;

        // Compute dangling node contribution
        let dangling_sum: f64 = dangling_nodes.iter().map(|node| scores[node]).sum();
        let dangling_contrib = dangling_sum / n_f64;

        let mut new_scores: HashMap<FunctionRef, f64> = HashMap::new();
        let mut max_delta: f64 = 0.0;

        for node in nodes {
            // Contribution from nodes that call this node (reverse graph)
            // In the original graph, these are the callers of `node`
            let incoming_contrib: f64 = reverse_graph.get(node).map_or(0.0, |callers| {
                callers
                    .iter()
                    .map(|caller| {
                        let caller_out_deg = out_degrees.get(caller).copied().unwrap_or(0);
                        if caller_out_deg > 0 {
                            scores[caller] / caller_out_deg as f64
                        } else {
                            0.0
                        }
                    })
                    .sum()
            });

            // CORRECTED formula (T1): dangling_contrib is inside the damping term
            let new_score = base_score + d * (incoming_contrib + dangling_contrib);

            let delta = (new_score - scores[node]).abs();
            if delta > max_delta {
                max_delta = delta;
            }

            new_scores.insert(node.clone(), new_score);
        }

        scores = new_scores;

        // Check convergence
        if max_delta < config.epsilon {
            converged = true;
            break;
        }
    }

    // Normalize to [0, 1] by dividing by max value
    let max_score = scores.values().copied().fold(0.0_f64, f64::max);
    if max_score > 0.0 {
        for score in scores.values_mut() {
            *score /= max_score;
        }
    }

    PageRankResult {
        scores,
        iterations_used,
        converged,
    }
}

// =============================================================================
// Betweenness Centrality (T4 mitigation - with sampling)
// =============================================================================

/// Compute betweenness centrality for all nodes
///
/// Betweenness measures how often a node lies on shortest paths between
/// other nodes. High betweenness indicates a "bridge" or "bottleneck".
///
/// ## Algorithm (Brandes)
///
/// For each source node s:
/// 1. BFS to find shortest path distances and predecessors
/// 2. Backward pass to accumulate dependency values
/// 3. Update betweenness for each node (except source)
///
/// ## Complexity
///
/// O(V * E) for unweighted graphs. For large graphs, use sampling (T4 mitigation).
///
/// ## Sampling (T4 mitigation)
///
/// When `sample_size` is Some(k), only k random sources are used.
/// The results are then scaled by n/k to approximate full betweenness.
/// Per Brandes 2008, k=100 gives good approximation.
///
/// ## Normalization (T3 mitigation)
///
/// For directed graphs: `b(v) / ((n-1)(n-2))`
/// Then normalized to [0, 1] by dividing by max value.
///
/// # Arguments
/// * `nodes` - Set of all function references
/// * `forward_graph` - Map from caller -> [callees]
/// * `config` - Betweenness configuration (sample_size, max_nodes)
///
/// # Returns
/// HashMap mapping each FunctionRef to its normalized betweenness [0, 1]
pub fn compute_betweenness(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    config: &BetweennessConfig,
) -> HashMap<FunctionRef, f64> {
    let n = nodes.len();

    // Handle edge cases
    if n <= 2 {
        return nodes.iter().map(|node| (node.clone(), 0.0)).collect();
    }

    // Check if graph is too large
    if n > config.max_nodes {
        // For very large graphs, return zeros with a warning
        // In practice, the caller should use sampling
        return nodes.iter().map(|node| (node.clone(), 0.0)).collect();
    }

    // Convert nodes to Vec for indexing
    let node_list: Vec<FunctionRef> = nodes.iter().cloned().collect();

    // Determine which sources to use
    let sources: Vec<&FunctionRef> = match config.sample_size {
        Some(k) if k < n => {
            // Sample k sources deterministically (using modular arithmetic for reproducibility)
            // For true randomness, you'd use rand, but determinism is better for testing
            let step = n / k.max(1);
            (0..k).map(|i| &node_list[(i * step) % n]).collect()
        }
        _ => {
            // Use all sources
            node_list.iter().collect()
        }
    };

    let num_sources = sources.len();
    let scaling_factor = if num_sources < n {
        n as f64 / num_sources as f64
    } else {
        1.0
    };

    let mut betweenness: HashMap<FunctionRef, f64> =
        nodes.iter().map(|node| (node.clone(), 0.0)).collect();

    // Brandes algorithm
    for source in &sources {
        // BFS for single-source shortest paths
        let mut dist: HashMap<&FunctionRef, usize> = HashMap::new();
        let mut sigma: HashMap<&FunctionRef, f64> = HashMap::new();
        let mut pred: HashMap<&FunctionRef, Vec<&FunctionRef>> = HashMap::new();

        dist.insert(source, 0);
        sigma.insert(source, 1.0);

        let mut queue: VecDeque<&FunctionRef> = VecDeque::new();
        queue.push_back(source);

        let mut order: Vec<&FunctionRef> = Vec::new();

        while let Some(current) = queue.pop_front() {
            order.push(current);

            // Get neighbors (callees in forward graph)
            if let Some(neighbors) = forward_graph.get(current) {
                for neighbor in neighbors {
                    if !nodes.contains(neighbor) {
                        continue;
                    }
                    // First time seeing neighbor?
                    if !dist.contains_key(&neighbor) {
                        dist.insert(neighbor, dist[&current] + 1);
                        queue.push_back(neighbor);
                    }

                    // Is this neighbor on a shortest path from source?
                    if dist.get(&neighbor) == Some(&(dist[&current] + 1)) {
                        *sigma.entry(neighbor).or_insert(0.0) += sigma[&current];
                        pred.entry(neighbor).or_default().push(current);
                    }
                }
            }
        }

        // Back-propagation of dependencies
        let mut delta: HashMap<&FunctionRef, f64> =
            node_list.iter().map(|node| (node, 0.0)).collect();

        // Process in reverse order (farthest to nearest)
        while let Some(w) = order.pop() {
            if let Some(predecessors) = pred.get(&w) {
                for v in predecessors {
                    let sigma_v = sigma.get(v).copied().unwrap_or(0.0);
                    let sigma_w = sigma.get(&w).copied().unwrap_or(0.0);
                    if sigma_w > 0.0 {
                        let contribution = (sigma_v / sigma_w) * (1.0 + delta[&w]);
                        *delta.get_mut(v).unwrap() += contribution;
                    }
                }
            }

            // Accumulate (skip source)
            if w != *source {
                *betweenness.get_mut(w).unwrap() += delta[&w];
            }
        }
    }

    // Apply scaling factor for sampling
    if scaling_factor > 1.0 {
        for value in betweenness.values_mut() {
            *value *= scaling_factor;
        }
    }

    // Normalize for directed graph: (n-1)(n-2)
    let normalizer = ((n - 1) * (n - 2)) as f64;
    if normalizer > 0.0 {
        for value in betweenness.values_mut() {
            *value /= normalizer;
        }
    }

    // Normalize to [0, 1] by dividing by max value
    let max_val = betweenness.values().copied().fold(0.0_f64, f64::max);
    if max_val > 1.0 {
        for value in betweenness.values_mut() {
            *value /= max_val;
        }
    }

    betweenness
}

// =============================================================================
// Utility Functions
// =============================================================================

/// Get raw callee counts for all nodes
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `forward_graph` - Map from caller -> [callees]
///
/// # Returns
/// HashMap mapping each FunctionRef to its raw callee count
pub fn get_callee_counts(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
) -> HashMap<FunctionRef, usize> {
    nodes
        .iter()
        .map(|node| {
            let count = forward_graph
                .get(node)
                .map(|callees| callees.len())
                .unwrap_or(0);
            (node.clone(), count)
        })
        .collect()
}

/// Compute HubScores for all nodes using in-degree and out-degree only
///
/// This is a convenience function that combines in-degree and out-degree
/// computation into full HubScore objects. For full centrality analysis
/// including PageRank and betweenness, use `compute_hub_scores_full`.
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `forward_graph` - Map from caller -> [callees]
/// * `reverse_graph` - Map from callee -> [callers]
///
/// # Returns
/// Vec of HubScores sorted by composite_score descending
pub fn compute_hub_scores(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
) -> Vec<HubScore> {
    let in_degrees = compute_in_degree(nodes, reverse_graph);
    let out_degrees = compute_out_degree(nodes, forward_graph);
    let caller_counts = get_caller_counts(nodes, reverse_graph);
    let callee_counts = get_callee_counts(nodes, forward_graph);

    let mut scores: Vec<HubScore> = nodes
        .iter()
        .map(|node| {
            let in_deg = in_degrees.get(node).copied().unwrap_or(0.0);
            let out_deg = out_degrees.get(node).copied().unwrap_or(0.0);
            let callers = caller_counts.get(node).copied().unwrap_or(0);
            let callees = callee_counts.get(node).copied().unwrap_or(0);

            HubScore::new(node.clone(), in_deg, out_deg, callers, callees)
        })
        .collect();

    // Sort by composite score descending
    scores.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    scores
}

/// Algorithm selection for hub detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HubAlgorithm {
    /// All algorithms: in_degree, out_degree, pagerank, betweenness
    #[default]
    All,
    /// In-degree only (fast)
    InDegree,
    /// Out-degree only (fast)
    OutDegree,
    /// PageRank only
    PageRank,
    /// Betweenness only (slow for large graphs)
    Betweenness,
}

impl std::str::FromStr for HubAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "all" => Ok(HubAlgorithm::All),
            "indegree" | "in_degree" | "in-degree" => Ok(HubAlgorithm::InDegree),
            "outdegree" | "out_degree" | "out-degree" => Ok(HubAlgorithm::OutDegree),
            "pagerank" | "page_rank" => Ok(HubAlgorithm::PageRank),
            "betweenness" => Ok(HubAlgorithm::Betweenness),
            _ => Err(format!(
                "Unknown algorithm '{}'. Valid: all, indegree, outdegree, pagerank, betweenness",
                s
            )),
        }
    }
}

impl std::fmt::Display for HubAlgorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HubAlgorithm::All => write!(f, "all"),
            HubAlgorithm::InDegree => write!(f, "indegree"),
            HubAlgorithm::OutDegree => write!(f, "outdegree"),
            HubAlgorithm::PageRank => write!(f, "pagerank"),
            HubAlgorithm::Betweenness => write!(f, "betweenness"),
        }
    }
}

/// Full hub detection report (spec Section 3 - hubs CLI)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubReport {
    /// Top hubs sorted by composite score descending
    pub hubs: Vec<HubScore>,
    /// Total number of nodes in the call graph
    pub total_nodes: usize,
    /// Number of hubs returned (may be less than total if threshold applied)
    pub hub_count: usize,
    /// Measures used in this analysis
    pub measures_used: Vec<String>,
    /// Top K by in-degree (for by_measure breakdown)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_in_degree: Vec<HubScore>,
    /// Top K by out-degree (for by_measure breakdown)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_out_degree: Vec<HubScore>,
    /// Top K by pagerank (for by_measure breakdown)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_pagerank: Vec<HubScore>,
    /// Top K by betweenness (for by_measure breakdown)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_betweenness: Vec<HubScore>,
    /// PageRank convergence info (if computed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagerank_info: Option<PageRankConvergenceInfo>,
    /// Explanation message (T16 mitigation: small graph messaging)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

/// PageRank convergence info for the report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRankConvergenceInfo {
    /// Number of iterations used
    pub iterations_used: usize,
    /// Whether the algorithm converged
    pub converged: bool,
}

impl From<&PageRankResult> for PageRankConvergenceInfo {
    fn from(result: &PageRankResult) -> Self {
        Self {
            iterations_used: result.iterations_used,
            converged: result.converged,
        }
    }
}

/// Compute HubScores with all four centrality measures
///
/// Computes in-degree, out-degree, PageRank, and betweenness centrality.
/// Uses weighted composite score with default weights:
/// - in_degree: 0.25
/// - out_degree: 0.25
/// - betweenness: 0.30
/// - pagerank: 0.20
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `forward_graph` - Map from caller -> [callees]
/// * `reverse_graph` - Map from callee -> [callers]
/// * `pagerank_config` - Optional PageRank configuration (uses default if None)
/// * `betweenness_config` - Optional betweenness configuration (uses default if None)
///
/// # Returns
/// (Vec<HubScore>, PageRankResult) - Scores sorted by composite descending, and PageRank info
pub fn compute_hub_scores_full(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    pagerank_config: Option<&PageRankConfig>,
    betweenness_config: Option<&BetweennessConfig>,
) -> (Vec<HubScore>, PageRankResult) {
    let in_degrees = compute_in_degree(nodes, reverse_graph);
    let out_degrees = compute_out_degree(nodes, forward_graph);
    let caller_counts = get_caller_counts(nodes, reverse_graph);
    let callee_counts = get_callee_counts(nodes, forward_graph);

    // Compute PageRank
    let pr_config = pagerank_config.cloned().unwrap_or_default();
    let pagerank_result = compute_pagerank(nodes, reverse_graph, forward_graph, &pr_config);

    // Compute betweenness
    let bc_config = betweenness_config.cloned().unwrap_or_default();
    let betweenness = compute_betweenness(nodes, forward_graph, &bc_config);

    let mut scores: Vec<HubScore> = nodes
        .iter()
        .map(|node| {
            let in_deg = in_degrees.get(node).copied().unwrap_or(0.0);
            let out_deg = out_degrees.get(node).copied().unwrap_or(0.0);
            let pr = pagerank_result.scores.get(node).copied().unwrap_or(0.0);
            let bc = betweenness.get(node).copied().unwrap_or(0.0);
            let callers = caller_counts.get(node).copied().unwrap_or(0);
            let callees = callee_counts.get(node).copied().unwrap_or(0);

            HubScore::with_all_measures(node.clone(), in_deg, out_deg, pr, bc, callers, callees)
        })
        .collect();

    // Sort by composite score descending
    scores.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    (scores, pagerank_result)
}

/// Compute HubScores with selected algorithm(s)
///
/// This function allows selecting which centrality measures to compute,
/// which is useful for faster analysis when only specific measures are needed.
///
/// # Arguments
/// * `nodes` - Set of all function references in the graph
/// * `forward_graph` - Map from caller -> [callees]
/// * `reverse_graph` - Map from callee -> [callers]
/// * `algorithm` - Which algorithm(s) to use
/// * `top_k` - Number of top hubs to return
/// * `threshold` - Optional minimum composite score to include
///
/// # Returns
/// HubReport containing the analysis results
pub fn compute_hub_report(
    nodes: &HashSet<FunctionRef>,
    forward_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    reverse_graph: &HashMap<FunctionRef, Vec<FunctionRef>>,
    algorithm: HubAlgorithm,
    top_k: usize,
    threshold: Option<f64>,
) -> HubReport {
    let total_nodes = nodes.len();

    // Handle empty graph (T16 mitigation)
    if total_nodes == 0 {
        return HubReport {
            hubs: Vec::new(),
            total_nodes: 0,
            hub_count: 0,
            measures_used: Vec::new(),
            by_in_degree: Vec::new(),
            by_out_degree: Vec::new(),
            by_pagerank: Vec::new(),
            by_betweenness: Vec::new(),
            pagerank_info: None,
            explanation: Some("Empty call graph - no functions found.".to_string()),
        };
    }

    // Compute base degrees (always needed)
    let in_degrees = compute_in_degree(nodes, reverse_graph);
    let out_degrees = compute_out_degree(nodes, forward_graph);
    let caller_counts = get_caller_counts(nodes, reverse_graph);
    let callee_counts = get_callee_counts(nodes, forward_graph);

    // Compute optional measures based on algorithm
    let (pagerank_scores, pagerank_info) =
        if matches!(algorithm, HubAlgorithm::All | HubAlgorithm::PageRank) {
            let config = PageRankConfig::default();
            let result = compute_pagerank(nodes, reverse_graph, forward_graph, &config);
            let info = PageRankConvergenceInfo::from(&result);
            (Some(result.scores), Some(info))
        } else {
            (None, None)
        };

    let betweenness_scores = if matches!(algorithm, HubAlgorithm::All | HubAlgorithm::Betweenness) {
        let config = BetweennessConfig::default();
        Some(compute_betweenness(nodes, forward_graph, &config))
    } else {
        None
    };

    // Build HubScores for all nodes
    let mut all_scores: Vec<HubScore> = nodes
        .iter()
        .map(|node| {
            let in_deg = in_degrees.get(node).copied().unwrap_or(0.0);
            let out_deg = out_degrees.get(node).copied().unwrap_or(0.0);
            let pr = pagerank_scores.as_ref().and_then(|s| s.get(node).copied());
            let bc = betweenness_scores
                .as_ref()
                .and_then(|s| s.get(node).copied());
            let callers = caller_counts.get(node).copied().unwrap_or(0);
            let callees = callee_counts.get(node).copied().unwrap_or(0);

            HubScore::with_optional_measures(
                node.clone(),
                in_deg,
                out_deg,
                pr,
                bc,
                callers,
                callees,
            )
        })
        .collect();

    // Apply threshold filter if specified
    if let Some(thresh) = threshold {
        all_scores.retain(|s| s.composite_score >= thresh);
    }

    // Sort by composite score descending
    all_scores.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Take top K
    let hubs: Vec<HubScore> = all_scores.into_iter().take(top_k).collect();
    let hub_count = hubs.len();

    // Build by_* breakdowns (only for 'all' algorithm)
    let (by_in_degree, by_out_degree, by_pagerank, by_betweenness) =
        if matches!(algorithm, HubAlgorithm::All) {
            // Sort copies by each measure
            let mut by_in: Vec<HubScore> = hubs.clone();
            by_in.sort_by(|a, b| {
                b.in_degree
                    .partial_cmp(&a.in_degree)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut by_out: Vec<HubScore> = hubs.clone();
            by_out.sort_by(|a, b| {
                b.out_degree
                    .partial_cmp(&a.out_degree)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut by_pr: Vec<HubScore> = hubs.clone();
            by_pr.sort_by(|a, b| {
                let a_pr = a.pagerank.unwrap_or(0.0);
                let b_pr = b.pagerank.unwrap_or(0.0);
                b_pr.partial_cmp(&a_pr).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut by_bc: Vec<HubScore> = hubs.clone();
            by_bc.sort_by(|a, b| {
                let a_bc = a.betweenness.unwrap_or(0.0);
                let b_bc = b.betweenness.unwrap_or(0.0);
                b_bc.partial_cmp(&a_bc).unwrap_or(std::cmp::Ordering::Equal)
            });

            (by_in, by_out, by_pr, by_bc)
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };

    // Build measures_used list
    let measures_used = match algorithm {
        HubAlgorithm::All => vec![
            "in_degree".to_string(),
            "out_degree".to_string(),
            "pagerank".to_string(),
            "betweenness".to_string(),
        ],
        HubAlgorithm::InDegree => vec!["in_degree".to_string()],
        HubAlgorithm::OutDegree => vec!["out_degree".to_string()],
        HubAlgorithm::PageRank => vec!["pagerank".to_string()],
        HubAlgorithm::Betweenness => vec!["betweenness".to_string()],
    };

    // T16 mitigation: small graph messaging
    let explanation = if total_nodes < 10 {
        Some(format!(
            "Small call graph ({} nodes). Hub metrics may not be statistically meaningful for graphs with fewer than 10 nodes.",
            total_nodes
        ))
    } else {
        // Count critical and high risk hubs
        let critical_count = hubs
            .iter()
            .filter(|h| h.risk_level == RiskLevel::Critical)
            .count();
        let high_count = hubs
            .iter()
            .filter(|h| h.risk_level == RiskLevel::High)
            .count();
        if critical_count > 0 || high_count > 0 {
            Some(format!(
                "Found {} critical and {} high-risk hubs. Changes to these functions may have widespread impact.",
                critical_count, high_count
            ))
        } else {
            None
        }
    };

    HubReport {
        hubs,
        total_nodes,
        hub_count,
        measures_used,
        by_in_degree,
        by_out_degree,
        by_pagerank,
        by_betweenness,
        pagerank_info,
        explanation,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::callgraph::graph_utils::{build_forward_graph, build_reverse_graph, collect_nodes};
    use crate::types::{CallEdge, ProjectCallGraph};

    /// Create a star topology: central_hub is called by 5 callers
    fn create_star_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        // 5 callers all call central_hub
        for i in 1..=5 {
            graph.add_edge(CallEdge {
                src_file: PathBuf::from(format!("caller_{}.py", i)),
                src_func: format!("caller_{}", i),
                dst_file: PathBuf::from("hub.py"),
                dst_func: "central_hub".to_string(),
            });
        }

        graph
    }

    /// Create a chain: A -> B -> C -> D
    fn create_chain_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        graph.add_edge(CallEdge {
            src_file: PathBuf::from("b.py"),
            src_func: "func_b".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "func_c".to_string(),
        });

        graph.add_edge(CallEdge {
            src_file: PathBuf::from("c.py"),
            src_func: "func_c".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "func_d".to_string(),
        });

        graph
    }

    /// Create a diamond: A -> B, A -> C, B -> D, C -> D
    fn create_diamond_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        // A -> B
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        // A -> C
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("c.py"),
            dst_func: "func_c".to_string(),
        });

        // B -> D
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("b.py"),
            src_func: "func_b".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "func_d".to_string(),
        });

        // C -> D
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("c.py"),
            src_func: "func_c".to_string(),
            dst_file: PathBuf::from("d.py"),
            dst_func: "func_d".to_string(),
        });

        graph
    }

    // -------------------------------------------------------------------------
    // Risk Level Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_risk_level_thresholds() {
        assert_eq!(RiskLevel::from_score(0.9), RiskLevel::Critical);
        assert_eq!(RiskLevel::from_score(0.8), RiskLevel::Critical);
        assert_eq!(RiskLevel::from_score(0.79), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.6), RiskLevel::High);
        assert_eq!(RiskLevel::from_score(0.59), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.4), RiskLevel::Medium);
        assert_eq!(RiskLevel::from_score(0.39), RiskLevel::Low);
        assert_eq!(RiskLevel::from_score(0.0), RiskLevel::Low);
    }

    #[test]
    fn test_risk_level_edge_cases() {
        // Boundary values
        assert_eq!(RiskLevel::from_score(1.0), RiskLevel::Critical);
        assert_eq!(RiskLevel::from_score(0.0), RiskLevel::Low);
    }

    // -------------------------------------------------------------------------
    // In-Degree Tests
    // -------------------------------------------------------------------------

    #[test]
    fn indegree_counts_callers() {
        let graph = create_star_graph();
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let in_degrees = compute_in_degree(&nodes, &reverse);

        // central_hub is called by 5 callers
        // n = 6 (central_hub + 5 callers)
        // in_degree(central_hub) = 5 / (6 - 1) = 1.0
        let hub = FunctionRef::new(PathBuf::from("hub.py"), "central_hub");
        assert_eq!(in_degrees.get(&hub), Some(&1.0));

        // Each caller has 0 callers
        for i in 1..=5 {
            let caller = FunctionRef::new(
                PathBuf::from(format!("caller_{}.py", i)),
                format!("caller_{}", i),
            );
            assert_eq!(in_degrees.get(&caller), Some(&0.0));
        }
    }

    #[test]
    fn indegree_normalized() {
        let graph = create_diamond_graph();
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let in_degrees = compute_in_degree(&nodes, &reverse);

        // All values should be in [0, 1]
        for &degree in in_degrees.values() {
            assert!(
                (0.0..=1.0).contains(&degree),
                "In-degree {} out of range",
                degree
            );
        }

        // D has 2 callers (B and C), n = 4
        // in_degree(D) = 2 / 3 = 0.666...
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        let d_degree = in_degrees.get(&d).unwrap();
        assert!((d_degree - 2.0 / 3.0).abs() < 0.001);

        // B and C each have 1 caller (A)
        // in_degree(B) = in_degree(C) = 1 / 3 = 0.333...
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert!((in_degrees.get(&b).unwrap() - 1.0 / 3.0).abs() < 0.001);
        assert!((in_degrees.get(&c).unwrap() - 1.0 / 3.0).abs() < 0.001);

        // A has 0 callers
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert_eq!(in_degrees.get(&a), Some(&0.0));
    }

    // -------------------------------------------------------------------------
    // Out-Degree Tests
    // -------------------------------------------------------------------------

    #[test]
    fn outdegree_counts_callees() {
        let graph = create_diamond_graph();
        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        let out_degrees = compute_out_degree(&nodes, &forward);

        // A calls 2 functions (B and C), n = 4
        // out_degree(A) = 2 / 3 = 0.666...
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        let a_degree = out_degrees.get(&a).unwrap();
        assert!((a_degree - 2.0 / 3.0).abs() < 0.001);

        // B and C each call 1 function (D)
        // out_degree(B) = out_degree(C) = 1 / 3 = 0.333...
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        assert!((out_degrees.get(&b).unwrap() - 1.0 / 3.0).abs() < 0.001);
        assert!((out_degrees.get(&c).unwrap() - 1.0 / 3.0).abs() < 0.001);

        // D calls 0 functions (leaf)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert_eq!(out_degrees.get(&d), Some(&0.0));
    }

    #[test]
    fn outdegree_normalized() {
        let graph = create_chain_graph();
        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        let out_degrees = compute_out_degree(&nodes, &forward);

        // All values should be in [0, 1]
        for &degree in out_degrees.values() {
            assert!(
                (0.0..=1.0).contains(&degree),
                "Out-degree {} out of range",
                degree
            );
        }

        // In chain A -> B -> C -> D, n = 4
        // Each of A, B, C calls 1 function
        // out_degree = 1 / 3 = 0.333...
        for name in ["func_a", "func_b", "func_c"] {
            let file = format!("{}.py", name.chars().last().unwrap());
            let func = FunctionRef::new(PathBuf::from(&file), name);
            let degree = out_degrees.get(&func).unwrap();
            assert!(
                (degree - 1.0 / 3.0).abs() < 0.001,
                "{} has unexpected out-degree {}",
                name,
                degree
            );
        }

        // D calls nothing (leaf)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        assert_eq!(out_degrees.get(&d), Some(&0.0));
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    fn empty_graph_returns_empty_map() {
        let graph = ProjectCallGraph::new();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let in_degrees = compute_in_degree(&nodes, &reverse);
        let out_degrees = compute_out_degree(&nodes, &forward);

        assert!(in_degrees.is_empty());
        assert!(out_degrees.is_empty());
    }

    #[test]
    fn single_node_graph() {
        // Create a graph with a self-loop (only way to have 1 node)
        let mut graph = ProjectCallGraph::new();
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("a.py"),
            dst_func: "func_a".to_string(),
        });

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        assert_eq!(nodes.len(), 1);

        let in_degrees = compute_in_degree(&nodes, &reverse);
        let out_degrees = compute_out_degree(&nodes, &forward);

        // Single node: n = 1, so max_degree = n - 1 = 0
        // Return 0.0 for single node (no other possible callers/callees)
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        assert_eq!(in_degrees.get(&a), Some(&0.0));
        assert_eq!(out_degrees.get(&a), Some(&0.0));
    }

    #[test]
    fn two_node_graph_normalization() {
        // A -> B: simplest possible graph
        let mut graph = ProjectCallGraph::new();
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        assert_eq!(nodes.len(), 2);

        let in_degrees = compute_in_degree(&nodes, &reverse);
        let out_degrees = compute_out_degree(&nodes, &forward);

        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");

        // n = 2, max_degree = 1
        // A has 0 callers, 1 callee -> in_degree = 0/1 = 0, out_degree = 1/1 = 1
        // B has 1 caller, 0 callees -> in_degree = 1/1 = 1, out_degree = 0/1 = 0
        assert_eq!(in_degrees.get(&a), Some(&0.0));
        assert_eq!(out_degrees.get(&a), Some(&1.0));
        assert_eq!(in_degrees.get(&b), Some(&1.0));
        assert_eq!(out_degrees.get(&b), Some(&0.0));
    }

    // -------------------------------------------------------------------------
    // HubScore Tests
    // -------------------------------------------------------------------------

    #[test]
    fn hub_score_composite_calculation() {
        let func = FunctionRef::new(PathBuf::from("test.py"), "test_func");
        let score = HubScore::new(func, 0.6, 0.4, 3, 2);

        // Composite = (in_degree + out_degree) / 2 = (0.6 + 0.4) / 2 = 0.5
        assert!((score.composite_score - 0.5).abs() < 0.001);
        assert_eq!(score.risk_level, RiskLevel::Medium);
    }

    #[test]
    fn hub_score_critical_threshold() {
        let func = FunctionRef::new(PathBuf::from("test.py"), "critical_func");
        let score = HubScore::new(func, 0.9, 0.8, 10, 8);

        // Composite = (0.9 + 0.8) / 2 = 0.85 -> Critical
        assert!(score.composite_score >= 0.8);
        assert_eq!(score.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn compute_hub_scores_sorting() {
        let graph = create_star_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let scores = compute_hub_scores(&nodes, &forward, &reverse);

        // Should be sorted by composite_score descending
        for window in scores.windows(2) {
            assert!(
                window[0].composite_score >= window[1].composite_score,
                "Scores not sorted: {} >= {}",
                window[0].composite_score,
                window[1].composite_score
            );
        }

        // central_hub should be first (highest in_degree = 1.0, out_degree = 0.0)
        // composite = 0.5
        assert_eq!(scores[0].name, "central_hub");
    }

    #[test]
    fn compute_hub_scores_includes_raw_counts() {
        let graph = create_star_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let scores = compute_hub_scores(&nodes, &forward, &reverse);

        // Find central_hub
        let hub_score = scores.iter().find(|s| s.name == "central_hub").unwrap();
        assert_eq!(hub_score.callers_count, 5);
        assert_eq!(hub_score.callees_count, 0);

        // Find one of the callers
        let caller_score = scores.iter().find(|s| s.name == "caller_1").unwrap();
        assert_eq!(caller_score.callers_count, 0);
        assert_eq!(caller_score.callees_count, 1);
    }

    // -------------------------------------------------------------------------
    // PageRank Tests
    // -------------------------------------------------------------------------

    #[test]
    fn pagerank_converges() {
        // Test on diamond: A -> B, A -> C, B -> D, C -> D
        let graph = create_diamond_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let config = PageRankConfig::default();
        let result = compute_pagerank(&nodes, &reverse, &forward, &config);

        // Should converge
        assert!(result.converged, "PageRank did not converge");
        assert!(
            result.iterations_used < config.max_iterations,
            "Used all iterations: {}",
            result.iterations_used
        );

        // All scores should be in [0, 1]
        for &score in result.scores.values() {
            assert!((0.0..=1.0).contains(&score), "Score {} out of range", score);
        }

        // D should have highest PageRank (most callers transitively)
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        let d_score = result.scores.get(&d).copied().unwrap_or(0.0);

        // D should be normalized to 1.0 (highest)
        assert!(
            (d_score - 1.0).abs() < 0.01,
            "D should have highest PR, got {}",
            d_score
        );
    }

    #[test]
    fn pagerank_damping_factor() {
        let graph = create_chain_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        // Test with different damping factors
        let config_high = PageRankConfig {
            damping: 0.95,
            ..Default::default()
        };
        let config_low = PageRankConfig {
            damping: 0.5,
            ..Default::default()
        };

        let result_high = compute_pagerank(&nodes, &reverse, &forward, &config_high);
        let result_low = compute_pagerank(&nodes, &reverse, &forward, &config_low);

        // Both should converge
        assert!(result_high.converged);
        assert!(result_low.converged);

        // With lower damping, scores should be more uniform (more random jumps)
        // With higher damping, end of chain should have more influence
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");

        let d_high = result_high.scores.get(&d).copied().unwrap_or(0.0);
        let d_low = result_low.scores.get(&d).copied().unwrap_or(0.0);

        // D is at the end of chain, so with higher damping it gets more from following edges
        // Both are normalized, but the relative distribution should differ
        // This is a sanity check - different dampings produce different results
        assert!(d_high > 0.0 && d_low > 0.0);
    }

    #[test]
    fn pagerank_handles_dangling_nodes() {
        // Chain: A -> B -> C -> D (D has no outgoing edges - it's a dangling node)
        let graph = create_chain_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let config = PageRankConfig::default();
        let result = compute_pagerank(&nodes, &reverse, &forward, &config);

        // Should converge without issues
        assert!(
            result.converged,
            "PageRank did not converge with dangling node"
        );

        // All nodes should have scores (no NaN or Inf)
        for (node, &score) in &result.scores {
            assert!(!score.is_nan(), "NaN score for {:?}", node);
            assert!(!score.is_infinite(), "Infinite score for {:?}", node);
            assert!(score >= 0.0, "Negative score for {:?}", node);
        }

        // D (dangling node) should still have a PageRank
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");
        let d_score = result.scores.get(&d).copied().unwrap_or(-1.0);
        assert!(d_score > 0.0, "Dangling node should have positive PR");
    }

    #[test]
    fn pagerank_empty_graph() {
        let nodes: HashSet<FunctionRef> = HashSet::new();
        let forward: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();
        let reverse: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();

        let config = PageRankConfig::default();
        let result = compute_pagerank(&nodes, &reverse, &forward, &config);

        assert!(result.scores.is_empty());
        assert!(result.converged);
        assert_eq!(result.iterations_used, 0);
    }

    #[test]
    fn pagerank_single_node() {
        let node = FunctionRef::new(PathBuf::from("single.py"), "single_func");
        let mut nodes: HashSet<FunctionRef> = HashSet::new();
        nodes.insert(node.clone());

        let forward: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();
        let reverse: HashMap<FunctionRef, Vec<FunctionRef>> = HashMap::new();

        let config = PageRankConfig::default();
        let result = compute_pagerank(&nodes, &reverse, &forward, &config);

        assert_eq!(result.scores.len(), 1);
        assert_eq!(result.scores.get(&node), Some(&1.0));
        assert!(result.converged);
    }

    // -------------------------------------------------------------------------
    // Betweenness Centrality Tests
    // -------------------------------------------------------------------------

    #[test]
    fn betweenness_bridge_detection() {
        // Chain: A -> B -> C -> D
        // B and C should have highest betweenness (they're on paths)
        let graph = create_chain_graph();
        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        let config = BetweennessConfig::default();
        let betweenness = compute_betweenness(&nodes, &forward, &config);

        // All scores should be in [0, 1]
        for &score in betweenness.values() {
            assert!(
                (0.0..=1.0).contains(&score),
                "Betweenness {} out of range",
                score
            );
        }

        // B and C are on paths between A and D
        let b = FunctionRef::new(PathBuf::from("b.py"), "func_b");
        let c = FunctionRef::new(PathBuf::from("c.py"), "func_c");
        let a = FunctionRef::new(PathBuf::from("a.py"), "func_a");
        let d = FunctionRef::new(PathBuf::from("d.py"), "func_d");

        let b_bc = betweenness.get(&b).copied().unwrap_or(0.0);
        let c_bc = betweenness.get(&c).copied().unwrap_or(0.0);
        let a_bc = betweenness.get(&a).copied().unwrap_or(0.0);
        let d_bc = betweenness.get(&d).copied().unwrap_or(0.0);

        // A and D are endpoints, they should have lower betweenness
        // B and C are on paths, they should have higher betweenness
        assert!(
            b_bc >= a_bc,
            "B should have >= betweenness than A (endpoint)"
        );
        assert!(
            c_bc >= d_bc,
            "C should have >= betweenness than D (endpoint)"
        );
    }

    #[test]
    fn betweenness_normalized() {
        // Diamond: A -> B, A -> C, B -> D, C -> D
        let graph = create_diamond_graph();
        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        let config = BetweennessConfig::default();
        let betweenness = compute_betweenness(&nodes, &forward, &config);

        // All values should be in [0, 1]
        for (node, &score) in &betweenness {
            assert!(
                (0.0..=1.0).contains(&score),
                "Betweenness for {:?} = {} is not in [0,1]",
                node,
                score
            );
            assert!(!score.is_nan(), "NaN betweenness for {:?}", node);
            assert!(!score.is_infinite(), "Infinite betweenness for {:?}", node);
        }
    }

    #[test]
    fn betweenness_sampling() {
        // Create a larger graph where sampling matters
        let mut graph = ProjectCallGraph::new();

        // Create a longer chain: node_0 -> node_1 -> ... -> node_9
        for i in 0..9 {
            graph.add_edge(CallEdge {
                src_file: PathBuf::from(format!("node_{}.py", i)),
                src_func: format!("func_{}", i),
                dst_file: PathBuf::from(format!("node_{}.py", i + 1)),
                dst_func: format!("func_{}", i + 1),
            });
        }

        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        // Full computation
        let config_full = BetweennessConfig {
            sample_size: None,
            max_nodes: 5000,
        };
        let bc_full = compute_betweenness(&nodes, &forward, &config_full);

        // Sampled computation (sample 5 out of 10 nodes)
        let config_sampled = BetweennessConfig {
            sample_size: Some(5),
            max_nodes: 5000,
        };
        let bc_sampled = compute_betweenness(&nodes, &forward, &config_sampled);

        // Both should produce results in [0, 1]
        for &score in bc_full.values() {
            assert!((0.0..=1.0).contains(&score));
        }
        for &score in bc_sampled.values() {
            assert!((0.0..=1.0).contains(&score));
        }

        // Middle nodes should have non-zero betweenness in both
        let mid = FunctionRef::new(PathBuf::from("node_5.py"), "func_5");
        assert!(bc_full.get(&mid).copied().unwrap_or(0.0) > 0.0);
        // Sampled might be 0 if source selection didn't include relevant nodes
        // but it shouldn't crash
    }

    #[test]
    fn betweenness_small_graph() {
        // Graphs with <= 2 nodes should return zeros
        let mut graph = ProjectCallGraph::new();
        graph.add_edge(CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "func_b".to_string(),
        });

        let forward = build_forward_graph(&graph);
        let nodes = collect_nodes(&graph);

        assert_eq!(nodes.len(), 2);

        let config = BetweennessConfig::default();
        let betweenness = compute_betweenness(&nodes, &forward, &config);

        // 2 nodes: all betweenness should be 0
        for &score in betweenness.values() {
            assert_eq!(score, 0.0);
        }
    }

    // -------------------------------------------------------------------------
    // Composite Score Tests
    // -------------------------------------------------------------------------

    #[test]
    fn composite_weighted_average() {
        // Test the weighted composite formula
        let in_deg = 0.4;
        let out_deg = 0.3;
        let pr = 0.5;
        let bc = 0.6;

        // Expected: (0.25*0.4 + 0.25*0.3 + 0.20*0.5 + 0.30*0.6) / 1.0
        //         = (0.1 + 0.075 + 0.1 + 0.18) / 1.0
        //         = 0.455
        let composite = compute_composite_score(in_deg, out_deg, Some(pr), Some(bc));

        let expected = WEIGHT_IN_DEGREE * in_deg
            + WEIGHT_OUT_DEGREE * out_deg
            + WEIGHT_PAGERANK * pr
            + WEIGHT_BETWEENNESS * bc;

        assert!(
            (composite - expected).abs() < 0.001,
            "Expected {}, got {}",
            expected,
            composite
        );
    }

    #[test]
    fn composite_partial_measures() {
        // When only in/out degree are available
        let in_deg = 0.6;
        let out_deg = 0.4;

        let composite = compute_composite_score(in_deg, out_deg, None, None);

        // Expected: (0.25*0.6 + 0.25*0.4) / (0.25 + 0.25) = 0.5
        let expected = (WEIGHT_IN_DEGREE * in_deg + WEIGHT_OUT_DEGREE * out_deg)
            / (WEIGHT_IN_DEGREE + WEIGHT_OUT_DEGREE);

        assert!(
            (composite - expected).abs() < 0.001,
            "Expected {}, got {}",
            expected,
            composite
        );
    }

    #[test]
    fn hub_score_with_all_measures() {
        let func = FunctionRef::new(PathBuf::from("test.py"), "test_func");
        let score = HubScore::with_all_measures(
            func, 0.4, // in_degree
            0.3, // out_degree
            0.5, // pagerank
            0.6, // betweenness
            5,   // callers_count
            4,   // callees_count
        );

        assert_eq!(score.pagerank, Some(0.5));
        assert_eq!(score.betweenness, Some(0.6));
        assert_eq!(score.callers_count, 5);
        assert_eq!(score.callees_count, 4);

        // Verify composite is calculated correctly
        let expected_composite = compute_composite_score(0.4, 0.3, Some(0.5), Some(0.6));
        assert!((score.composite_score - expected_composite).abs() < 0.001);
    }

    #[test]
    fn compute_hub_scores_full_integration() {
        let graph = create_diamond_graph();
        let forward = build_forward_graph(&graph);
        let reverse = build_reverse_graph(&graph);
        let nodes = collect_nodes(&graph);

        let (scores, pr_result) = compute_hub_scores_full(&nodes, &forward, &reverse, None, None);

        // PageRank should have converged
        assert!(pr_result.converged);

        // All scores should have all measures
        for score in &scores {
            assert!(
                score.pagerank.is_some(),
                "Missing pagerank for {}",
                score.name
            );
            assert!(
                score.betweenness.is_some(),
                "Missing betweenness for {}",
                score.name
            );
            assert!(score.in_degree >= 0.0 && score.in_degree <= 1.0);
            assert!(score.out_degree >= 0.0 && score.out_degree <= 1.0);
        }

        // Scores should be sorted by composite descending
        for window in scores.windows(2) {
            assert!(
                window[0].composite_score >= window[1].composite_score,
                "Not sorted: {} >= {}",
                window[0].composite_score,
                window[1].composite_score
            );
        }
    }
}
