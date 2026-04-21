//! Property-based tests for the octagon domain.
//!
//! Uses proptest to verify algebraic properties that must hold
//! for ALL inputs, not just hand-picked examples.
//!
//! # Properties Verified
//!
//! 1. Closure idempotency: close(close(m)) == close(m)
//! 2. Join commutativity: join(a, b) == join(b, a)
//! 3. Join monotonicity: a <= join(a, b)
//! 4. Widening soundness: widen(old, new) >= new
//! 5. Incremental = full: incr_close(m, c) == full_close(add(m, c))
//!
//! # References
//!
//! - Prior art Section 5: Property-based testing strategy

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::dataflow::octagon::bound::Bound;
    use crate::dataflow::octagon::closure::{strong_closure, incremental_closure, ClosureResult};
    use crate::dataflow::octagon::dbm::Dbm;
    use crate::dataflow::octagon::operations::{join, widen};

    /// Generate a random DBM with n_vars variables and random finite constraints.
    ///
    /// Each non-diagonal entry has a 50% chance of being a random finite value
    /// in [-100, 100], otherwise it stays at +infinity.
    fn arb_dbm(n_vars: usize) -> impl Strategy<Value = Dbm<f64>> {
        let dim = 2 * n_vars;
        let n_entries = dim * (dim + 1) / 2;

        proptest::collection::vec(
            prop_oneof![
                1 => Just(f64::INFINITY),
                1 => (-100.0_f64..100.0_f64),
            ],
            n_entries,
        )
        .prop_map(move |values| {
            let mut dbm = Dbm::<f64>::new(n_vars);
            let mut idx = 0;
            for i in 0..dim {
                for j in 0..=i {
                    if i == j {
                        // Diagonal always 0
                        idx += 1;
                        continue;
                    }
                    dbm.set(i, j, values[idx]);
                    idx += 1;
                }
            }
            dbm
        })
    }

    /// Generate a pair of random DBMs with the same number of variables.
    fn arb_dbm_pair(n_vars: usize) -> impl Strategy<Value = (Dbm<f64>, Dbm<f64>)> {
        (arb_dbm(n_vars), arb_dbm(n_vars))
    }

    // =========================================================================
    // Property 1: Closure idempotency
    // =========================================================================

    proptest! {
        #[test]
        fn prop_closure_idempotent(dbm in arb_dbm(3)) {
            let mut m1 = dbm.clone();
            let result1 = strong_closure(&mut m1);

            if result1 == ClosureResult::Closed {
                let mut m2 = m1.clone();
                let result2 = strong_closure(&mut m2);

                prop_assert_eq!(result2, ClosureResult::Closed,
                    "Second closure of a non-empty DBM should also be Closed");

                for i in 0..m1.dim() {
                    for j in 0..m1.dim() {
                        let v1 = m1.get(i, j);
                        let v2 = m2.get(i, j);
                        prop_assert!(
                            (v1 - v2).abs() < 1e-10 || (v1.is_infinite() && v2.is_infinite()),
                            "close(close(m))[{},{}] = {} != close(m)[{},{}] = {}",
                            i, j, v2, i, j, v1
                        );
                    }
                }
            }
        }
    }

    // =========================================================================
    // Property 2: Join commutativity
    // =========================================================================

    proptest! {
        #[test]
        fn prop_join_commutative((a, b) in arb_dbm_pair(3)) {
            let ab = join(&a, &b).unwrap();
            let ba = join(&b, &a).unwrap();

            for i in 0..ab.dim() {
                for j in 0..ab.dim() {
                    let vab = ab.get(i, j);
                    let vba = ba.get(i, j);
                    prop_assert!(
                        (vab - vba).abs() < 1e-10 || (vab.is_infinite() && vba.is_infinite()),
                        "join(a,b)[{},{}] = {} != join(b,a)[{},{}] = {}",
                        i, j, vab, i, j, vba
                    );
                }
            }
        }
    }

    // =========================================================================
    // Property 3: Join monotonicity (soundness)
    // =========================================================================

    proptest! {
        #[test]
        fn prop_join_monotone((a, b) in arb_dbm_pair(3)) {
            let joined = join(&a, &b).unwrap();

            for i in 0..a.dim() {
                for j in 0..a.dim() {
                    prop_assert!(
                        a.get(i, j) <= joined.get(i, j) + 1e-10,
                        "Monotonicity: a[{},{}] = {} > join(a,b)[{},{}] = {}",
                        i, j, a.get(i, j), i, j, joined.get(i, j)
                    );
                    prop_assert!(
                        b.get(i, j) <= joined.get(i, j) + 1e-10,
                        "Monotonicity: b[{},{}] = {} > join(a,b)[{},{}] = {}",
                        i, j, b.get(i, j), i, j, joined.get(i, j)
                    );
                }
            }
        }
    }

    // =========================================================================
    // Property 4: Widening soundness
    // =========================================================================

    proptest! {
        #[test]
        fn prop_widen_sound((old, new) in arb_dbm_pair(3)) {
            let widened = widen(&old, &new);

            for i in 0..new.dim() {
                for j in 0..new.dim() {
                    prop_assert!(
                        widened.get(i, j) >= new.get(i, j) - 1e-10,
                        "Widening soundness: widen[{},{}] = {} < new[{},{}] = {}",
                        i, j, widened.get(i, j), i, j, new.get(i, j)
                    );
                }
            }
        }
    }

    // =========================================================================
    // Property 5: Incremental closure matches full closure
    // =========================================================================

    proptest! {
        #[test]
        fn prop_incremental_matches_full(
            dbm in arb_dbm(3),
            i in 0_usize..6,
            j in 0_usize..6,
            value in -50.0_f64..50.0_f64,
        ) {
            // Ensure i != j (no diagonal modification)
            prop_assume!(i != j);

            // Start with a closed DBM
            let mut m_incr = dbm.clone();
            let r1 = strong_closure(&mut m_incr);

            if r1 == ClosureResult::Closed {
                // Clone for full closure comparison
                let mut m_full = m_incr.clone();

                // Add constraint incrementally
                let r_incr = incremental_closure(&mut m_incr, i, j, value);

                // Add same constraint and re-close fully
                let old_val = m_full.get(i, j);
                m_full.set(i, j, Bound::min(old_val, value));
                let r_full = strong_closure(&mut m_full);

                // Results should match
                prop_assert_eq!(
                    r_incr, r_full,
                    "Incremental and full closure should agree on emptiness"
                );

                if r_incr == ClosureResult::Closed {
                    for ii in 0..m_incr.dim() {
                        for jj in 0..m_incr.dim() {
                            let vi = m_incr.get(ii, jj);
                            let vf = m_full.get(ii, jj);
                            prop_assert!(
                                (vi - vf).abs() < 1e-10
                                    || (vi.is_infinite() && vf.is_infinite()),
                                "incr[{},{}] = {} != full[{},{}] = {}",
                                ii, jj, vi, ii, jj, vf
                            );
                        }
                    }
                }
            }
        }
    }
}
