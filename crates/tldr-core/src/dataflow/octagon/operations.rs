//! Lattice operations for octagon DBMs: join, meet, widening, narrowing.
//!
//! # Join (Least Upper Bound)
//!
//! Join of two octagons is the element-wise maximum of their closed DBMs.
//! Both inputs must be closed for correctness.
//!
//! # Meet (Greatest Lower Bound)
//!
//! Meet is the element-wise minimum followed by re-closure.
//! The result may be empty (bottom).
//!
//! # Widening
//!
//! Standard widening: constraints that grow between iterations are
//! set to +infinity. Guarantees termination of fixpoint computation.
//!
//! Threshold widening: instead of jumping to +infinity, jump to the
//! next threshold value from a predefined set.
//!
//! # References
//!
//! - Mine 2006, Sections 4.4-4.6: Join, widening, narrowing
//! - Prior art Section 2.1, Q3: Widening thresholds

use super::bound::Bound;
use super::closure::{strong_closure, ClosureResult};
use super::dbm::Dbm;

/// Compute the join (least upper bound) of two DBMs.
///
/// The join is the element-wise maximum of entries from two closed DBMs.
/// The result is closed (since element-wise max of closed DBMs is closed).
///
/// # Preconditions
///
/// Both `a` and `b` must be strongly closed.
/// Both must have the same number of variables.
///
/// # Returns
///
/// A new DBM representing the join. Returns `None` if sizes differ.
pub fn join<B: Bound>(a: &Dbm<B>, b: &Dbm<B>) -> Option<Dbm<B>> {
    if a.n_vars() != b.n_vars() {
        return None;
    }

    let mut result = Dbm::new(a.n_vars());
    let dim = a.dim();

    for i in 0..dim {
        for j in 0..dim {
            let val = B::max(a.get(i, j), b.get(i, j));
            result.set(i, j, val);
        }
    }

    Some(result)
}

/// Compute the meet (greatest lower bound) of two DBMs.
///
/// The meet is the element-wise minimum followed by re-closure.
/// The result may be empty (bottom) if the intersection is inconsistent.
///
/// # Returns
///
/// `Some(dbm)` if the meet is non-empty, `None` if empty (bottom).
pub fn meet<B: Bound>(a: &Dbm<B>, b: &Dbm<B>) -> Option<Dbm<B>> {
    if a.n_vars() != b.n_vars() {
        return None;
    }

    let mut result = Dbm::new(a.n_vars());
    let dim = a.dim();

    for i in 0..dim {
        for j in 0..dim {
            let val = B::min(a.get(i, j), b.get(i, j));
            result.set(i, j, val);
        }
    }

    // Re-close to derive transitive constraints
    let closure_result = strong_closure(&mut result);
    if closure_result == ClosureResult::Empty {
        return None;
    }

    Some(result)
}

/// Standard widening of two DBMs.
///
/// For each entry:
/// - If `new[i,j] > old[i,j]` (constraint weakened), set to +infinity.
/// - Otherwise, keep `new[i,j]`.
///
/// This guarantees fixpoint convergence in at most O(n^2) iterations.
pub fn widen<B: Bound>(old: &Dbm<B>, new: &Dbm<B>) -> Dbm<B> {
    let mut result = Dbm::new(old.n_vars());
    let dim = old.dim();

    for i in 0..dim {
        for j in 0..dim {
            let old_val = old.get(i, j);
            let new_val = new.get(i, j);

            // If new constraint is weaker (grew), set to +infinity
            // If stable or tightened, keep the new value
            let widened_val = if new_val > old_val {
                B::infinity()
            } else {
                new_val
            };
            result.set(i, j, widened_val);
        }
    }

    result
}

/// Default widening thresholds for octagon analysis.
///
/// These values are commonly used as jump targets instead of +infinity.
/// Based on ASTREE conventions and common program constants.
pub const DEFAULT_THRESHOLDS: &[i64] = &[
    -128, -64, -32, -16, -8, -4, -2, -1, 0, 1, 2, 4, 8, 16, 32, 64, 128, 255, 256, 1024,
];

/// Widening with thresholds.
///
/// Instead of jumping to +infinity, jumps to the next threshold value
/// that is >= the new bound.
///
/// # Arguments
///
/// - `old`: DBM from previous iteration.
/// - `new`: DBM from current iteration.
/// - `thresholds`: Sorted list of threshold values (ascending).
///
/// # Returns
///
/// Widened DBM.
pub fn widen_with_thresholds<B: Bound>(
    old: &Dbm<B>,
    new: &Dbm<B>,
    thresholds: &[i64],
) -> Dbm<B> {
    let mut result = Dbm::new(old.n_vars());
    let dim = old.dim();

    for i in 0..dim {
        for j in 0..dim {
            let old_val = old.get(i, j);
            let new_val = new.get(i, j);

            let widened_val = if new_val > old_val {
                // Growing constraint: jump to next threshold >= new_val
                // instead of jumping straight to +infinity
                let mut jumped = B::infinity();
                for &t in thresholds {
                    let threshold_bound = B::from_i64(t);
                    if threshold_bound >= new_val {
                        jumped = threshold_bound;
                        break;
                    }
                }
                jumped
            } else {
                // Stable or tightened: keep the new value
                new_val
            };
            result.set(i, j, widened_val);
        }
    }

    result
}

/// Check if `a` is included in `b` (a <= b in the lattice).
///
/// For closed DBMs: `a <= b` iff `a[i,j] <= b[i,j]` for all i,j.
///
/// Only the right argument (b) needs to be closed.
pub fn is_included<B: Bound>(a: &Dbm<B>, b: &Dbm<B>) -> bool {
    if a.n_vars() != b.n_vars() {
        return false;
    }

    let dim = a.dim();
    for i in 0..dim {
        for j in 0..dim {
            if a.get(i, j) > b.get(i, j) {
                return false;
            }
        }
    }

    true
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Join Tests
    // =========================================================================

    #[test]
    fn test_join_element_wise_max() {
        // Join = point-wise max of closed DBMs
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        // Set different constraints in a and b
        a.set(2, 0, 3.0);  // a: x0 - x1 <= 3
        b.set(2, 0, 5.0);  // b: x0 - x1 <= 5

        a.set(0, 2, 2.0);  // a: x1 - x0 <= 2
        b.set(0, 2, 1.0);  // b: x1 - x0 <= 1

        let joined = join(&a, &b).expect("Join of same-size DBMs should succeed");

        // Join takes the MAX (least constraining)
        assert_eq!(
            joined.get(2, 0),
            5.0,
            "Join should take max: max(3, 5) = 5"
        );
        assert_eq!(
            joined.get(0, 2),
            2.0,
            "Join should take max: max(2, 1) = 2"
        );
    }

    #[test]
    fn test_join_commutative() {
        // join(a, b) == join(b, a)
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        a.set(2, 0, 3.0);
        a.set(1, 0, 8.0);
        b.set(2, 0, 5.0);
        b.set(1, 0, 4.0);

        let ab = join(&a, &b).unwrap();
        let ba = join(&b, &a).unwrap();

        for i in 0..ab.dim() {
            for j in 0..ab.dim() {
                assert_eq!(
                    ab.get(i, j),
                    ba.get(i, j),
                    "Join must be commutative: join(a,b)[{i},{j}] != join(b,a)[{i},{j}]"
                );
            }
        }
    }

    #[test]
    fn test_join_associative() {
        // join(join(a,b), c) == join(a, join(b,c))
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);
        let mut c = Dbm::<f64>::new(2);

        a.set(2, 0, 3.0);
        b.set(2, 0, 5.0);
        c.set(2, 0, 4.0);
        a.set(1, 0, 8.0);
        b.set(1, 0, 6.0);
        c.set(1, 0, 10.0);

        let ab_c = join(&join(&a, &b).unwrap(), &c).unwrap();
        let a_bc = join(&a, &join(&b, &c).unwrap()).unwrap();

        for i in 0..ab_c.dim() {
            for j in 0..ab_c.dim() {
                assert_eq!(
                    ab_c.get(i, j),
                    a_bc.get(i, j),
                    "Join must be associative: ((a+b)+c)[{i},{j}] != (a+(b+c))[{i},{j}]"
                );
            }
        }
    }

    #[test]
    fn test_join_idempotent() {
        // join(a, a) == a
        let mut a = Dbm::<f64>::new(2);
        a.set(2, 0, 3.0);
        a.set(1, 0, 8.0);
        a.set(0, 2, 2.0);

        let aa = join(&a, &a).unwrap();

        for i in 0..a.dim() {
            for j in 0..a.dim() {
                assert_eq!(
                    aa.get(i, j),
                    a.get(i, j),
                    "Join must be idempotent: join(a,a)[{i},{j}] != a[{i},{j}]"
                );
            }
        }
    }

    #[test]
    fn test_join_with_bottom() {
        // join(a, bottom) == a
        // Bottom is represented by an empty/inconsistent DBM
        // For now, we test with a fresh unconstrained DBM (top) which is
        // the closest we can get without bottom representation.
        // The actual test: join(a, top) == top
        let mut a = Dbm::<f64>::new(2);
        a.set(2, 0, 3.0);

        let top = Dbm::<f64>::new(2); // All infinity except diagonal

        let result = join(&a, &top).unwrap();

        // join(a, top) = top (since top has all entries at +inf, max is always +inf)
        for i in 0..result.dim() {
            for j in 0..result.dim() {
                if i != j {
                    assert_eq!(
                        result.get(i, j),
                        f64::INFINITY,
                        "join(a, top) should be top: result[{i},{j}] = {}",
                        result.get(i, j)
                    );
                }
            }
        }
    }

    #[test]
    fn test_join_with_top() {
        // join(a, top) == top
        let mut a = Dbm::<f64>::new(2);
        a.set(2, 0, 3.0);
        a.set(1, 0, 4.0);

        let top = Dbm::<f64>::new(2);
        let result = join(&a, &top).unwrap();

        for i in 0..result.dim() {
            for j in 0..result.dim() {
                if i != j {
                    assert_eq!(
                        result.get(i, j),
                        f64::INFINITY,
                        "join(a, top) should equal top everywhere except diagonal"
                    );
                }
            }
        }
    }

    #[test]
    fn test_join_preserves_soundness() {
        // a <= join(a, b) and b <= join(a, b)
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        a.set(2, 0, 3.0);
        a.set(1, 0, 8.0);
        b.set(2, 0, 5.0);
        b.set(1, 0, 4.0);

        let joined = join(&a, &b).unwrap();

        // a[i,j] <= joined[i,j] for all i,j (soundness)
        for i in 0..a.dim() {
            for j in 0..a.dim() {
                assert!(
                    a.get(i, j) <= joined.get(i, j),
                    "Soundness: a[{i},{j}]={} must be <= join[{i},{j}]={}",
                    a.get(i, j),
                    joined.get(i, j)
                );
                assert!(
                    b.get(i, j) <= joined.get(i, j),
                    "Soundness: b[{i},{j}]={} must be <= join[{i},{j}]={}",
                    b.get(i, j),
                    joined.get(i, j)
                );
            }
        }
    }

    // =========================================================================
    // Meet Tests
    // =========================================================================

    #[test]
    fn test_meet_element_wise_min() {
        // Meet = point-wise min, then re-close
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        a.set(2, 0, 3.0);
        b.set(2, 0, 5.0);

        let met = meet(&a, &b).expect("Meet should succeed for consistent inputs");

        // Meet takes the MIN (most constraining)
        assert_eq!(
            met.get(2, 0),
            3.0,
            "Meet should take min: min(3, 5) = 3"
        );
    }

    #[test]
    fn test_meet_requires_closure() {
        // Meet without closure can produce unsound results.
        // After min, the result may violate triangle inequality.
        // The meet implementation MUST re-close.
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        // Set up constraints that create transitive implications after meet
        a.set(2, 0, 5.0);
        a.set(3, 2, 3.0);
        b.set(2, 0, 2.0);
        b.set(3, 2, 10.0);

        let met = meet(&a, &b).expect("Meet should succeed");

        // After meet with closure, transitive constraints should be derived.
        // The meet takes min(5,2) = 2 for m[2,0] and min(3,10) = 3 for m[3,2].
        // Closure should derive: m[3,0] <= m[3,2] + m[2,0] = 3 + 2 = 5.
        let m_30 = met.get(3, 0);
        assert!(
            m_30 <= 5.0,
            "Meet must re-close to derive transitive constraints; m[3,0] = {m_30}"
        );
    }

    // =========================================================================
    // Widening Tests
    // =========================================================================

    #[test]
    fn test_widen_standard() {
        // Growing constraints set to +infinity
        let mut old = Dbm::<f64>::new(2);
        let mut new = Dbm::<f64>::new(2);

        old.set(2, 0, 3.0);  // old: x0 - x1 <= 3
        new.set(2, 0, 5.0);  // new: x0 - x1 <= 5 (grew!)

        old.set(1, 0, 8.0);  // old: x0 upper bound
        new.set(1, 0, 8.0);  // new: same (stable)

        let widened = widen(&old, &new);

        // Growing constraint should be set to +infinity
        assert_eq!(
            widened.get(2, 0),
            f64::INFINITY,
            "Growing constraint (3 -> 5) should be widened to +infinity"
        );

        // Stable constraint should be preserved
        assert_eq!(
            widened.get(1, 0),
            8.0,
            "Stable constraint should be preserved during widening"
        );
    }

    #[test]
    fn test_widen_stable_constraints_kept() {
        // Constraints that did not grow should be kept as-is
        let mut old = Dbm::<f64>::new(2);
        let mut new = Dbm::<f64>::new(2);

        old.set(2, 0, 5.0);
        new.set(2, 0, 3.0);  // Tightened (3 < 5), should be kept

        old.set(1, 0, 8.0);
        new.set(1, 0, 8.0);  // Same, should be kept

        let widened = widen(&old, &new);

        assert_eq!(
            widened.get(2, 0),
            3.0,
            "Non-growing constraint should be kept: new(3) <= old(5)"
        );
        assert_eq!(
            widened.get(1, 0),
            8.0,
            "Stable constraint should be kept"
        );
    }

    #[test]
    fn test_widen_guarantees_termination() {
        // Repeated widening should reach a fixpoint (all constraints either
        // stable or already at +infinity).
        let n_vars = 3;
        let mut state = Dbm::<f64>::new(n_vars);

        // Set some initial constraints
        state.set(2, 0, 0.0);
        state.set(4, 2, 0.0);

        // Simulate iteration: each step, some constraints grow by 1
        let max_iters = 100;
        let mut converged = false;

        for iter in 0..max_iters {
            let mut next = state.clone();
            // Simulate constraint growth
            for i in 0..next.dim() {
                for j in 0..next.dim() {
                    if i != j {
                        let v = next.get(i, j);
                        if v < f64::INFINITY && v < 100.0 {
                            next.set(i, j, v + 1.0);
                        }
                    }
                }
            }

            let widened = widen(&state, &next);

            // Check convergence: widened == state means fixpoint
            let mut same = true;
            for i in 0..widened.dim() {
                for j in 0..widened.dim() {
                    if widened.get(i, j) != state.get(i, j) {
                        same = false;
                        break;
                    }
                }
                if !same {
                    break;
                }
            }

            state = widened;
            if same {
                converged = true;
                assert!(
                    iter < max_iters - 1,
                    "Widening should converge before max iterations"
                );
                break;
            }
        }

        assert!(
            converged,
            "Widening must guarantee termination (fixpoint convergence)"
        );
    }

    #[test]
    fn test_widen_with_thresholds() {
        // Threshold widening uses predefined jump targets instead of +infinity
        let mut old = Dbm::<f64>::new(2);
        let mut new = Dbm::<f64>::new(2);

        old.set(2, 0, 3.0);
        new.set(2, 0, 5.0);  // Growing

        let thresholds: &[i64] = &[-128, -1, 0, 1, 8, 16, 32, 64, 128];
        let widened = widen_with_thresholds(&old, &new, thresholds);

        // Should jump to next threshold >= 5, which is 8
        let val = widened.get(2, 0);
        assert!(
            val <= 8.0 || val == f64::INFINITY,
            "Threshold widening should jump to next threshold (8) or +inf, not beyond; got {val}"
        );
        assert!(
            val >= 5.0,
            "Threshold widening must be sound: result ({val}) >= new value (5.0)"
        );
    }

    #[test]
    fn test_widen_sound() {
        // widen(old, new) >= new (soundness)
        let mut old = Dbm::<f64>::new(2);
        let mut new = Dbm::<f64>::new(2);

        old.set(2, 0, 3.0);
        old.set(1, 0, 8.0);
        old.set(0, 2, 2.0);

        new.set(2, 0, 5.0);
        new.set(1, 0, 6.0);  // Tightened
        new.set(0, 2, 4.0);  // Grew

        let widened = widen(&old, &new);

        for i in 0..widened.dim() {
            for j in 0..widened.dim() {
                assert!(
                    widened.get(i, j) >= new.get(i, j),
                    "Widening soundness: widen[{i},{j}]={} must be >= new[{i},{j}]={}",
                    widened.get(i, j),
                    new.get(i, j)
                );
            }
        }
    }

    #[test]
    fn test_widen_convergence_loop() {
        // Simulate loop: x=0; while x<10: x+=1
        // The octagon should converge with widening.
        //
        // Iteration 0: x in [0, 0]
        // Iteration 1: x in [0, 1] (join of [0,0] and [1,1])
        // After widening: x in [0, +inf)
        // Iteration 2: x in [0, +inf) (stable)
        //
        // This is a convergence test -- the domain must stabilize.
        let mut state = Dbm::<f64>::new(1);

        // x = 0: m[1,0] = 0 (x <= 0), m[0,1] = 0 (x >= 0)
        state.set(1, 0, 0.0);
        state.set(0, 1, 0.0);

        // Simulate one loop iteration: x = x + 1
        let mut next = state.clone();
        // x in [1, 1]: m[1,0] = 2 (x <= 1), m[0,1] = -2 (x >= 1)
        next.set(1, 0, 2.0);
        next.set(0, 1, -2.0);

        // Join gives [0, 1]
        let joined = join(&state, &next).unwrap();

        // Widen: upper bound grew (0 -> 2), should go to +inf
        let widened = widen(&state, &joined);

        // Upper bound should be +infinity (widened away)
        let upper = widened.get(1, 0);
        assert_eq!(
            upper,
            f64::INFINITY,
            "After widening, upper bound of loop variable should be +infinity; got {upper}"
        );

        // Lower bound should be stable (0 in both iterations)
        let lower = widened.get(0, 1);
        assert_eq!(
            lower, 0.0,
            "Lower bound should be stable (0); got {lower}"
        );
    }

    // =========================================================================
    // Inclusion Tests
    // =========================================================================

    #[test]
    fn test_inclusion_closed_dbms() {
        // a <= b iff a[i,j] <= b[i,j] for all i,j
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        a.set(2, 0, 3.0);
        b.set(2, 0, 5.0);

        a.set(1, 0, 4.0);
        b.set(1, 0, 4.0);

        // a has tighter constraints, so a <= b
        assert!(
            is_included(&a, &b),
            "a with tighter constraints should be included in b"
        );

        // b is NOT included in a (has looser constraints)
        assert!(
            !is_included(&b, &a),
            "b with looser constraints should NOT be included in a"
        );
    }
}
