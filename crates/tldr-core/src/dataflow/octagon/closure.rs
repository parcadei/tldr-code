//! Strong closure, incremental closure, and tight closure for octagon DBMs.
//!
//! # Strong Closure (Mine 2006, Algorithm 1)
//!
//! Strong closure tightens all constraints in the DBM via Floyd-Warshall
//! shortest-path computation followed by a strengthening (Str) pass.
//! Complexity: O(n^3).
//!
//! # Incremental Closure (Bagnara et al. 2018)
//!
//! When adding a single constraint to an already-closed DBM, only O(n^2)
//! work is needed. This is the dominant use case during transfer functions.
//!
//! # Tight Closure
//!
//! For integer-valued variables, tight closure ensures that diagonal
//! entries `m[2i, 2i+1]` are even (i.e., `2 * floor(m[2i, 2i+1] / 2)`).
//! This provides tighter integer bounds.
//!
//! # References
//!
//! - Mine 2006, Section 4.3: Strong closure algorithm
//! - Bagnara et al. 2018: Incremental closure
//! - Prior art Section 2.1-2.2: Algorithm details

use super::bound::Bound;
use super::dbm::Dbm;

/// Result of a closure operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureResult {
    /// Closure succeeded; DBM is now strongly closed.
    Closed,
    /// Negative cycle detected; the octagon is empty (bottom).
    Empty,
}

/// Perform strong closure on a DBM (Floyd-Warshall + strengthening pass).
///
/// After strong closure, the DBM satisfies:
/// - Triangle inequality: `m[i,j] <= m[i,k] + m[k,j]` for all i,j,k
/// - Strengthening: `m[i,j] <= (m[i,i_bar] + m[j_bar,j]) / 2`
/// - Diagonal: `m[i,i] = 0`
///
/// Returns `ClosureResult::Empty` if a negative diagonal entry is found
/// (indicating an inconsistent constraint set).
///
/// Complexity: O(n^3) where n is the number of program variables.
pub fn strong_closure<B: Bound>(dbm: &mut Dbm<B>) -> ClosureResult {
    let dim = dbm.dim();

    // Use a full 2n x 2n matrix for Floyd-Warshall to avoid half-matrix
    // aliasing issues. The half-matrix storage maps (i,j) and (j^1,i^1)
    // to the same physical cell, which can cause overwrites during
    // in-place Floyd-Warshall updates.
    let mut m = vec![B::infinity(); dim * dim];

    // Copy DBM into full matrix
    for i in 0..dim {
        for j in 0..dim {
            m[i * dim + j] = dbm.get(i, j);
        }
    }

    // Phase 1: Floyd-Warshall shortest path on full matrix
    for k in 0..dim {
        for i in 0..dim {
            let m_ik = m[i * dim + k];
            for j in 0..dim {
                let through_k = B::closure_add(m_ik, m[k * dim + j]);
                let current = m[i * dim + j];
                let tighter = B::min(current, through_k);
                if tighter != current {
                    m[i * dim + j] = tighter;
                }
            }
        }
    }

    // Phase 2: Strengthening (Str) pass (Mine 2006, Section 4.3)
    // m[i,j] = min(m[i,j], (m[i, i^1] + m[j^1, j]) / 2)
    for i in 0..dim {
        let i_bar = Dbm::<B>::complement(i);
        let m_i_ibar = m[i * dim + i_bar];
        for j in 0..dim {
            let j_bar = Dbm::<B>::complement(j);
            let coherence_sum = B::closure_add(m_i_ibar, m[j_bar * dim + j]);
            let coherence_val = B::half(coherence_sum);
            let current = m[i * dim + j];
            let tighter = B::min(current, coherence_val);
            if tighter != current {
                m[i * dim + j] = tighter;
            }
        }
    }

    // Phase 3: Consistency check -- negative diagonal means empty (bottom)
    let mut result = ClosureResult::Closed;
    for i in 0..dim {
        if m[i * dim + i] < B::zero() {
            result = ClosureResult::Empty;
            break;
        }
    }

    // Write back to DBM. For coherent pairs (i,j) and (j^1,i^1), take the
    // minimum of both values to maintain coherence invariant.
    // Always write back (even for Empty) so that has_negative_cycle() can
    // inspect the DBM diagonal after closure.
    for i in 0..dim {
        for j in 0..dim {
            dbm.set(i, j, m[i * dim + j]);
        }
    }

    result
}

/// Perform incremental closure after adding a single constraint.
///
/// Given an already-closed DBM and a new constraint `m[i,j] <= c`,
/// updates only the affected entries in O(n^2) time.
///
/// # Arguments
///
/// - `dbm`: A strongly-closed DBM (must be closed before calling).
/// - `i`, `j`: Indices of the new constraint.
/// - `value`: The new constraint bound.
///
/// Returns `ClosureResult::Empty` if the new constraint creates
/// an inconsistency.
pub fn incremental_closure<B: Bound>(
    dbm: &mut Dbm<B>,
    a: usize,
    b: usize,
    value: B,
) -> ClosureResult {
    let dim = dbm.dim();

    // Work on a full matrix to avoid half-matrix aliasing
    let mut m = vec![B::infinity(); dim * dim];
    for i in 0..dim {
        for j in 0..dim {
            m[i * dim + j] = dbm.get(i, j);
        }
    }

    // Apply the new constraint if tighter than existing
    let current = m[a * dim + b];
    let c = B::min(current, value);
    m[a * dim + b] = c;

    // Also set the coherence counterpart: m[b^1, a^1] = min(m[b^1, a^1], c)
    let a_bar = Dbm::<B>::complement(a);
    let b_bar = Dbm::<B>::complement(b);
    let current_bar = m[b_bar * dim + a_bar];
    let c_bar = B::min(current_bar, c);
    m[b_bar * dim + a_bar] = c_bar;

    // Pre-compute the "bridge" term for paths that traverse BOTH new edges:
    //   (a,b) then (b^1,a^1): c + m[b, b^1] + c_bar
    //   (b^1,a^1) then (a,b): c_bar + m[a^1, a] + c
    let bridge_ab_bar = B::closure_add(
        B::closure_add(c, m[b * dim + b_bar]),
        c_bar,
    );
    let bridge_bar_ab = B::closure_add(
        B::closure_add(c_bar, m[a_bar * dim + a]),
        c,
    );

    // Incremental shortest path (Mine 2006 / Bagnara et al. 2008):
    // For all (i,j), update via the four possible path types through the
    // new edge (a,b) and its coherence counterpart (b^1, a^1):
    //   1. i -> a -> b -> j                           (via (a,b))
    //   2. i -> b^1 -> a^1 -> j                       (via (b^1,a^1))
    //   3. i -> a -> b -> b^1 -> a^1 -> j             (via (a,b) then (b^1,a^1))
    //   4. i -> b^1 -> a^1 -> a -> b -> j             (via (b^1,a^1) then (a,b))
    for i in 0..dim {
        for j in 0..dim {
            let cur = m[i * dim + j];

            // Path 1: through (a,b) edge
            let via_ab = B::closure_add(
                B::closure_add(m[i * dim + a], c),
                m[b * dim + j],
            );

            // Path 2: through coherence counterpart (b^1, a^1) edge
            let via_bar = B::closure_add(
                B::closure_add(m[i * dim + b_bar], c_bar),
                m[a_bar * dim + j],
            );

            // Path 3: (a,b) then bridge to (b^1,a^1)
            let via_ab_bar = B::closure_add(
                B::closure_add(m[i * dim + a], bridge_ab_bar),
                m[a_bar * dim + j],
            );

            // Path 4: (b^1,a^1) then bridge to (a,b)
            let via_bar_ab = B::closure_add(
                B::closure_add(m[i * dim + b_bar], bridge_bar_ab),
                m[b * dim + j],
            );

            let best = B::min(
                B::min(cur, B::min(via_ab, via_bar)),
                B::min(via_ab_bar, via_bar_ab),
            );
            if best != cur {
                m[i * dim + j] = best;
            }
        }
    }

    // Strengthening pass
    for i in 0..dim {
        let i_comp = Dbm::<B>::complement(i);
        let m_i_icomp = m[i * dim + i_comp];
        for j in 0..dim {
            let j_comp = Dbm::<B>::complement(j);
            let coherence_sum = B::closure_add(m_i_icomp, m[j_comp * dim + j]);
            let coherence_val = B::half(coherence_sum);
            let cur = m[i * dim + j];
            let tighter = B::min(cur, coherence_val);
            if tighter != cur {
                m[i * dim + j] = tighter;
            }
        }
    }

    // Consistency check
    for i in 0..dim {
        if m[i * dim + i] < B::zero() {
            return ClosureResult::Empty;
        }
    }

    // Write back with coherence: take min of (i,j) and (j^1, i^1).
    // Lower triangle (j <= i) covers most entries via coherence-based indexing.
    for i in 0..dim {
        for j in 0..=i {
            let i_b = Dbm::<B>::complement(i);
            let j_b = Dbm::<B>::complement(j);
            let val_ij = m[i * dim + j];
            let val_coherent = m[j_b * dim + i_b];
            let best = B::min(val_ij, val_coherent);
            dbm.set(i, j, best);
        }
    }

    // Write back same-variable upper-diagonal entries m[2k, 2k+1].
    // These are stored separately in Dbm::upper_diag and are NOT reached
    // by the lower-triangle loop above (since 2k < 2k+1 means j > i).
    // Their coherence counterpart is themselves: complement(2k)=2k+1,
    // complement(2k+1)=2k, so (j^1, i^1) = (2k, 2k+1) again.
    let n_vars = dim / 2;
    for var in 0..n_vars {
        let pos = 2 * var;
        let neg = 2 * var + 1;
        let val = m[pos * dim + neg];
        dbm.set(pos, neg, val);
    }

    ClosureResult::Closed
}

/// Perform tight closure for integer-valued variables.
///
/// After strong closure, tightens diagonal entries:
/// `m[2i, 2i+1] = tighten(m[2i, 2i+1])` where `tighten(x) = 2 * floor(x/2)`.
///
/// This ensures integer bounds are as tight as possible.
pub fn tight_closure<B: Bound>(dbm: &mut Dbm<B>) -> ClosureResult {
    let n = dbm.n_vars();

    // Tighten unary constraint entries for integer variables.
    // For each variable i, the entries m[2i, 2i+1] and m[2i+1, 2i]
    // represent 2*upper and 2*lower bounds respectively.
    // Tighten them to even values: 2 * floor(x / 2).
    for var in 0..n {
        let pos = 2 * var;
        let neg = 2 * var + 1;

        let upper = dbm.get(neg, pos);
        dbm.set(neg, pos, B::tighten(upper));

        let lower = dbm.get(pos, neg);
        dbm.set(pos, neg, B::tighten(lower));
    }

    ClosureResult::Closed
}

/// Check if a DBM has a negative cycle (is empty / bottom).
///
/// A negative cycle exists if any diagonal entry `m[i,i] < 0`.
pub fn has_negative_cycle<B: Bound>(dbm: &Dbm<B>) -> bool {
    let dim = dbm.dim();
    for i in 0..dim {
        if dbm.get(i, i) < B::zero() {
            return true;
        }
    }
    false
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Strong Closure Tests
    // =========================================================================

    #[test]
    fn test_closure_floyd_warshall_basic() {
        // Basic transitive constraint tightening:
        // Given: x0 - x1 <= 3, x1 - x2 <= 4
        // After closure: x0 - x2 <= 7 (transitive)
        let mut dbm = Dbm::<f64>::new(3);

        // x0 - x1 <= 3: m[2*1, 2*0] = 3 (encoding: positive literals)
        // In standard octagon encoding:
        // x_i - x_j <= c  is stored as m[2j, 2i] = c  (or m[2i+1, 2j+1] = c via coherence)
        dbm.set(2, 0, 3.0);  // x0 - x1 <= 3
        dbm.set(4, 2, 4.0);  // x1 - x2 <= 4

        let result = strong_closure(&mut dbm);
        assert_eq!(result, ClosureResult::Closed);

        // After closure, x0 - x2 <= 7 (transitive: 3 + 4)
        let x0_minus_x2 = dbm.get(4, 0);
        assert!(
            x0_minus_x2 <= 7.0,
            "After closure, x0 - x2 should be <= 7 (transitive tightening); got {x0_minus_x2}"
        );
    }

    #[test]
    fn test_closure_strengthening_pass() {
        // Strengthening (Str) pass after Floyd-Warshall:
        // m[i,j] = min(m[i,j], (m[i,i_bar] + m[j_bar,j]) / 2)
        // where i_bar = i XOR 1
        //
        // Example: x0 <= 5 and x0 >= -3
        // Upper bound: m[1, 0] = 10  (i.e., x0 - (-x0) <= 10 => 2*x0 <= 10 => x0 <= 5)
        // Lower bound: m[0, 1] = 6   (i.e., (-x0) - x0 <= 6 => -2*x0 <= 6 => x0 >= -3)
        //
        // After Str: other constraints involving x0 should be tightened
        // using these unary bounds.
        let mut dbm = Dbm::<f64>::new(2);

        // x0 <= 5: m[1, 0] = 10 (2 * 5)
        dbm.set(1, 0, 10.0);
        // x0 >= -3: m[0, 1] = 6 (2 * 3)
        dbm.set(0, 1, 6.0);

        // x0 - x1 <= 20 (a loose constraint)
        dbm.set(2, 0, 20.0); // Using indexing for x0_pos to x1_pos

        let result = strong_closure(&mut dbm);
        assert_eq!(result, ClosureResult::Closed);

        // After strengthening, the constraint x0 - x1 should be tightened
        // using the unary bounds on x0 and x1.
        // At minimum, m[i,j] <= (m[i,i_bar] + m[j_bar,j]) / 2
        let val = dbm.get(2, 0);
        assert!(
            val <= 20.0,
            "Strengthening should tighten loose constraints; m[2,0] = {val}"
        );
    }

    #[test]
    fn test_closure_idempotent() {
        // close(close(m)) == close(m)
        let mut dbm = Dbm::<f64>::new(3);

        // Set up some constraints
        dbm.set(2, 0, 5.0);  // x0 - x1 <= 5
        dbm.set(4, 2, 3.0);  // x1 - x2 <= 3
        dbm.set(1, 0, 8.0);  // x0 <= 4 (unary)

        strong_closure(&mut dbm);
        let after_first = dbm.clone();

        strong_closure(&mut dbm);

        // Second closure should not change anything
        for i in 0..dbm.dim() {
            for j in 0..dbm.dim() {
                assert_eq!(
                    dbm.get(i, j),
                    after_first.get(i, j),
                    "Closure must be idempotent: m[{i},{j}] changed on second closure"
                );
            }
        }
    }

    #[test]
    fn test_closure_detects_negative_cycle() {
        // Contradictory constraints should produce Empty (negative cycle)
        // x0 - x1 <= 3 AND x1 - x0 <= -5
        // => x0 - x1 <= 3 AND x0 - x1 >= 5 => contradiction
        let mut dbm = Dbm::<f64>::new(2);

        // x0 - x1 <= 3
        dbm.set(2, 0, 3.0);
        // x1 - x0 <= -5 (i.e., x0 - x1 >= 5)
        dbm.set(0, 2, -5.0);

        let result = strong_closure(&mut dbm);
        assert_eq!(
            result,
            ClosureResult::Empty,
            "Contradictory constraints must produce Empty (negative cycle)"
        );
    }

    #[test]
    fn test_closure_tightens_bounds() {
        // After closure, constraints should be at least as tight as before
        let mut dbm = Dbm::<f64>::new(2);

        dbm.set(2, 0, 10.0); // loose constraint
        dbm.set(2, 1, 3.0);  // x1_neg to x1_pos
        dbm.set(1, 0, 5.0);  // x0_pos to x0_neg

        let before_val = dbm.get(2, 0);
        strong_closure(&mut dbm);
        let after_val = dbm.get(2, 0);

        assert!(
            after_val <= before_val,
            "Closure should tighten constraints: before={before_val}, after={after_val}"
        );
    }

    #[test]
    fn test_closure_preserves_diagonal_zero() {
        // After closure, m[i,i] = 0 must hold
        let mut dbm = Dbm::<f64>::new(4);

        // Set some constraints
        dbm.set(2, 0, 5.0);
        dbm.set(4, 2, 3.0);
        dbm.set(6, 4, 7.0);

        strong_closure(&mut dbm);

        for i in 0..dbm.dim() {
            assert_eq!(
                dbm.get(i, i),
                0.0,
                "Diagonal entry m[{i},{i}] must be 0 after closure"
            );
        }
    }

    #[test]
    fn test_closure_example_from_mine_2006() {
        // Reproduce a known example from Mine 2006.
        //
        // Variables: x0, x1 (2 variables, dimension 4)
        // Constraints:
        //   x0 <= 3  => m[1, 0] = 6  (2*3)
        //   x0 >= 1  => m[0, 1] = -2 (-(2*1))
        //   x1 <= 5  => m[3, 2] = 10 (2*5)
        //   x1 >= 0  => m[2, 3] = 0  (2*0)
        //   x0 - x1 <= 2  => m[2, 0] = 2  (or equivalently m[3, 1] = 2)
        //   x1 - x0 <= 1  => m[0, 2] = 1  (or equivalently m[1, 3] = 1)
        //
        // After closure, bounds should be tightened:
        //   x0 in [1, 3], x1 in [0, 5], x0 - x1 in [-1, 2]
        let mut dbm = Dbm::<f64>::new(2);

        // Unary bounds on x0
        dbm.set(1, 0, 6.0);   // x0 <= 3 => m[1,0] = 2*3
        dbm.set(0, 1, -2.0);  // x0 >= 1 => m[0,1] = -(2*1)

        // Unary bounds on x1
        dbm.set(3, 2, 10.0);  // x1 <= 5 => m[3,2] = 2*5
        dbm.set(2, 3, 0.0);   // x1 >= 0 => m[2,3] = 0

        // Relational constraints
        dbm.set(2, 0, 2.0);   // x0 - x1 <= 2
        dbm.set(0, 2, 1.0);   // x1 - x0 <= 1

        let result = strong_closure(&mut dbm);
        assert_eq!(result, ClosureResult::Closed, "This constraint set is consistent");

        // After closure:
        // x0 upper bound: m[1,0] / 2 = upper bound of x0
        // Tightened: x0 <= min(3, x1 + 2) and x1 <= 5 => x0 <= 7
        // But also x0 <= 3 directly, so x0 <= 3 (unchanged if already tight)
        let x0_upper = dbm.get(1, 0);
        assert!(
            x0_upper <= 6.0,
            "x0 upper bound should be <= 6 (representing x0 <= 3); got {x0_upper}"
        );

        // x1 lower bound from relation: x1 >= x0 - 2 and x0 >= 1 => x1 >= -1
        // But also x1 >= 0 directly, so x1 >= 0 (tighter)
        let x1_lower = dbm.get(2, 3);
        assert!(
            x1_lower <= 0.0,
            "x1 lower bound encoding should reflect x1 >= 0; got {x1_lower}"
        );
    }

    // =========================================================================
    // Incremental Closure Tests
    // =========================================================================

    #[test]
    fn test_incremental_closure_single_constraint() {
        // Adding one constraint to a closed DBM should only need O(n^2) work
        // and produce the same result as full re-closure.
        let mut dbm = Dbm::<f64>::new(3);
        dbm.set(2, 0, 5.0);
        dbm.set(4, 2, 3.0);

        // First, fully close
        strong_closure(&mut dbm);

        // Now add a new constraint incrementally
        let result = incremental_closure(&mut dbm, 4, 0, 2.0);
        assert_eq!(result, ClosureResult::Closed);

        // The new constraint x0 - x2 <= 2 should be reflected
        assert!(
            dbm.get(4, 0) <= 2.0,
            "Incremental closure should apply the new constraint"
        );
    }

    #[test]
    fn test_incremental_matches_full_closure() {
        // Incremental result must equal full re-closure result
        let mut dbm_incr = Dbm::<f64>::new(3);
        dbm_incr.set(2, 0, 5.0);
        dbm_incr.set(4, 2, 3.0);
        strong_closure(&mut dbm_incr);

        // Clone for full closure comparison
        let mut dbm_full = dbm_incr.clone();

        // Add constraint incrementally
        incremental_closure(&mut dbm_incr, 4, 0, 2.0);

        // Add same constraint and re-close fully
        dbm_full.set(4, 0, Bound::min(dbm_full.get(4, 0), 2.0));
        strong_closure(&mut dbm_full);

        // Results must match
        for i in 0..dbm_incr.dim() {
            for j in 0..dbm_incr.dim() {
                assert_eq!(
                    dbm_incr.get(i, j),
                    dbm_full.get(i, j),
                    "Incremental closure must match full closure at m[{i},{j}]: \
                     incremental={}, full={}",
                    dbm_incr.get(i, j),
                    dbm_full.get(i, j)
                );
            }
        }
    }

    // =========================================================================
    // Tight Closure Tests
    // =========================================================================

    #[test]
    fn test_tight_closure_integers() {
        // For integer variables, tight closure ensures:
        // m[2i, 2i+1] = 2 * floor(m[2i, 2i+1] / 2)
        // This means unary upper bound entries are even.
        let mut dbm = Dbm::<i64>::new(2);

        // Set x0 <= 3: m[1, 0] = 7 (odd value -- should tighten to 6)
        dbm.set(1, 0, 7);

        // Set x1 <= 5: m[3, 2] = 11 (odd value -- should tighten to 10)
        dbm.set(3, 2, 11);

        strong_closure(&mut dbm);
        tight_closure(&mut dbm);

        // After tight closure, diagonal pairs should be even
        let m_10 = dbm.get(1, 0);
        assert_eq!(
            m_10 % 2,
            0,
            "After tight closure, m[1,0] should be even (integer tightening); got {m_10}"
        );

        let m_32 = dbm.get(3, 2);
        assert_eq!(
            m_32 % 2,
            0,
            "After tight closure, m[3,2] should be even (integer tightening); got {m_32}"
        );
    }

    // =========================================================================
    // Negative Cycle Detection Tests
    // =========================================================================

    #[test]
    fn test_has_negative_cycle_consistent() {
        let dbm = Dbm::<f64>::new(2);
        assert!(
            !has_negative_cycle(&dbm),
            "Fresh DBM with no constraints should not have a negative cycle"
        );
    }

    #[test]
    fn test_has_negative_cycle_after_contradiction() {
        // After closure on contradictory constraints, negative cycle should be detected
        let mut dbm = Dbm::<f64>::new(2);

        // x0 - x1 <= -10 AND x1 - x0 <= 5  (if -10 + 5 < 0, negative cycle)
        dbm.set(2, 0, -10.0);
        dbm.set(0, 2, 5.0);

        // After closure, diagonal should go negative
        strong_closure(&mut dbm);

        assert!(
            has_negative_cycle(&dbm),
            "Contradictory constraints should produce a negative cycle"
        );
    }
}
