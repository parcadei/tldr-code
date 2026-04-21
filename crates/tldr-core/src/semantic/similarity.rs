//! Cosine similarity and top-K selection for semantic search
//!
//! This module provides vector similarity operations used by the semantic
//! search system to rank embeddings by relevance.
//!
//! # Functions
//!
//! - [`cosine_similarity`]: Compute similarity between two vectors
//! - [`normalize`]: Normalize a vector to unit length
//! - [`is_normalized`]: Check if a vector has unit length
//! - [`top_k_similar`]: Find the K most similar vectors to a query
//!
//! # Example
//!
//! ```rust
//! use tldr_core::semantic::similarity::{cosine_similarity, normalize, top_k_similar};
//!
//! // Normalize vectors before comparison
//! let mut query = vec![1.0, 0.0, 0.0];
//! normalize(&mut query);
//!
//! let mut candidate = vec![0.9, 0.1, 0.0];
//! normalize(&mut candidate);
//!
//! let sim = cosine_similarity(&query, &candidate);
//! assert!(sim > 0.9); // High similarity
//! ```
//!
//! # Performance
//!
//! - `cosine_similarity`: O(n) where n = vector dimensions
//! - `top_k_similar`: O(m * n) where m = candidates, n = dimensions
//!
//! For 10K functions with 768-dim embeddings: ~7.68M operations

/// Epsilon tolerance for floating point comparisons
const EPSILON: f64 = 1e-6;

/// Compute cosine similarity between two vectors
///
/// Returns 1.0 for identical normalized vectors, 0.0 for orthogonal vectors,
/// and -1.0 for opposite vectors.
///
/// # Arguments
///
/// * `a` - First vector
/// * `b` - Second vector (must have same length as `a`)
///
/// # Returns
///
/// Cosine similarity value in range [-1.0, 1.0]
///
/// # Panics
///
/// Panics if vectors have different lengths.
///
/// # P0 Mitigation
///
/// Returns 0.0 for zero vectors (not NaN) to prevent propagation of
/// undefined values through the search pipeline.
///
/// # Example
///
/// ```rust
/// use tldr_core::semantic::similarity::cosine_similarity;
///
/// // Identical normalized vectors
/// let a = vec![0.6_f32, 0.8, 0.0];
/// let b = vec![0.6_f32, 0.8, 0.0];
/// let sim = cosine_similarity(&a, &b);
/// assert!((sim - 1.0).abs() < 1e-6);
///
/// // Orthogonal vectors
/// let a = vec![1.0_f32, 0.0, 0.0];
/// let b = vec![0.0_f32, 1.0, 0.0];
/// let sim = cosine_similarity(&a, &b);
/// assert!(sim.abs() < 1e-6);
/// ```
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(
        a.len(),
        b.len(),
        "Vectors must have same length: {} vs {}",
        a.len(),
        b.len()
    );

    // Compute dot product and magnitudes in a single pass
    let mut dot_product: f64 = 0.0;
    let mut norm_a: f64 = 0.0;
    let mut norm_b: f64 = 0.0;

    for (ai, bi) in a.iter().zip(b.iter()) {
        let ai_f64 = *ai as f64;
        let bi_f64 = *bi as f64;
        dot_product += ai_f64 * bi_f64;
        norm_a += ai_f64 * ai_f64;
        norm_b += bi_f64 * bi_f64;
    }

    // P0 Mitigation: Handle zero vectors explicitly
    // If either vector has zero norm, return 0.0 (not NaN)
    let magnitude = (norm_a * norm_b).sqrt();
    if magnitude < EPSILON {
        return 0.0;
    }

    dot_product / magnitude
}

/// Normalize a vector in place to unit length (L2 norm = 1.0)
///
/// After normalization, the vector will have L2 norm equal to 1.0,
/// making it suitable for cosine similarity comparisons.
///
/// # Arguments
///
/// * `v` - Vector to normalize (modified in place)
///
/// # Behavior
///
/// - For non-zero vectors: scales to unit length
/// - For zero vectors: leaves unchanged (all zeros)
///
/// # Example
///
/// ```rust
/// use tldr_core::semantic::similarity::normalize;
///
/// let mut v = vec![3.0_f32, 4.0, 0.0]; // Length 5
/// normalize(&mut v);
///
/// // Check L2 norm is 1.0
/// let l2_norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
/// assert!((l2_norm - 1.0).abs() < 1e-6);
///
/// // Check components (3/5 = 0.6, 4/5 = 0.8)
/// assert!((v[0] - 0.6).abs() < 1e-6);
/// assert!((v[1] - 0.8).abs() < 1e-6);
/// ```
pub fn normalize(v: &mut [f32]) {
    // Compute L2 norm
    let norm: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();

    // P0 Mitigation: Don't divide by zero
    if norm < EPSILON {
        return;
    }

    // Scale each component
    let scale = 1.0 / norm;
    for x in v.iter_mut() {
        *x = (*x as f64 * scale) as f32;
    }
}

/// Check if a vector is normalized (L2 norm approximately 1.0)
///
/// Uses a tolerance of 1e-4 to account for floating point precision.
///
/// # Arguments
///
/// * `v` - Vector to check
///
/// # Returns
///
/// `true` if the vector's L2 norm is within 1e-4 of 1.0
///
/// # Example
///
/// ```rust
/// use tldr_core::semantic::similarity::{normalize, is_normalized};
///
/// let unit = vec![0.6_f32, 0.8, 0.0]; // Already unit length
/// assert!(is_normalized(&unit));
///
/// let non_unit = vec![3.0_f32, 4.0, 0.0]; // Length 5
/// assert!(!is_normalized(&non_unit));
/// ```
pub fn is_normalized(v: &[f32]) -> bool {
    let norm_squared: f64 = v.iter().map(|x| (*x as f64).powi(2)).sum();
    let norm = norm_squared.sqrt();

    // Use slightly larger epsilon for is_normalized check
    // to account for accumulated floating point errors
    (norm - 1.0).abs() < 1e-4
}

/// Find top-K most similar items to a query
///
/// Returns indices and scores of the most similar candidates,
/// sorted by score in descending order.
///
/// # Arguments
///
/// * `query` - Query embedding vector (should be normalized for accurate results)
/// * `candidates` - List of (index, embedding) pairs to search
/// * `k` - Maximum number of results to return
/// * `threshold` - Minimum similarity score (0.0 to 1.0); results below this are filtered
///
/// # Returns
///
/// Vector of (index, score) pairs, sorted by score descending.
/// Returns at most `k` results, all with scores >= `threshold`.
///
/// # Performance
///
/// O(m * n + m log k) where m = candidates, n = dimensions
///
/// # Example
///
/// ```rust
/// use tldr_core::semantic::similarity::top_k_similar;
///
/// let query = vec![1.0_f32, 0.0];
/// let candidates: Vec<(usize, &[f32])> = vec![
///     (0, &[0.9_f32, 0.1][..]),
///     (1, &[0.1_f32, 0.9][..]),
///     (2, &[0.7_f32, 0.3][..]),
/// ];
///
/// let results = top_k_similar(&query, &candidates, 2, 0.0);
/// assert_eq!(results.len(), 2);
/// assert_eq!(results[0].0, 0); // Index 0 has highest similarity
/// ```
pub fn top_k_similar(
    query: &[f32],
    candidates: &[(usize, &[f32])],
    k: usize,
    threshold: f64,
) -> Vec<(usize, f64)> {
    // Handle empty candidates
    if candidates.is_empty() || k == 0 {
        return Vec::new();
    }

    // Compute similarity for all candidates
    let mut scored: Vec<(usize, f64)> = candidates
        .iter()
        .map(|(idx, embedding)| {
            let score = cosine_similarity(query, embedding);
            (*idx, score)
        })
        .filter(|(_, score)| *score >= threshold)
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top k
    scored.truncate(k);

    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // cosine_similarity tests
    // =========================================================================

    #[test]
    fn cosine_similarity_identical_vectors_equals_one() {
        // GIVEN: Two identical normalized vectors
        let v = vec![0.5_f32, 0.5, 0.5, 0.5];
        let mut normalized = v.clone();
        normalize(&mut normalized);

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&normalized, &normalized);

        // THEN: Similarity should be 1.0
        assert!((sim - 1.0).abs() < 1e-6, "Expected 1.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_equals_zero() {
        // GIVEN: Two orthogonal vectors
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![0.0_f32, 1.0, 0.0];

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&a, &b);

        // THEN: Similarity should be 0.0
        assert!(sim.abs() < 1e-6, "Expected 0.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_opposite_vectors_equals_negative_one() {
        // GIVEN: Two opposite vectors
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![-1.0_f32, 0.0, 0.0];

        // WHEN: We compute cosine similarity
        let sim = cosine_similarity(&a, &b);

        // THEN: Similarity should be -1.0
        assert!((sim - (-1.0)).abs() < 1e-6, "Expected -1.0, got {}", sim);
    }

    #[test]
    fn cosine_similarity_is_symmetric() {
        // GIVEN: Two random vectors
        let a = vec![0.3_f32, 0.7, 0.2, 0.5];
        let b = vec![0.6_f32, 0.1, 0.8, 0.3];

        // WHEN: We compute similarity both ways
        let sim_ab = cosine_similarity(&a, &b);
        let sim_ba = cosine_similarity(&b, &a);

        // THEN: Results should be identical (symmetric)
        assert!((sim_ab - sim_ba).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "Vectors must have same length")]
    fn cosine_similarity_different_lengths_panics() {
        // GIVEN: Vectors of different lengths
        let a = vec![1.0_f32, 0.0, 0.0];
        let b = vec![1.0_f32, 0.0];

        // WHEN: We compute cosine similarity
        // THEN: Should panic
        let _ = cosine_similarity(&a, &b);
    }

    #[test]
    fn cosine_similarity_zero_vectors_returns_zero() {
        // P0 Mitigation test: Zero vectors should return 0.0, not NaN
        let zero = vec![0.0_f32, 0.0, 0.0];
        let normal = vec![1.0_f32, 0.0, 0.0];

        // Zero vs zero
        let sim1 = cosine_similarity(&zero, &zero);
        assert!(
            sim1.abs() < 1e-6,
            "Zero vs zero should be 0.0, got {}",
            sim1
        );
        assert!(!sim1.is_nan(), "Should not return NaN for zero vectors");

        // Zero vs normal
        let sim2 = cosine_similarity(&zero, &normal);
        assert!(
            sim2.abs() < 1e-6,
            "Zero vs normal should be 0.0, got {}",
            sim2
        );
        assert!(!sim2.is_nan(), "Should not return NaN for zero vector");
    }

    // =========================================================================
    // normalize tests
    // =========================================================================

    #[test]
    fn normalize_creates_unit_vector() {
        // GIVEN: A non-normalized vector
        let mut v = vec![3.0_f32, 4.0, 0.0]; // Length 5

        // WHEN: We normalize it
        normalize(&mut v);

        // THEN: L2 norm should be 1.0
        let l2_norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((l2_norm - 1.0).abs() < 1e-6);
        assert!((v[0] - 0.6).abs() < 1e-6); // 3/5
        assert!((v[1] - 0.8).abs() < 1e-6); // 4/5
    }

    #[test]
    fn normalize_zero_vector_stays_zero() {
        // GIVEN: A zero vector
        let mut v = vec![0.0_f32, 0.0, 0.0];

        // WHEN: We normalize it
        normalize(&mut v);

        // THEN: Should stay all zeros (not produce NaN)
        for x in &v {
            assert!(x.abs() < 1e-6);
            assert!(!x.is_nan());
        }
    }

    #[test]
    fn normalize_already_normalized_stays_same() {
        // GIVEN: An already normalized vector
        let mut v = vec![0.6_f32, 0.8, 0.0];
        let original = v.clone();

        // WHEN: We normalize it
        normalize(&mut v);

        // THEN: Should stay the same
        for (a, b) in v.iter().zip(original.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    // =========================================================================
    // is_normalized tests
    // =========================================================================

    #[test]
    fn is_normalized_detects_unit_vectors() {
        // GIVEN: Normalized and non-normalized vectors
        let unit = vec![0.6_f32, 0.8, 0.0]; // Already unit length
        let non_unit = vec![3.0_f32, 4.0, 0.0]; // Length 5

        // THEN: is_normalized should correctly identify them
        assert!(is_normalized(&unit));
        assert!(!is_normalized(&non_unit));
    }

    #[test]
    fn is_normalized_false_for_zero_vector() {
        // GIVEN: A zero vector
        let zero = vec![0.0_f32, 0.0, 0.0];

        // THEN: Should not be considered normalized (norm is 0, not 1)
        assert!(!is_normalized(&zero));
    }

    // =========================================================================
    // top_k_similar tests
    // =========================================================================

    #[test]
    fn top_k_similar_returns_k_results() {
        // GIVEN: A query and candidate vectors
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.9_f32, 0.1][..]),
            (1, &[0.1_f32, 0.9][..]),
            (2, &[0.7_f32, 0.3][..]),
            (3, &[0.8_f32, 0.2][..]),
        ];

        // WHEN: We find top-2 similar
        let results = top_k_similar(&query, &candidates, 2, 0.0);

        // THEN: Should return exactly 2 results
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn top_k_similar_ordered_by_score_descending() {
        // GIVEN: A query and candidate vectors
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.9_f32, 0.1][..]), // highest
            (1, &[0.1_f32, 0.9][..]), // lowest
            (2, &[0.7_f32, 0.3][..]), // medium
        ];

        // WHEN: We find top-3 similar
        let results = top_k_similar(&query, &candidates, 3, 0.0);

        // THEN: Results should be ordered by score descending
        assert_eq!(results.len(), 3);
        assert!(results[0].1 >= results[1].1);
        assert!(results[1].1 >= results[2].1);
        assert_eq!(results[0].0, 0); // Index of highest similarity
    }

    #[test]
    fn top_k_similar_respects_threshold() {
        // GIVEN: A query and candidates with varying similarities
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.99_f32, 0.01][..]), // very high similarity
            (1, &[0.1_f32, 0.9][..]),   // low similarity
            (2, &[0.5_f32, 0.5][..]),   // medium similarity
        ];

        // WHEN: We search with high threshold (0.8)
        let results = top_k_similar(&query, &candidates, 10, 0.8);

        // THEN: Only results above threshold should be returned
        assert!(!results.is_empty());
        for (_, score) in &results {
            assert!(*score >= 0.8, "Score {} below threshold 0.8", score);
        }
    }

    #[test]
    fn top_k_similar_empty_candidates_returns_empty() {
        // GIVEN: A query and empty candidates
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![];

        // WHEN: We search
        let results = top_k_similar(&query, &candidates, 10, 0.0);

        // THEN: Should return empty
        assert!(results.is_empty());
    }

    #[test]
    fn top_k_similar_k_larger_than_candidates() {
        // GIVEN: 2 candidates but requesting top-10
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> =
            vec![(0, &[0.9_f32, 0.1][..]), (1, &[0.1_f32, 0.9][..])];

        // WHEN: We request top-10
        let results = top_k_similar(&query, &candidates, 10, 0.0);

        // THEN: Should return all available (2)
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn top_k_similar_k_zero_returns_empty() {
        // GIVEN: A query and candidates
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![(0, &[0.9_f32, 0.1][..])];

        // WHEN: We request top-0
        let results = top_k_similar(&query, &candidates, 0, 0.0);

        // THEN: Should return empty
        assert!(results.is_empty());
    }

    #[test]
    fn top_k_similar_all_below_threshold_returns_empty() {
        // GIVEN: A query and candidates all with low similarity
        let query = vec![1.0_f32, 0.0];
        let candidates: Vec<(usize, &[f32])> = vec![
            (0, &[0.0_f32, 1.0][..]),  // orthogonal
            (1, &[-1.0_f32, 0.0][..]), // opposite
        ];

        // WHEN: We search with positive threshold
        let results = top_k_similar(&query, &candidates, 10, 0.5);

        // THEN: Should return empty (all below threshold)
        assert!(results.is_empty());
    }
}
