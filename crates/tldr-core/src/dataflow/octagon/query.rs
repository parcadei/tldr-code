//! Query and projection operations for octagon DBMs.
//!
//! This module provides operations to extract information from a closed DBM:
//!
//! - **Interval projection**: Extract `[lo, hi]` for a single variable.
//! - **Bottom/Top detection**: Check if the octagon is empty or unconstrained.
//! - **Inclusion testing**: Check if one octagon is contained in another.
//! - **Constraint extraction**: Extract all octagonal constraints as a list.
//!
//! # References
//!
//! - Mine 2006, Section 4.2: Interval extraction from DBM
//! - Prior art Section 7: query.rs module specification

use super::bound::Bound;
use super::dbm::Dbm;

/// Extract the interval `[lo, hi]` for a variable from the DBM.
///
/// For variable `var_idx`:
/// - Upper bound: `m[2i+1, 2i] / 2`
/// - Lower bound: `-m[2i, 2i+1] / 2`
///
/// Returns `(Option<lo>, Option<hi>)` where `None` means unbounded.
pub fn project_interval<B: Bound>(dbm: &Dbm<B>, var_idx: usize) -> (Option<f64>, Option<f64>) {
    let pos = 2 * var_idx;       // positive literal index
    let neg = 2 * var_idx + 1;   // negative literal index

    // Upper bound: m[2v+1, 2v] / 2
    let upper_raw = dbm.get(neg, pos);
    let hi = if upper_raw.is_pos_infinity() {
        None
    } else {
        Some(bound_to_f64(upper_raw.half()))
    };

    // Lower bound: -m[2v, 2v+1] / 2
    let lower_raw = dbm.get(pos, neg);
    let lo = if lower_raw.is_pos_infinity() {
        None
    } else {
        let h = lower_raw.neg().half();
        Some(bound_to_f64(h))
    };

    (lo, hi)
}

/// Convert a Bound value to f64 for interval projection and constraint extraction.
///
/// Uses `std::any::Any` downcast to handle both `f64` (identity) and `i64`
/// (cast) bound types at runtime. The `Bound: 'static` constraint enables this.
fn bound_to_f64<B: Bound>(val: B) -> f64 {
    use std::any::Any;
    let any_val: &dyn Any = &val;
    if let Some(&f) = any_val.downcast_ref::<f64>() {
        f
    } else if let Some(&i) = any_val.downcast_ref::<i64>() {
        i as f64
    } else {
        // Unreachable for the two current Bound impls (f64, i64).
        0.0
    }
}

/// Check if the DBM represents the empty set (bottom).
///
/// A DBM is bottom if it has a negative diagonal entry (negative cycle).
pub fn is_bottom<B: Bound>(dbm: &Dbm<B>) -> bool {
    let dim = dbm.dim();
    for i in 0..dim {
        let diag = dbm.get(i, i);
        // Negative diagonal entry means a negative cycle (contradiction)
        if diag < B::zero() {
            return true;
        }
    }
    false
}

/// Check if the DBM represents the universal set (top).
///
/// A DBM is top if all non-diagonal entries are +infinity.
pub fn is_top<B: Bound>(dbm: &Dbm<B>) -> bool {
    let dim = dbm.dim();
    for i in 0..dim {
        for j in 0..dim {
            let val = dbm.get(i, j);
            if i == j {
                // Diagonal entries must be exactly zero
                if val != B::zero() {
                    return false;
                }
            } else {
                // All non-diagonal entries must be +infinity
                if !val.is_pos_infinity() {
                    return false;
                }
            }
        }
    }
    true
}

/// Check if octagon `a` is included in octagon `b`.
///
/// For closed DBMs: `a <= b` iff `a[i,j] <= b[i,j]` for all i,j.
///
/// Only `b` needs to be closed for this check.
pub fn is_included<B: Bound>(a: &Dbm<B>, b: &Dbm<B>) -> bool {
    assert_eq!(a.dim(), b.dim(), "DBMs must have the same dimension for inclusion check");
    let dim = a.dim();
    for i in 0..dim {
        for j in 0..dim {
            let a_val = a.get(i, j);
            let b_val = b.get(i, j);
            // a is included in b iff a[i,j] <= b[i,j] for all i,j
            // If b_val is +infinity, any a_val satisfies.
            // If a_val > b_val, inclusion fails.
            if a_val > b_val {
                return false;
            }
        }
    }
    true
}

/// Check if two DBMs represent the same octagon.
///
/// For closed DBMs: `a == b` iff `a <= b` and `b <= a`.
pub fn is_equal<B: Bound>(a: &Dbm<B>, b: &Dbm<B>) -> bool {
    is_included(a, b) && is_included(b, a)
}

/// An octagonal constraint extracted from the DBM.
#[derive(Debug, Clone, PartialEq)]
pub struct OctConstraint {
    /// First variable index.
    pub var_i: usize,
    /// Whether first variable is negated.
    pub neg_i: bool,
    /// Second variable index (may equal var_i for unary constraints).
    pub var_j: usize,
    /// Whether second variable is negated.
    pub neg_j: bool,
    /// The bound value.
    pub bound: f64,
}

/// Extract all non-trivial constraints from the DBM.
///
/// Skips constraints at +infinity (unconstrained) and diagonal entries.
pub fn extract_constraints<B: Bound>(dbm: &Dbm<B>) -> Vec<OctConstraint> {
    let dim = dbm.dim();
    let mut constraints = Vec::new();

    for i in 0..dim {
        for j in 0..dim {
            if i == j {
                continue; // Skip diagonal entries (always 0 in well-formed DBMs)
            }
            let val = dbm.get(i, j);
            if val.is_pos_infinity() {
                continue; // Skip unconstrained entries
            }
            // DBM entry m[i,j] represents: x_j' - x_i' <= m[i,j]
            // where x_k' = x_{k/2} if k is even, -x_{k/2} if k is odd
            //
            // var index for i: i / 2, negated if i is odd
            // var index for j: j / 2, negated if j is odd
            let var_i = i / 2;
            let neg_i = (i % 2) == 1;
            let var_j = j / 2;
            let neg_j = (j % 2) == 1;

            constraints.push(OctConstraint {
                var_i,
                neg_i,
                var_j,
                neg_j,
                bound: bound_to_f64(val),
            });
        }
    }

    constraints
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_interval_from_dbm() {
        // Extract [lo, hi] for a single variable from DBM diagonal entries.
        //
        // For variable x0 (index 0):
        //   Upper bound = m[1, 0] / 2
        //   Lower bound = -m[0, 1] / 2
        let mut dbm = Dbm::<f64>::new(2);

        // x0 <= 5: m[1, 0] = 10 (2*5)
        dbm.set(1, 0, 10.0);
        // x0 >= -3: m[0, 1] = 6 (2*3 because -(-3) = 3, then 2*3)
        dbm.set(0, 1, 6.0);

        let (lo, hi) = project_interval(&dbm, 0);

        assert_eq!(
            hi,
            Some(5.0),
            "Upper bound should be m[1,0]/2 = 10/2 = 5; got {hi:?}"
        );
        assert_eq!(
            lo,
            Some(-3.0),
            "Lower bound should be -m[0,1]/2 = -6/2 = -3; got {lo:?}"
        );
    }

    #[test]
    fn test_project_interval_unbounded() {
        // Fresh DBM: all entries +infinity => unbounded interval
        let dbm = Dbm::<f64>::new(2);

        let (lo, hi) = project_interval(&dbm, 0);

        assert_eq!(lo, None, "Unbounded lower bound should be None");
        assert_eq!(hi, None, "Unbounded upper bound should be None");
    }

    #[test]
    fn test_project_consistent_with_constraints() {
        // Projected interval must respect all DBM constraints.
        //
        // Set up: x in [2, 8], y in [0, 5], x - y <= 3
        // After closure, x's upper bound should be tightened:
        //   x <= y + 3, y <= 5 => x <= 8 (from unary)
        //   But also x <= y + 3 and y <= 5, so x <= 8 (consistent)
        let mut dbm = Dbm::<f64>::new(2);

        // x in [2, 8]
        dbm.set(1, 0, 16.0);   // x <= 8: m[1,0] = 2*8
        dbm.set(0, 1, -4.0);   // x >= 2: m[0,1] = -(2*2)

        // y in [0, 5]
        dbm.set(3, 2, 10.0);   // y <= 5
        dbm.set(2, 3, 0.0);    // y >= 0

        // x - y <= 3
        dbm.set(2, 0, 3.0);

        let (lo, hi) = project_interval(&dbm, 0);

        // x upper bound: should be at most 8 (from unary bound)
        if let Some(h) = hi {
            assert!(
                h <= 8.0,
                "Projected upper bound must respect constraints; got {h}"
            );
        }
        // x lower bound: should be at least 2
        if let Some(l) = lo {
            assert!(
                l >= 2.0,
                "Projected lower bound must respect constraints; got {l}"
            );
        }
    }

    #[test]
    fn test_is_bottom_negative_cycle() {
        // DBM with negative cycle -> is_bottom() == true
        let mut dbm = Dbm::<f64>::new(1);

        // Create negative diagonal: m[0, 0] = -1 (inconsistent)
        dbm.set(0, 0, -1.0);

        assert!(
            is_bottom(&dbm),
            "DBM with negative diagonal entry should be bottom"
        );
    }

    #[test]
    fn test_is_bottom_consistent() {
        // Consistent DBM -> is_bottom() == false
        let mut dbm = Dbm::<f64>::new(2);
        dbm.set(1, 0, 10.0); // x <= 5
        dbm.set(0, 1, -4.0); // x >= 2

        assert!(
            !is_bottom(&dbm),
            "Consistent DBM should not be bottom"
        );
    }

    #[test]
    fn test_is_top_all_infinity() {
        // DBM with all entries +infinity (except diagonal) -> is_top() == true
        let dbm = Dbm::<f64>::new(3);

        assert!(
            is_top(&dbm),
            "Fresh DBM (all +infinity except diagonal) should be top"
        );
    }

    #[test]
    fn test_is_top_with_constraints() {
        // DBM with any finite non-diagonal entry is not top
        let mut dbm = Dbm::<f64>::new(2);
        dbm.set(1, 0, 10.0); // x <= 5

        assert!(
            !is_top(&dbm),
            "DBM with finite constraint should not be top"
        );
    }

    #[test]
    fn test_inclusion_closed_dbms() {
        // a <= b iff a[i,j] <= b[i,j] for all i,j
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        // a has tighter constraints than b
        a.set(1, 0, 4.0);   // a: x <= 2
        b.set(1, 0, 10.0);  // b: x <= 5

        a.set(2, 0, 3.0);   // a: x-y <= 3
        b.set(2, 0, 7.0);   // b: x-y <= 7

        assert!(
            is_included(&a, &b),
            "a (tighter) should be included in b (looser)"
        );
        assert!(
            !is_included(&b, &a),
            "b (looser) should NOT be included in a (tighter)"
        );
    }

    #[test]
    fn test_inclusion_equal_dbms() {
        // Equal DBMs should be mutually included
        let mut a = Dbm::<f64>::new(2);
        a.set(1, 0, 10.0);
        a.set(2, 0, 5.0);

        let b = a.clone();

        assert!(is_included(&a, &b));
        assert!(is_included(&b, &a));
        assert!(is_equal(&a, &b));
    }

    #[test]
    fn test_extract_constraints_empty() {
        // Fresh DBM has no non-trivial constraints
        let dbm = Dbm::<f64>::new(3);
        let constraints = extract_constraints(&dbm);
        assert!(
            constraints.is_empty(),
            "Fresh DBM should have no non-trivial constraints"
        );
    }

    #[test]
    fn test_extract_constraints_with_bounds() {
        // DBM with finite constraints should extract them
        let mut dbm = Dbm::<f64>::new(2);
        dbm.set(1, 0, 10.0); // x <= 5
        dbm.set(2, 0, 3.0);  // x - y <= 3

        let constraints = extract_constraints(&dbm);
        assert!(
            constraints.len() >= 2,
            "Should extract at least 2 constraints; got {}",
            constraints.len()
        );
    }
}
