//! Octagon Abstract Domain (Mine 2006)
//!
//! This module implements the octagon abstract domain, which tracks
//! relational constraints of the form `+-xi +- xj <= c` using a
//! Difference Bound Matrix (DBM) representation.
//!
//! # Architecture
//!
//! ```text
//! octagon/
//! +-- mod.rs        -- Public API (this file)
//! +-- bound.rs      -- Bound trait + f64/i64 implementations
//! +-- dbm.rs        -- Half-matrix DBM storage, indexing
//! +-- closure.rs    -- Strong closure, incremental closure, tight closure
//! +-- operations.rs -- Join, meet, widening, narrowing
//! +-- transfer.rs   -- Assignment, guard/test, forget
//! +-- pack.rs       -- Variable packing infrastructure
//! +-- query.rs      -- Inclusion, equality, constraint extraction
//! ```
//!
//! # Usage
//!
//! The octagon domain provides the same public API as the interval domain
//! (`AbstractState::get()`, `AbstractState::set()`, etc.) but internally
//! uses a DBM for relational precision.
//!
//! # References
//!
//! - Mine, A. (2006). "The Octagon Abstract Domain."
//!   Higher-Order and Symbolic Computation, 19(1), 31-100.
//! - Bagnara, R., Hill, P.M., Zaffanella, E. (2018).
//!   "Incrementally Closing Octagons."
//!   Formal Methods in System Design, 51(2), 342-363.

pub mod bound;
pub mod closure;
pub mod dbm;
pub mod operations;
pub mod pack;
pub mod query;
pub mod transfer;

// Property-based tests (proptest)
#[cfg(test)]
mod properties;

// Re-exports for convenience
pub use bound::Bound;
pub use closure::{strong_closure, incremental_closure, tight_closure, ClosureResult};
pub use dbm::Dbm;
pub use operations::{join, meet, widen, widen_with_thresholds, is_included};
pub use pack::{Pack, HybridState, IntervalValue, DEFAULT_PACK_LIMIT};
pub use query::{project_interval, is_bottom, is_top, extract_constraints};
pub use transfer::{assign, guard, forget, OctExpr, OctGuard};

/// Octagon state: wraps a DBM with variable name mapping.
///
/// This is the main entry point for the octagon domain. It provides
/// the same API as the existing `AbstractState` but with relational
/// precision internally.
#[derive(Debug, Clone)]
pub struct OctagonState {
    /// The hybrid state (pack + overflow intervals).
    state: HybridState<f64>,
}

impl OctagonState {
    /// Create a new empty octagon state.
    pub fn new() -> Self {
        OctagonState {
            state: HybridState::new(),
        }
    }

    /// Get the abstract value (interval projection) for a variable.
    ///
    /// For packed variables, projects from the DBM.
    /// For overflow variables, returns the stored interval.
    /// For unknown variables, returns top (no information).
    pub fn get(&self, var: &str) -> IntervalValue {
        self.state.get_interval(var)
    }

    /// Set the value of a variable.
    ///
    /// If the variable is in the pack, updates the DBM.
    /// If the variable is in overflow, updates the interval.
    /// If the variable is new and the pack is not full, adds to pack.
    /// If the variable is new and the pack is full, adds to overflow.
    pub fn set(&mut self, var: &str, value: IntervalValue) {
        // Try to add the variable to the pack (returns existing index if already present)
        if let Some(idx) = self.state.pack_mut().add_var(var) {
            // Variable is in the pack: set bounds in the DBM
            let pos = 2 * idx;
            let neg = pos + 1;
            let dbm = self.state.pack_mut().dbm_mut();

            // First forget old constraints on this variable
            forget(dbm, idx);

            // Set upper bound: x <= hi => m[2v+1, 2v] = 2*hi
            if let Some(hi) = value.high {
                let two_hi = 2.0 * hi as f64;
                dbm.set(neg, pos, two_hi);
            }
            // Set lower bound: x >= lo => m[2v, 2v+1] = -2*lo
            if let Some(lo) = value.low {
                let neg_two_lo = -2.0 * lo as f64;
                dbm.set(pos, neg, neg_two_lo);
            }
        } else {
            // Pack is full: store in overflow intervals
            self.state.overflow_mut().insert(var.to_string(), value);
        }
    }

    /// Copy the state (explicit clone).
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Get all variable names tracked in this octagon state.
    ///
    /// Returns packed variable names followed by overflow variable names.
    /// The order within each group is deterministic: packed variables are
    /// ordered by pack index; overflow variables are in arbitrary order.
    pub fn var_names(&self) -> Vec<String> {
        let pack_names = self.state.pack().var_names();
        let overflow_names = self.state.overflow().keys();
        let mut names: Vec<String> = pack_names.to_vec();
        for name in overflow_names {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names
    }

    /// Check if a variable may be zero.
    ///
    /// Conservative: returns `true` if unsure.
    pub fn may_be_zero(&self, var: &str) -> bool {
        let iv = self.get(var);
        // If the lower bound is known and strictly positive, zero is excluded.
        if let Some(lo) = iv.low {
            if lo > 0 {
                return false;
            }
        }
        // If the upper bound is known and strictly negative, zero is excluded.
        if let Some(hi) = iv.high {
            if hi < 0 {
                return false;
            }
        }
        // Both bounds known and zero is outside the range.
        if let (Some(lo), Some(hi)) = (iv.low, iv.high) {
            return lo <= 0 && hi >= 0;
        }
        // At least one bound is unknown and the known bound (if any) doesn't
        // exclude zero -- conservatively assume zero is possible.
        true
    }

    /// Check if a variable may be null.
    ///
    /// Conservative: returns `true` if unsure.
    /// (Nullability is tracked separately, not in the DBM.)
    pub fn may_be_null(&self, _var: &str) -> bool {
        // Conservative: always true until we have nullability tracking
        true
    }

    /// Access the inner `HybridState` (immutable).
    ///
    /// Used for direct DBM operations such as join/widen on raw DBMs.
    pub fn hybrid_state(&self) -> &HybridState<f64> {
        &self.state
    }

    /// Access the inner `HybridState` (mutable).
    ///
    /// Used for in-place DBM modifications during transfer functions.
    pub fn hybrid_state_mut(&mut self) -> &mut HybridState<f64> {
        &mut self.state
    }

    /// Join two octagon states (least upper bound).
    ///
    /// If both states have packs with the same variables (same size and names),
    /// uses `operations::join()` on the DBMs directly for relational precision.
    /// Otherwise, falls back to interval-based join: projects both states to
    /// intervals, takes the wider range for each variable, and builds a new
    /// `OctagonState` from the result.
    ///
    /// Overflow intervals are joined independently (wider range wins).
    pub fn join(&self, other: &OctagonState) -> OctagonState {
        let self_pack = self.state.pack();
        let other_pack = other.state.pack();

        // Check if packs are compatible (same variables in same order)
        let packs_compatible = self_pack.len() == other_pack.len()
            && self_pack.var_names() == other_pack.var_names();

        if packs_compatible && !self_pack.is_empty() {
            // Use relational join on DBMs
            if let Some(joined_dbm) = operations::join(self_pack.dbm(), other_pack.dbm()) {
                let mut result = OctagonState::new();
                // Rebuild pack with same variables
                for name in self_pack.var_names() {
                    result.state.pack_mut().add_var(name);
                }
                // Copy joined DBM entries
                let dim = joined_dbm.dim();
                for i in 0..dim {
                    for j in 0..dim {
                        result.state.pack_mut().dbm_mut().set(i, j, joined_dbm.get(i, j));
                    }
                }
                // Join overflow intervals
                let all_overflow_keys: std::collections::HashSet<&String> = self
                    .state
                    .overflow()
                    .keys()
                    .chain(other.state.overflow().keys())
                    .collect();
                for key in all_overflow_keys {
                    let iv_self = self.state.overflow().get(key).cloned().unwrap_or_else(IntervalValue::top);
                    let iv_other = other.state.overflow().get(key).cloned().unwrap_or_else(IntervalValue::top);
                    let joined_iv = join_intervals(&iv_self, &iv_other);
                    result.state.overflow_mut().insert(key.clone(), joined_iv);
                }
                return result;
            }
        }

        // Fallback: interval-based join
        self.join_by_intervals(other)
    }

    /// Fallback join via interval projection.
    ///
    /// Projects both states to intervals, takes the wider range per variable,
    /// and constructs a fresh `OctagonState`.  Loses relational information
    /// but is always correct (sound over-approximation).
    fn join_by_intervals(&self, other: &OctagonState) -> OctagonState {
        let mut result = OctagonState::new();
        let mut all_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
        for name in self.var_names() {
            all_vars.insert(name);
        }
        for name in other.var_names() {
            all_vars.insert(name);
        }
        for name in &all_vars {
            let iv_self = self.get(name);
            let iv_other = other.get(name);
            let joined = join_intervals(&iv_self, &iv_other);
            result.set(name, joined);
        }
        result
    }

    /// Widen two octagon states (old, new) for fixpoint termination.
    ///
    /// If both states have compatible packs, uses `operations::widen()` on the
    /// DBMs directly.  Otherwise, falls back to interval-based widening.
    ///
    /// Constraints that grew between `old` and `new` are set to +infinity,
    /// guaranteeing termination of the fixpoint iteration in finite steps.
    pub fn widen(old: &OctagonState, new: &OctagonState) -> OctagonState {
        let old_pack = old.state.pack();
        let new_pack = new.state.pack();

        let packs_compatible = old_pack.len() == new_pack.len()
            && old_pack.var_names() == new_pack.var_names();

        if packs_compatible && !old_pack.is_empty() {
            let widened_dbm = operations::widen(old_pack.dbm(), new_pack.dbm());
            let mut result = OctagonState::new();
            // Rebuild pack with same variables
            for name in old_pack.var_names() {
                result.state.pack_mut().add_var(name);
            }
            // Copy widened DBM entries
            let dim = widened_dbm.dim();
            for i in 0..dim {
                for j in 0..dim {
                    result.state.pack_mut().dbm_mut().set(i, j, widened_dbm.get(i, j));
                }
            }
            // Widen overflow intervals
            let all_overflow_keys: std::collections::HashSet<&String> = old
                .state
                .overflow()
                .keys()
                .chain(new.state.overflow().keys())
                .collect();
            for key in all_overflow_keys {
                let iv_old = old.state.overflow().get(key).cloned().unwrap_or_else(IntervalValue::top);
                let iv_new = new.state.overflow().get(key).cloned().unwrap_or_else(IntervalValue::top);
                let widened_iv = widen_intervals(&iv_old, &iv_new);
                result.state.overflow_mut().insert(key.clone(), widened_iv);
            }
            return result;
        }

        // Fallback: interval-based widening
        let mut result = OctagonState::new();
        let mut all_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
        for name in old.var_names() {
            all_vars.insert(name);
        }
        for name in new.var_names() {
            all_vars.insert(name);
        }
        for name in &all_vars {
            let iv_old = old.get(name);
            let iv_new = new.get(name);
            let widened = widen_intervals(&iv_old, &iv_new);
            result.set(name, widened);
        }
        result
    }
}

/// Join two interval values (element-wise wider range).
///
/// For each bound, takes the less restrictive option:
/// - Lower bound: min of the two (or None if either is unbounded below)
/// - Upper bound: max of the two (or None if either is unbounded above)
fn join_intervals(a: &IntervalValue, b: &IntervalValue) -> IntervalValue {
    let low = match (a.low, b.low) {
        (Some(la), Some(lb)) => Some(Ord::min(la, lb)),
        _ => None, // Either unbounded -> result unbounded
    };
    let high = match (a.high, b.high) {
        (Some(ha), Some(hb)) => Some(Ord::max(ha, hb)),
        _ => None,
    };
    IntervalValue { low, high }
}

/// Widen two interval values for fixpoint termination.
///
/// If a bound grew (became less restrictive), set it to unbounded (+/-infinity).
/// If a bound is stable or tightened, keep the new value.
fn widen_intervals(old: &IntervalValue, new: &IntervalValue) -> IntervalValue {
    let low = match (old.low, new.low) {
        (Some(lo), Some(ln)) => {
            if ln < lo {
                None // Grew downward -> -infinity
            } else {
                Some(ln)
            }
        }
        (Some(_), None) => None, // Already grew to -infinity
        (None, new_lo) => new_lo, // Old was -inf, keep new
    };
    let high = match (old.high, new.high) {
        (Some(ho), Some(hn)) => {
            if hn > ho {
                None // Grew upward -> +infinity
            } else {
                Some(hn)
            }
        }
        (Some(_), None) => None, // Already grew to +infinity
        (None, new_hi) => new_hi, // Old was +inf, keep new
    };
    IntervalValue { low, high }
}

impl Default for OctagonState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Integration Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // OctagonState API Tests
    // =========================================================================

    #[test]
    fn test_octagon_state_implements_abstract_state_api() {
        // get(), set(), copy() must work
        let mut state = OctagonState::new();

        // Get unknown variable returns top
        let val = state.get("x");
        assert_eq!(val, IntervalValue::top(), "Unknown variable should be top");

        // Set a value
        state.set("x", IntervalValue::exact(5));

        // Get should return what we set
        let val = state.get("x");
        assert_eq!(
            val,
            IntervalValue::exact(5),
            "After set(x, [5,5]), get(x) should return [5,5]; got {val:?}"
        );

        // Copy should be independent
        let copy = state.copy();
        state.set("x", IntervalValue::exact(10));
        let copy_val = copy.get("x");
        assert_eq!(
            copy_val,
            IntervalValue::exact(5),
            "Copy should be independent of original after modification"
        );
    }

    #[test]
    fn test_octagon_may_be_zero_conservative() {
        // may_be_zero() returns true when unsure (conservative for div-by-zero)
        let state = OctagonState::new();

        // Unknown variable -> may_be_zero should be true (conservative)
        assert!(
            state.may_be_zero("unknown"),
            "may_be_zero for unknown variable should be true (conservative)"
        );
    }

    #[test]
    fn test_octagon_may_be_zero_known_nonzero() {
        let mut state = OctagonState::new();
        state.set("x", IntervalValue { low: Some(3), high: Some(10) });

        assert!(
            !state.may_be_zero("x"),
            "may_be_zero should be false when range is [3, 10] (excludes 0)"
        );
    }

    #[test]
    fn test_octagon_may_be_zero_includes_zero() {
        let mut state = OctagonState::new();
        state.set("x", IntervalValue { low: Some(-5), high: Some(5) });

        assert!(
            state.may_be_zero("x"),
            "may_be_zero should be true when range is [-5, 5] (includes 0)"
        );
    }

    #[test]
    fn test_octagon_may_be_null_conservative() {
        // may_be_null() returns true when unsure (conservative for null-deref)
        let state = OctagonState::new();

        assert!(
            state.may_be_null("unknown"),
            "may_be_null for unknown variable should be true (conservative)"
        );
    }

    #[test]
    fn test_octagon_relational_precision() {
        // Octagon should track relational constraints that intervals cannot.
        //
        // x = 5; y = x + 1
        // Interval: x in [5,5], y in [6,6] (but no relation between them)
        // Octagon: x in [5,5], y in [6,6], y - x = 1
        //
        // After implementation, querying the relational constraint
        // y - x should give exactly 1.
        let mut dbm = Dbm::<f64>::new(2);

        // x = 5
        dbm.set(1, 0, 10.0);    // x <= 5
        dbm.set(0, 1, -10.0);   // x >= 5

        // y = x + 1 => y - x = 1
        dbm.set(3, 2, 12.0);    // y <= 6
        dbm.set(2, 3, -12.0);   // y >= 6
        dbm.set(2, 0, 1.0);     // x - y <= -1 (i.e., y - x >= 1)
        // Wait, the encoding is: m[j_pos, i_pos] for x_i - x_j <= c
        // For y - x <= 1: m[0, 2] = 1  (x0 at pos 0, x1 at pos 2)
        // For x - y <= -1: m[2, 0] = -1
        dbm.set(0, 2, 1.0);     // y - x <= 1
        dbm.set(2, 0, -1.0);    // x - y <= -1

        // After closure, the relational constraint y - x = 1 should be reflected
        strong_closure(&mut dbm);

        // Project x: should be [5, 5]
        let (x_lo, x_hi) = project_interval(&dbm, 0);
        assert_eq!(
            x_hi,
            Some(5.0),
            "x upper bound should be 5; got {x_hi:?}"
        );
        assert_eq!(
            x_lo,
            Some(5.0),
            "x lower bound should be 5; got {x_lo:?}"
        );

        // Project y: should be [6, 6]
        let (y_lo, y_hi) = project_interval(&dbm, 1);
        assert_eq!(
            y_hi,
            Some(6.0),
            "y upper bound should be 6; got {y_hi:?}"
        );
        assert_eq!(
            y_lo,
            Some(6.0),
            "y lower bound should be 6; got {y_lo:?}"
        );

        // Relational: y - x <= 1 and x - y <= -1 (i.e., y - x = 1)
        assert_eq!(
            dbm.get(0, 2),
            1.0,
            "y - x should be <= 1"
        );
        assert_eq!(
            dbm.get(2, 0),
            -1.0,
            "x - y should be <= -1 (i.e., y - x >= 1)"
        );
    }

    #[test]
    fn test_octagon_conditional_precision() {
        // Octagon should exploit relational info from conditionals.
        //
        // if x > 0:
        //     y = x + 1
        //     z = 10 / y  # Safe: y > 1 (octagon knows this)
        //
        // Interval domain: after guard(x > 0), x in [1, +inf)
        //   y = x + 1 => y in [2, +inf)
        //   BUT after join with other branch, x could be [0, +inf), y could be [1, +inf)
        //   => interval says y MIGHT be 1 => div-by-zero warning (false positive)
        //
        // Octagon: y - x = 1, x >= 1 => y >= 2 (no false positive)
        let mut dbm = Dbm::<f64>::new(2);

        // guard(x > 0): x >= 1
        dbm.set(0, 1, -2.0);  // x >= 1: m[0,1] = -(2*1)

        // y = x + 1: y - x = 1
        // y - x <= 1
        dbm.set(0, 2, 1.0);
        // x - y <= -1
        dbm.set(2, 0, -1.0);

        strong_closure(&mut dbm);

        // Project y: should be [2, +inf) (tighter than [1, +inf) from intervals)
        let (y_lo, _y_hi) = project_interval(&dbm, 1);

        assert!(
            y_lo.is_some() && y_lo.unwrap() >= 2.0,
            "Octagon should derive y >= 2 from x >= 1 and y = x + 1; got y_lo = {y_lo:?}"
        );
    }

    #[test]
    fn test_octagon_div_zero_fewer_false_positives() {
        // Octagon should eliminate false positives that interval domain misses.
        //
        // x = input (unknown, but > 0 from guard)
        // y = x + 1
        // z = 10 / y  # Interval: maybe FP. Octagon: definitely safe.
        let mut state = OctagonState::new();

        // After guard(x > 0) and y = x + 1:
        // The octagon knows y >= 2, so may_be_zero(y) == false.
        state.set("y", IntervalValue { low: Some(2), high: None });

        assert!(
            !state.may_be_zero("y"),
            "Octagon should know y >= 2 (not zero), eliminating false positive"
        );
    }

    // =========================================================================
    // DBM + Closure Integration Tests
    // =========================================================================

    #[test]
    fn test_dbm_closure_join_roundtrip() {
        // Close two DBMs, join them, verify result is still valid.
        let mut a = Dbm::<f64>::new(2);
        let mut b = Dbm::<f64>::new(2);

        // a: x in [0, 5], y in [1, 3]
        a.set(1, 0, 10.0);
        a.set(0, 1, 0.0);
        a.set(3, 2, 6.0);
        a.set(2, 3, -2.0);

        // b: x in [2, 8], y in [0, 4]
        b.set(1, 0, 16.0);
        b.set(0, 1, -4.0);
        b.set(3, 2, 8.0);
        b.set(2, 3, 0.0);

        strong_closure(&mut a);
        strong_closure(&mut b);

        let joined = join(&a, &b).expect("Join should succeed");

        // Verify joined is at least as permissive as both a and b
        for i in 0..joined.dim() {
            for j in 0..joined.dim() {
                assert!(
                    joined.get(i, j) >= a.get(i, j),
                    "joined[{i},{j}] must be >= a[{i},{j}]"
                );
                assert!(
                    joined.get(i, j) >= b.get(i, j),
                    "joined[{i},{j}] must be >= b[{i},{j}]"
                );
            }
        }
    }

    #[test]
    fn test_assign_then_project() {
        // Assign x := 7, then project interval for x.
        let mut dbm = Dbm::<f64>::new(2);

        assign(&mut dbm, 0, &OctExpr::Constant(7));

        let (lo, hi) = project_interval(&dbm, 0);
        assert_eq!(
            hi,
            Some(7.0),
            "After x := 7, upper bound should be 7; got {hi:?}"
        );
        assert_eq!(
            lo,
            Some(7.0),
            "After x := 7, lower bound should be 7; got {lo:?}"
        );
    }

    #[test]
    fn test_forget_then_query() {
        // Forget x, then query should return unbounded.
        let mut dbm = Dbm::<f64>::new(2);

        // Set x bounds
        dbm.set(1, 0, 10.0);
        dbm.set(0, 1, -4.0);

        forget(&mut dbm, 0);

        let (lo, hi) = project_interval(&dbm, 0);
        assert_eq!(lo, None, "After forget, lower bound should be None");
        assert_eq!(hi, None, "After forget, upper bound should be None");
    }
}
