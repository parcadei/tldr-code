//! Tests for the hotspot algorithm upgrade.
//!
//! These tests define the TARGET behavior after the upgrade from:
//!   hotspot_score = minmax(commit_count) * minmax(cognitive_complexity)
//! To:
//!   hotspot_score = 0.35*percentile(recency_weighted_relative_churn)
//!                 + 0.35*percentile(cognitive_complexity)
//!                 + 0.15*percentile(knowledge_fragmentation)
//!                 + 0.15*percentile(temporal_coupling)
//!
//! Spec: thoughts/hotspot-upgrade/spec.md

// Import existing types from tldr_core
use tldr_core::quality::hotspots::{
    calculate_trend, normalize_value, HotspotEntry, HotspotsMetadata, HotspotsOptions,
    HotspotsReport, HotspotsSummary, TrendDirection,
};

// Import NEW types and functions from the real implementation
use tldr_core::quality::hotspots::{
    composite_score_weighted, has_variance, is_bot_author, knowledge_fragmentation,
    percentile_ranks, recency_weight, relative_churn, ScoringWeights,
};

// ============================================================================
// Helper: composite_score wrapper using default weights
// ============================================================================

/// Convenience wrapper around composite_score_weighted using default weights.
fn composite_score(
    pct_churn: f64,
    pct_complexity: f64,
    pct_fragmentation: f64,
    pct_temporal_coupling: f64,
) -> f64 {
    let weights = ScoringWeights::default();
    composite_score_weighted(
        pct_churn,
        pct_complexity,
        pct_fragmentation,
        pct_temporal_coupling,
        &weights,
    )
}

// ============================================================================
// GROUP 1: Percentile Rank Normalization
//
// Formula: (average_rank - 1) / (N - 1) for N >= 2
// This maps the lowest value to 0.0 and highest to 1.0.
// ============================================================================

#[test]
fn test_percentile_rank_basic() {
    // [1, 2, 3, 4, 5] -> ranks [1, 2, 3, 4, 5]
    // percentiles = [(1-1)/4, (2-1)/4, (3-1)/4, (4-1)/4, (5-1)/4]
    //             = [0.0, 0.25, 0.5, 0.75, 1.0]
    let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = percentile_ranks(&values);
    assert_eq!(result.len(), 5);
    let expected = vec![0.0, 0.25, 0.5, 0.75, 1.0];
    for (got, exp) in result.iter().zip(expected.iter()) {
        assert!((got - exp).abs() < 0.001, "expected {}, got {}", exp, got);
    }
}

#[test]
fn test_percentile_rank_ties() {
    // [1, 1, 2, 3] -> sorted positions [1,2,3,4]
    // value 1 appears at positions 1,2 -> avg rank = 1.5
    // value 2 at position 3 -> rank = 3
    // value 3 at position 4 -> rank = 4
    // percentiles = [(1.5-1)/3, (1.5-1)/3, (3-1)/3, (4-1)/3]
    //             = [0.1667, 0.1667, 0.6667, 1.0]
    let values = vec![1.0, 1.0, 2.0, 3.0];
    let result = percentile_ranks(&values);
    assert_eq!(result.len(), 4);
    let expected = vec![0.5 / 3.0, 0.5 / 3.0, 2.0 / 3.0, 1.0];
    for (got, exp) in result.iter().zip(expected.iter()) {
        assert!((got - exp).abs() < 0.001, "expected {}, got {}", exp, got);
    }
}

#[test]
fn test_percentile_rank_single() {
    // Single value -> percentile = 1.0
    let values = vec![42.0];
    let result = percentile_ranks(&values);
    assert_eq!(result.len(), 1);
    assert!((result[0] - 1.0).abs() < 0.001);
}

#[test]
fn test_percentile_rank_empty() {
    let values: Vec<f64> = vec![];
    let result = percentile_ranks(&values);
    assert!(result.is_empty());
}

#[test]
fn test_percentile_rank_power_law() {
    // Power-law distribution: [1, 1, 1, 1, 100]
    // With min-max, 100 would dominate and compress all others to ~0.0.
    // With percentile ranking (N=5):
    //   value 1 at positions 1,2,3,4 -> avg rank = 2.5
    //   value 100 at position 5 -> rank = 5
    //   percentiles = [(2.5-1)/4, ..., (5-1)/4] = [0.375, 0.375, 0.375, 0.375, 1.0]
    //
    // Key insight: the outlier (100) gets 1.0 but the others get 0.375, NOT ~0.0.
    // This is the primary advantage over min-max normalization.
    let values = vec![1.0, 1.0, 1.0, 1.0, 100.0];
    let result = percentile_ranks(&values);
    assert_eq!(result.len(), 5);

    // All "1" values should have percentile 0.375, not near 0.0
    let expected_tied = (2.5 - 1.0) / 4.0; // 0.375
    for i in 0..4 {
        assert!(
            result[i] > 0.3,
            "Power-law: value=1 should have percentile > 0.3, got {}",
            result[i]
        );
        assert!(
            (result[i] - expected_tied).abs() < 0.001,
            "Expected {} for tied low values, got {}",
            expected_tied,
            result[i]
        );
    }
    // Outlier should be 1.0
    assert!((result[4] - 1.0).abs() < 0.001);

    // Compare with min-max to show the difference:
    // min-max would give 1 -> (1-1)/(100-1) = 0.0 for all "1" values
    let minmax_for_ones = normalize_value(1.0, 1.0, 100.0);
    assert!(
        minmax_for_ones < 0.02,
        "Min-max compresses low values: {}",
        minmax_for_ones
    );
    // Percentile ranking does NOT have this problem
    assert!(
        result[0] > minmax_for_ones + 0.3,
        "Percentile rank ({}) should be much higher than min-max ({}) for power-law data",
        result[0],
        minmax_for_ones
    );
}

#[test]
fn test_percentile_rank_all_same() {
    // All same values: [5, 5, 5, 5]
    // All tied at positions 1,2,3,4 -> avg rank = 2.5
    // percentile = (2.5 - 1) / (4 - 1) = 1.5 / 3.0 = 0.5
    let values = vec![5.0, 5.0, 5.0, 5.0];
    let result = percentile_ranks(&values);
    assert_eq!(result.len(), 4);
    let expected_pct = (2.5 - 1.0) / 3.0; // 0.5
    for (i, pct) in result.iter().enumerate() {
        assert!(
            (pct - expected_pct).abs() < 0.001,
            "All-same: index {} expected {}, got {}",
            i,
            expected_pct,
            pct
        );
    }
}

#[test]
fn test_percentile_rank_preserves_order() {
    // Input in non-sorted order: [5, 1, 3, 2, 4]
    // Sorted ranks: 1->rank1, 2->rank2, 3->rank3, 4->rank4, 5->rank5
    // Original order results should map correctly (N=5):
    //   index 0 (value 5) -> rank 5 -> (5-1)/4 = 1.0
    //   index 1 (value 1) -> rank 1 -> (1-1)/4 = 0.0
    //   index 2 (value 3) -> rank 3 -> (3-1)/4 = 0.5
    //   index 3 (value 2) -> rank 2 -> (2-1)/4 = 0.25
    //   index 4 (value 4) -> rank 4 -> (4-1)/4 = 0.75
    let values = vec![5.0, 1.0, 3.0, 2.0, 4.0];
    let result = percentile_ranks(&values);
    let expected = vec![1.0, 0.0, 0.5, 0.25, 0.75];
    for (i, (got, exp)) in result.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - exp).abs() < 0.001,
            "index {}: expected {}, got {}",
            i,
            exp,
            got
        );
    }
}

// ============================================================================
// GROUP 2: Relative Churn
// Uses MIN_LOC_FLOOR=10 as denominator floor (RISK-C6 mitigation)
// ============================================================================

#[test]
fn test_relative_churn_basic() {
    // 100 lines changed in a 1000-line file -> relative churn = 0.1
    assert!((relative_churn(100, 1000) - 0.1).abs() < 0.001);

    // 500 lines changed in a 500-line file -> relative churn = 1.0
    assert!((relative_churn(500, 500) - 1.0).abs() < 0.001);

    // 10 lines changed in a 10000-line file -> relative churn = 0.001
    assert!((relative_churn(10, 10000) - 0.001).abs() < 0.001);
}

#[test]
fn test_relative_churn_zero_loc() {
    // File with 0 LOC (empty file) should not cause division by zero.
    // With MIN_LOC_FLOOR=10: lines_changed / max(0, 10) = 50 / 10 = 5.0
    let result = relative_churn(50, 0);
    assert!(
        (result - 5.0).abs() < 0.001,
        "0 LOC: expected 5.0, got {}",
        result
    );

    // 0 changes, 0 LOC -> 0.0
    let result2 = relative_churn(0, 0);
    assert!((result2 - 0.0).abs() < 0.001);
}

#[test]
fn test_relative_churn_more_changed_than_total() {
    // lines_changed > current_loc (file was rewritten multiple times).
    // Spec says relative_churn can be > 1.0 (unbounded above).
    // 5000 / max(100, 10) = 5000 / 100 = 50.0
    let result = relative_churn(5000, 100);
    assert!(
        (result - 50.0).abs() < 0.001,
        "Expected 50.0, got {}",
        result
    );
    assert!(
        result > 1.0,
        "Relative churn should be > 1.0 when lines_changed > loc"
    );
}

// ============================================================================
// GROUP 3: Recency Weighting
// ============================================================================

#[test]
fn test_recency_weight_today() {
    // age = 0 days -> weight = e^0 = 1.0
    let w = recency_weight(0.0, 90.0);
    assert!(
        (w - 1.0).abs() < 0.001,
        "age=0 should give weight=1.0, got {}",
        w
    );
}

#[test]
fn test_recency_weight_halflife() {
    // age = halflife (90 days) -> weight = e^(-ln(2)) = 0.5
    let w = recency_weight(90.0, 90.0);
    assert!(
        (w - 0.5).abs() < 0.01,
        "age=halflife should give weight~0.5, got {}",
        w
    );
}

#[test]
fn test_recency_weight_double_halflife() {
    // age = 2*halflife (180 days) -> weight = e^(-2*ln(2)) = 0.25
    let w = recency_weight(180.0, 90.0);
    assert!(
        (w - 0.25).abs() < 0.01,
        "age=2*halflife should give weight~0.25, got {}",
        w
    );
}

#[test]
fn test_recency_weight_old() {
    // age = 365 days with halflife=90 -> weight = e^(-ln(2)*365/90) ~ 0.062
    let w = recency_weight(365.0, 90.0);
    assert!(
        w < 0.10,
        "1-year-old commit should have very low weight, got {}",
        w
    );
    assert!(w > 0.0, "Weight should never be exactly 0");
    // More precisely: e^(-0.6931*4.056) = e^(-2.812) ~ 0.060
    assert!(
        (w - 0.060).abs() < 0.01,
        "Expected ~0.060 for 365-day age with 90-day halflife, got {}",
        w
    );
}

#[test]
fn test_recency_weight_zero_halflife() {
    // halflife = 0 means no decay -> all weights = 1.0 (legacy behavior)
    let w = recency_weight(365.0, 0.0);
    assert!(
        (w - 1.0).abs() < 0.001,
        "halflife=0 should give weight=1.0, got {}",
        w
    );

    let w2 = recency_weight(0.0, 0.0);
    assert!((w2 - 1.0).abs() < 0.001);
}

#[test]
fn test_recency_weight_custom_halflife() {
    // halflife = 30 days (aggressive decay)
    let w30 = recency_weight(30.0, 30.0);
    assert!(
        (w30 - 0.5).abs() < 0.01,
        "halflife=30, age=30 -> ~0.5, got {}",
        w30
    );

    // halflife = 180 days (gentle decay)
    let w180 = recency_weight(90.0, 180.0);
    // At age 90 with halflife 180: weight = e^(-ln(2)*90/180) = e^(-ln(2)/2) ~ 0.707
    assert!(
        (w180 - 0.707).abs() < 0.01,
        "halflife=180, age=90 -> ~0.707, got {}",
        w180
    );
}

// ============================================================================
// GROUP 4: Bot Filtering
// ============================================================================

#[test]
fn test_bot_filter_dependabot() {
    assert!(is_bot_author(
        "dependabot[bot]",
        "dependabot[bot]@users.noreply.github.com"
    ));
}

#[test]
fn test_bot_filter_renovate() {
    assert!(is_bot_author(
        "renovate[bot]",
        "renovate[bot]@users.noreply.github.com"
    ));
}

#[test]
fn test_bot_filter_github_actions() {
    assert!(is_bot_author(
        "github-actions[bot]",
        "github-actions[bot]@users.noreply.github.com"
    ));
}

#[test]
fn test_bot_filter_snyk() {
    assert!(is_bot_author("snyk-bot", "snyk-bot@snyk.io"));
}

#[test]
fn test_bot_filter_generic_bot_suffix() {
    // Any author with [bot] in name should match
    assert!(is_bot_author("my-custom-app[bot]", "custom@example.com"));
}

#[test]
fn test_bot_filter_human_preserved() {
    // Regular human authors should NOT be filtered
    assert!(!is_bot_author("John Smith", "john@company.com"));
    assert!(!is_bot_author("Alice Developer", "alice@dev.org"));
    assert!(!is_bot_author("robot-enthusiast", "robot@company.com"));
}

#[test]
fn test_bot_filter_case_insensitive() {
    // Bot matching should be case-insensitive
    assert!(is_bot_author("Dependabot[Bot]", "DEPENDABOT@github.com"));
    assert!(is_bot_author("RENOVATE[BOT]", "renovate@example.com"));
}

#[test]
fn test_bot_filter_email_match() {
    // Should match on email even if name looks human
    assert!(is_bot_author(
        "Dependency Updater",
        "dependabot@users.noreply.github.com"
    ));
}

#[test]
fn test_bot_filter_partial_no_false_positive() {
    // "robotics-engineer" should NOT match just because it contains "bot"
    // The patterns are specific: "dependabot", "renovate", "[bot]", etc.
    // "bot" alone is NOT a pattern.
    assert!(!is_bot_author("robotics-engineer", "robotics@company.com"));
    assert!(!is_bot_author("abbott", "abbott@company.com"));
}

// ============================================================================
// GROUP 5: Knowledge Fragmentation
// ============================================================================

#[test]
fn test_knowledge_frag_single_author() {
    // 1 author = 0.0 fragmentation (single owner)
    let authors = vec![("alice@dev.com".to_string(), 50u32)];
    let frag = knowledge_fragmentation(&authors);
    assert!(
        frag.abs() < 0.001,
        "Single author -> fragmentation = 0.0, got {}",
        frag
    );
}

#[test]
fn test_knowledge_frag_even_split() {
    // 2 authors, 50/50 split
    // top_fraction = 50/100 = 0.5
    // fragmentation = 1.0 - 0.5 = 0.5
    let authors = vec![
        ("alice@dev.com".to_string(), 50),
        ("bob@dev.com".to_string(), 50),
    ];
    let frag = knowledge_fragmentation(&authors);
    assert!(
        (frag - 0.5).abs() < 0.001,
        "Two 50/50 authors -> fragmentation = 0.5, got {}",
        frag
    );
}

#[test]
fn test_knowledge_frag_dominant_author() {
    // 1 author has 90% of commits, 3 minor authors have 10%
    // top_fraction = 90/100 = 0.9
    // fragmentation = 1.0 - 0.9 = 0.1
    // minor contributors with < 5% of 100 = < 5 commits: authors with ~3 commits each
    // 3 minor contributors is not > 3, so no penalty
    let authors = vec![
        ("alice@dev.com".to_string(), 90),
        ("bob@dev.com".to_string(), 4),
        ("carol@dev.com".to_string(), 3),
        ("dave@dev.com".to_string(), 3),
    ];
    let frag = knowledge_fragmentation(&authors);
    assert!(
        frag < 0.15,
        "Dominant author (90%) -> low fragmentation, got {}",
        frag
    );
    assert!((frag - 0.1).abs() < 0.05, "Expected ~0.1, got {}", frag);
}

#[test]
fn test_knowledge_frag_many_minor() {
    // 10 authors each with 10% of commits = 10 commits each, total 100
    // top_fraction = 10/100 = 0.1
    // fragmentation = 1.0 - 0.1 = 0.9
    // 5% threshold = 5 commits. Authors with < 5 commits: none (all have 10)
    let authors: Vec<(String, u32)> = (0..10)
        .map(|i| (format!("author{}@dev.com", i), 10))
        .collect();
    let frag = knowledge_fragmentation(&authors);
    assert!(
        frag > 0.8,
        "10 equal authors -> high fragmentation, got {}",
        frag
    );
}

#[test]
fn test_knowledge_frag_many_minor_with_penalty() {
    // 1 author with 50 commits, 5 authors with 1 commit each = 55 total
    // top_fraction = 50/55 = 0.909
    // base fragmentation = 1.0 - 0.909 = 0.091
    // 5% of 55 = 2.75 -> threshold = max(2, 1) = 2 commits
    // Authors with < 2 commits: 5 authors (each has 1)
    // 5 > 3 -> penalty applies: 0.091 * 1.2 = 0.109
    let authors = vec![
        ("alice@dev.com".to_string(), 50),
        ("minor1@dev.com".to_string(), 1),
        ("minor2@dev.com".to_string(), 1),
        ("minor3@dev.com".to_string(), 1),
        ("minor4@dev.com".to_string(), 1),
        ("minor5@dev.com".to_string(), 1),
    ];
    let frag_with_penalty = knowledge_fragmentation(&authors);

    // Without penalty it would be ~0.091, with penalty ~0.109
    // The penalty should make it slightly higher
    let top_fraction = 50.0 / 55.0;
    let base_frag = 1.0 - top_fraction;
    assert!(
        frag_with_penalty > base_frag,
        "Penalty should increase fragmentation: {} > {}",
        frag_with_penalty,
        base_frag
    );
    assert!(
        (frag_with_penalty - base_frag * 1.2).abs() < 0.01,
        "Expected ~{}, got {}",
        base_frag * 1.2,
        frag_with_penalty
    );
}

#[test]
fn test_knowledge_frag_empty() {
    // No authors at all -> 0.0 fragmentation
    let authors: Vec<(String, u32)> = vec![];
    let frag = knowledge_fragmentation(&authors);
    assert!((frag - 0.0).abs() < 0.001);
}

#[test]
fn test_knowledge_frag_capped_at_one() {
    // Even with penalty, fragmentation should not exceed 1.0
    // total = 100, top author = 10 commits, 90 authors with 1 commit each
    let mut authors: Vec<(String, u32)> = vec![("top@dev.com".to_string(), 10)];
    for i in 0..90 {
        authors.push((format!("minor{}@dev.com", i), 1));
    }
    // total = 100, top_fraction = 10/100 = 0.1
    // base_frag = 0.9
    // 5% of 100 = 5. Authors with < 5 commits: 90 authors (all with 1)
    // 90 > 3 -> penalty: 0.9 * 1.2 = 1.08 -> capped at 1.0
    let frag = knowledge_fragmentation(&authors);
    assert!(
        frag <= 1.0,
        "Fragmentation should be capped at 1.0, got {}",
        frag
    );
    assert!(
        (frag - 1.0).abs() < 0.001,
        "Expected 1.0 (capped), got {}",
        frag
    );
}

// ============================================================================
// GROUP 6: Composite Score
// ============================================================================

#[test]
fn test_composite_score_weights_sum_to_one() {
    let weights = ScoringWeights::default();
    let sum = weights.churn
        + weights.complexity
        + weights.knowledge_fragmentation
        + weights.temporal_coupling;
    assert!(
        (sum - 1.0).abs() < 0.001,
        "Weights should sum to 1.0, got {}",
        sum
    );
}

#[test]
fn test_composite_score_weights_values() {
    let weights = ScoringWeights::default();
    assert!((weights.churn - 0.35).abs() < 0.001);
    assert!((weights.complexity - 0.35).abs() < 0.001);
    assert!((weights.knowledge_fragmentation - 0.15).abs() < 0.001);
    assert!((weights.temporal_coupling - 0.15).abs() < 0.001);
}

#[test]
fn test_composite_score_formula() {
    // All dimensions at max (1.0): score = 0.35 + 0.35 + 0.15 + 0.15 = 1.0
    let score = composite_score(1.0, 1.0, 1.0, 1.0);
    assert!((score - 1.0).abs() < 0.001);

    // All dimensions at 0.0: score = 0.0
    let score_zero = composite_score(0.0, 0.0, 0.0, 0.0);
    assert!((score_zero - 0.0).abs() < 0.001);

    // Mixed: churn=0.8, complexity=0.6, fragmentation=0.4, temporal=0.0
    // = 0.35*0.8 + 0.35*0.6 + 0.15*0.4 + 0.15*0.0
    // = 0.28 + 0.21 + 0.06 + 0.0 = 0.55
    let score_mixed = composite_score(0.8, 0.6, 0.4, 0.0);
    assert!(
        (score_mixed - 0.55).abs() < 0.001,
        "Expected 0.55, got {}",
        score_mixed
    );
}

#[test]
fn test_composite_score_max_phase1() {
    // In Phase 1, temporal_coupling = 0.0 for all files.
    // Maximum possible score = 0.35*1.0 + 0.35*1.0 + 0.15*1.0 + 0.15*0.0 = 0.85
    let max_phase1 = composite_score(1.0, 1.0, 1.0, 0.0);
    assert!(
        (max_phase1 - 0.85).abs() < 0.001,
        "Phase 1 max score should be 0.85, got {}",
        max_phase1
    );
}

#[test]
fn test_composite_vs_multiplicative() {
    // The OLD formula was multiplicative: churn * complexity
    // This means if churn=1.0 but complexity=0.0, old score=0.0.
    // The NEW formula is additive-weighted, so score > 0.0.
    let pct_churn = 1.0;
    let pct_complexity = 0.0;
    let pct_frag = 0.5;

    let old_score = pct_churn * pct_complexity; // = 0.0
    let new_score = composite_score(pct_churn, pct_complexity, pct_frag, 0.0);
    // new = 0.35*1.0 + 0.35*0.0 + 0.15*0.5 + 0.0 = 0.35 + 0 + 0.075 = 0.425

    assert!((old_score - 0.0).abs() < 0.001, "Old multiplicative = 0.0");
    assert!(
        new_score > 0.4,
        "New composite should be > 0.4, got {}",
        new_score
    );
    assert!(
        (new_score - 0.425).abs() < 0.001,
        "Expected 0.425, got {}",
        new_score
    );
}

#[test]
fn test_composite_no_temporal() {
    // When temporal_coupling not computed (Phase 1), score uses 3 dimensions.
    // The score should still be valid and meaningful.
    let score = composite_score(0.8, 0.7, 0.6, 0.0);
    // = 0.35*0.8 + 0.35*0.7 + 0.15*0.6 + 0
    // = 0.28 + 0.245 + 0.09 = 0.615
    assert!(
        (score - 0.615).abs() < 0.001,
        "Expected 0.615, got {}",
        score
    );
    // Score should be > 0 and < 0.85 (Phase 1 max)
    assert!(score > 0.0);
    assert!(score <= 0.85 + 0.001);
}

// ============================================================================
// GROUP 7: Text Formatter (output quality)
//
// These tests verify that the upgraded text output does NOT use Unicode
// box-drawing characters. They test against the format_hotspots_text function
// which lives in the CLI crate, but we test the contract here.
// ============================================================================

/// Box-drawing Unicode characters that should NOT appear in plain-text output.
const BOX_DRAWING_CHARS: &[char] = &[
    '\u{2550}', '\u{2551}', '\u{2552}', '\u{2553}', '\u{2554}', '\u{2555}', '\u{2556}', '\u{2557}',
    '\u{2558}', '\u{2559}', '\u{255A}', '\u{255B}', '\u{255C}', '\u{255D}', '\u{255E}', '\u{255F}',
    '\u{2560}', '\u{2561}', '\u{2562}', '\u{2563}', '\u{2564}', '\u{2565}', '\u{2566}', '\u{2567}',
    '\u{2568}', '\u{2569}', '\u{256A}', '\u{256B}', '\u{256C}', '\u{2500}', '\u{2502}', '\u{250C}',
    '\u{2510}', '\u{2514}', '\u{2518}', '\u{251C}', '\u{2524}', '\u{252C}', '\u{2534}', '\u{253C}',
];

/// Helper: Format a minimal HotspotsReport as plain text.
/// This simulates what format_hotspots_text should produce after the upgrade.
///
/// TARGET: This helper will be replaced by calling the real
/// tldr_cli::commands::hotspots::format_hotspots_text function.
fn format_hotspots_text_stub(report: &HotspotsReport) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        " {:>3}  {:>5}  {:>5}  {:>5}  {:>7}  {:>4}  {}\n",
        "#", "Score", "Churn", "Cmplx", "Commits", "Cog", "File"
    ));
    for (i, hotspot) in report.hotspots.iter().enumerate() {
        output.push_str(&format!(
            " {:>3}  {:>5.2}  {:>5.2}  {:>5.2}  {:>7}  {:>4}  {}\n",
            i + 1,
            hotspot.hotspot_score,
            hotspot.churn_score,
            hotspot.complexity_score,
            hotspot.commit_count,
            hotspot.complexity,
            hotspot.file
        ));
    }
    output
}

fn make_test_report() -> HotspotsReport {
    HotspotsReport {
        hotspots: vec![
            HotspotEntry {
                file: "src/main.rs".to_string(),
                function: None,
                line: None,
                churn_score: 0.85,
                complexity_score: 0.90,
                hotspot_score: 0.78,
                commit_count: 42,
                lines_changed: 500,
                complexity: 25,
                trend: None,
                recommendation: "Critical: High churn + high complexity. Prioritize refactoring."
                    .to_string(),
                relative_churn: Some(0.5),
                knowledge_fragmentation: Some(0.3),
                current_loc: Some(1000),
                author_count: Some(5),
                algorithm_version: 2,
            },
            HotspotEntry {
                file: "src/lib.rs".to_string(),
                function: None,
                line: None,
                churn_score: 0.60,
                complexity_score: 0.40,
                hotspot_score: 0.45,
                commit_count: 20,
                lines_changed: 200,
                complexity: 12,
                trend: None,
                recommendation: "Medium priority: Consider simplification.".to_string(),
                relative_churn: Some(0.1),
                knowledge_fragmentation: Some(0.1),
                current_loc: Some(2000),
                author_count: Some(2),
                algorithm_version: 2,
            },
        ],
        summary: HotspotsSummary {
            total_files_analyzed: 2,
            total_commits: 62,
            time_window_days: 365,
            hotspot_concentration: 80.0,
            recommendation: "High concentration.".to_string(),
            total_bot_commits_filtered: None,
            avg_knowledge_fragmentation: None,
        },
        metadata: HotspotsMetadata {
            path: ".".to_string(),
            days: 365,
            by_function: false,
            min_commits: 3,
            is_shallow: false,
            shallow_depth: None,
            bot_commits_filtered: None,
            recency_halflife: None,
            scoring_weights: None,
            algorithm_version: 2,
        },
        warnings: vec![],
    }
}

#[test]
fn test_text_format_no_box_drawing() {
    let report = make_test_report();
    let output = format_hotspots_text_stub(&report);

    for ch in BOX_DRAWING_CHARS {
        assert!(
            !output.contains(*ch),
            "Text output should not contain box-drawing character U+{:04X} ('{}')",
            *ch as u32,
            ch
        );
    }
}

#[test]
fn test_text_format_has_table() {
    let report = make_test_report();
    let output = format_hotspots_text_stub(&report);

    // Should contain column headers
    assert!(
        output.contains("Score"),
        "Output should contain 'Score' header"
    );
    assert!(
        output.contains("Churn"),
        "Output should contain 'Churn' header"
    );
    assert!(
        output.contains("File"),
        "Output should contain 'File' header"
    );

    // Should contain file paths from the report
    assert!(output.contains("src/main.rs"));
    assert!(output.contains("src/lib.rs"));

    // Should have multiple lines
    let lines: Vec<&str> = output.lines().collect();
    assert!(
        lines.len() >= 3,
        "Should have header + at least 2 data rows"
    );
}

// ============================================================================
// GROUP 8: Backwards Compatibility (struct fields)
//
// These tests verify that the UPGRADED types have the NEW fields required
// by the spec.
// ============================================================================

#[test]
fn test_hotspot_entry_has_new_fields() {
    let entry = HotspotEntry {
        file: "test.rs".to_string(),
        function: None,
        line: None,
        churn_score: 0.5,
        complexity_score: 0.5,
        hotspot_score: 0.25,
        commit_count: 10,
        lines_changed: 100,
        complexity: 15,
        trend: None,
        recommendation: "Monitor for changes.".to_string(),
        relative_churn: Some(0.1),
        knowledge_fragmentation: Some(0.3),
        current_loc: Some(1000),
        author_count: Some(5),
        algorithm_version: 2,
    };

    assert_eq!(entry.file, "test.rs");
    assert_eq!(entry.commit_count, 10);
    assert_eq!(entry.relative_churn, Some(0.1));
    assert_eq!(entry.knowledge_fragmentation, Some(0.3));
    assert_eq!(entry.current_loc, Some(1000));
    assert_eq!(entry.author_count, Some(5));
    assert_eq!(entry.algorithm_version, 2);

    let json = serde_json::to_value(&entry).unwrap();
    assert!(
        json.get("relative_churn").is_some(),
        "HotspotEntry should have relative_churn field"
    );
    assert!(
        json.get("knowledge_fragmentation").is_some(),
        "HotspotEntry should have knowledge_fragmentation field"
    );
    assert!(
        json.get("current_loc").is_some(),
        "HotspotEntry should have current_loc field"
    );
    assert!(
        json.get("author_count").is_some(),
        "HotspotEntry should have author_count field"
    );
    assert!(
        json.get("algorithm_version").is_some(),
        "HotspotEntry should have algorithm_version field"
    );
}

#[test]
fn test_hotspot_entry_none_fields_skipped_in_json() {
    let entry = HotspotEntry {
        file: "test.rs".to_string(),
        function: None,
        line: None,
        churn_score: 0.5,
        complexity_score: 0.5,
        hotspot_score: 0.25,
        commit_count: 10,
        lines_changed: 100,
        complexity: 15,
        trend: None,
        recommendation: "Monitor".to_string(),
        relative_churn: None,
        knowledge_fragmentation: None,
        current_loc: None,
        author_count: None,
        algorithm_version: 2,
    };

    let json = serde_json::to_value(&entry).unwrap();
    assert!(json.get("relative_churn").is_none());
    assert!(json.get("knowledge_fragmentation").is_none());
    assert!(json.get("current_loc").is_none());
    assert!(json.get("author_count").is_none());
}

#[test]
fn test_options_has_new_flags() {
    let opts = HotspotsOptions::new();

    assert_eq!(opts.days, 365);
    assert_eq!(opts.top, 20);
    assert_eq!(opts.min_commits, 3);

    assert!((opts.recency_halflife - 90.0).abs() < 0.001);
    assert!(!opts.include_bots);

    let opts2 = HotspotsOptions::new()
        .with_recency_halflife(30.0)
        .with_include_bots(true);
    assert!((opts2.recency_halflife - 30.0).abs() < 0.001);
    assert!(opts2.include_bots);
}

#[test]
fn test_metadata_has_bot_count() {
    let metadata = HotspotsMetadata {
        path: ".".to_string(),
        days: 365,
        by_function: false,
        min_commits: 3,
        is_shallow: false,
        shallow_depth: None,
        bot_commits_filtered: Some(42),
        recency_halflife: Some(90),
        scoring_weights: Some(ScoringWeights::default()),
        algorithm_version: 2,
    };

    let json = serde_json::to_value(&metadata).unwrap();
    assert!(json.get("bot_commits_filtered").is_some());
    assert!(json.get("recency_halflife").is_some());
    assert!(json.get("scoring_weights").is_some());
    assert!(json.get("algorithm_version").is_some());
}

#[test]
fn test_summary_has_new_fields() {
    let summary = HotspotsSummary {
        total_files_analyzed: 10,
        total_commits: 100,
        time_window_days: 365,
        hotspot_concentration: 70.0,
        recommendation: "Focus on top hotspots.".to_string(),
        total_bot_commits_filtered: Some(25),
        avg_knowledge_fragmentation: Some(0.4),
    };

    let json = serde_json::to_value(&summary).unwrap();
    assert!(json.get("total_bot_commits_filtered").is_some());
    assert!(json.get("avg_knowledge_fragmentation").is_some());
}

// ============================================================================
// GROUP 9: Scoring Semantics (behavior contracts for the upgraded algorithm)
// ============================================================================

#[test]
fn test_percentile_beats_minmax_for_outliers() {
    // Scenario: 6 files with churn values [1, 3, 3, 5, 10, 100]
    // Min-max would give file with churn=10: (10-1)/(100-1) = 0.091
    // Percentile gives file with churn=10 (N=6):
    //   rank=5, percentile = (5-1)/(6-1) = 0.8
    let values = vec![1.0, 3.0, 3.0, 5.0, 10.0, 100.0];
    let pct = percentile_ranks(&values);

    assert!(
        pct[4] > 0.75,
        "File with churn=10 should have percentile > 0.75, got {}",
        pct[4]
    );

    let minmax_score = normalize_value(10.0, 1.0, 100.0);
    assert!(
        pct[4] > minmax_score * 5.0,
        "Percentile ({}) should be much higher than min-max ({}) for penultimate value",
        pct[4],
        minmax_score
    );
}

#[test]
fn test_relative_churn_discriminates_file_size() {
    let churn_a = relative_churn(100, 100); // small file
    let churn_b = relative_churn(100, 10000); // large file

    assert!(
        churn_a > churn_b * 50.0,
        "Small file ({}) should have much higher relative churn than large file ({})",
        churn_a,
        churn_b
    );
}

#[test]
fn test_recency_favors_recent_changes() {
    let weight_a = recency_weight(7.0, 90.0);
    let weight_b = recency_weight(300.0, 90.0);

    assert!(
        weight_a > weight_b * 5.0,
        "Recent file weight ({}) should be much higher than old file weight ({})",
        weight_a,
        weight_b
    );
    assert!(weight_a > 0.9);
    assert!(weight_b < 0.15);
}

// ============================================================================
// GROUP 10: Edge Cases from Spec
// ============================================================================

#[test]
fn test_single_file_percentile() {
    let values = vec![42.0];
    let pct = percentile_ranks(&values);
    assert!((pct[0] - 1.0).abs() < 0.001);
}

#[test]
fn test_all_files_same_churn_percentile() {
    // N=5, all tied: average rank = (1+2+3+4+5)/5 = 3.0
    // percentile = (3.0 - 1) / (5 - 1) = 2.0 / 4.0 = 0.5
    let values = vec![10.0, 10.0, 10.0, 10.0, 10.0];
    let pct = percentile_ranks(&values);
    for p in &pct {
        assert!(
            (p - 0.5).abs() < 0.001,
            "All same values should have equal percentile 0.5, got {}",
            p
        );
    }
}

#[test]
fn test_empty_file_relative_churn() {
    let churn = relative_churn(10, 0);
    assert!(
        churn.is_finite(),
        "Relative churn for empty file should be finite"
    );
    assert!(churn >= 0.0, "Relative churn should be non-negative");
}

#[test]
fn test_halflife_zero_preserves_legacy() {
    for age in &[0.0, 30.0, 90.0, 180.0, 365.0] {
        let w = recency_weight(*age, 0.0);
        assert!(
            (w - 1.0).abs() < 0.001,
            "halflife=0, age={}: weight should be 1.0, got {}",
            age,
            w
        );
    }
}

#[test]
fn test_existing_normalize_value_still_works() {
    assert!((normalize_value(50.0, 0.0, 100.0) - 0.5).abs() < 0.001);
    assert!((normalize_value(0.0, 0.0, 100.0) - 0.0).abs() < 0.001);
    assert!((normalize_value(100.0, 0.0, 100.0) - 1.0).abs() < 0.001);
    assert!((normalize_value(50.0, 50.0, 50.0) - 0.5).abs() < 0.001);
}

#[test]
fn test_existing_calculate_trend_still_works() {
    assert_eq!(calculate_trend(-5), TrendDirection::Improving);
    assert_eq!(calculate_trend(0), TrendDirection::Stable);
    assert_eq!(calculate_trend(2), TrendDirection::Stable);
    assert_eq!(calculate_trend(-2), TrendDirection::Stable);
    assert_eq!(calculate_trend(5), TrendDirection::Degrading);
}

#[test]
fn test_existing_options_builder_still_works() {
    let opts = HotspotsOptions::new()
        .with_days(30)
        .with_top(10)
        .with_min_commits(5)
        .with_by_function(true)
        .with_show_trend(true)
        .with_exclude(vec!["*.lock".to_string()])
        .with_threshold(0.5)
        .with_since("2025-01-01".to_string());

    assert_eq!(opts.days, 30);
    assert_eq!(opts.top, 10);
    assert_eq!(opts.min_commits, 5);
    assert!(opts.by_function);
    assert!(opts.show_trend);
    assert_eq!(opts.exclude, vec!["*.lock".to_string()]);
    assert_eq!(opts.threshold, Some(0.5));
    assert_eq!(opts.since, Some("2025-01-01".to_string()));
}

// ============================================================================
// GROUP 11: ScoringWeights advanced behavior
// ============================================================================

#[test]
fn test_scoring_weights_renormalize() {
    let weights = ScoringWeights {
        churn: 0.35,
        complexity: 0.35,
        knowledge_fragmentation: 0.15,
        temporal_coupling: 0.0,
    };
    let renormed = weights.renormalize();
    let sum = renormed.churn
        + renormed.complexity
        + renormed.knowledge_fragmentation
        + renormed.temporal_coupling;
    assert!(
        (sum - 1.0).abs() < 0.001,
        "Renormalized weights should sum to 1.0, got {}",
        sum
    );
    assert!((renormed.temporal_coupling - 0.0).abs() < 0.001);
}

#[test]
fn test_scoring_weights_default_phase1() {
    let w = ScoringWeights::default_phase1();
    let sum = w.churn + w.complexity + w.knowledge_fragmentation + w.temporal_coupling;
    assert!(
        (sum - 1.0).abs() < 0.001,
        "Phase 1 weights should sum to 1.0, got {}",
        sum
    );
    assert!(
        (w.temporal_coupling - 0.0).abs() < 0.001,
        "Phase 1 should have 0 temporal"
    );
    assert!((w.churn - 0.4118).abs() < 0.001);
    assert!((w.complexity - 0.4118).abs() < 0.001);
    assert!((w.knowledge_fragmentation - 0.1765).abs() < 0.001);
}

#[test]
fn test_scoring_weights_for_active_dimensions() {
    let w = ScoringWeights::default();
    let active = [false, true, true, false];
    let adjusted = w.for_active_dimensions(active);
    assert!((adjusted.churn - 0.0).abs() < 0.001);
    assert!((adjusted.temporal_coupling - 0.0).abs() < 0.001);
    let sum = adjusted.complexity + adjusted.knowledge_fragmentation;
    assert!((sum - 1.0).abs() < 0.001);
}

#[test]
fn test_has_variance_all_same() {
    assert!(!has_variance(&[5.0, 5.0, 5.0, 5.0]));
}

#[test]
fn test_has_variance_different() {
    assert!(has_variance(&[5.0, 5.0, 5.0, 6.0]));
}

#[test]
fn test_has_variance_single() {
    assert!(!has_variance(&[42.0]));
}

#[test]
fn test_has_variance_empty() {
    assert!(!has_variance(&[]));
}
