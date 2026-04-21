//! Transfer functions for the octagon domain.
//!
//! Transfer functions model the effect of program statements on the
//! octagonal abstract state:
//!
//! - **Assignment** (`x := expr`): Forgets old constraints on `x`,
//!   then derives new constraints from the expression.
//! - **Guard/Test** (`x > 0`, `x <= y`): Adds constraints to the DBM.
//! - **Forget** (`forget x`): Sets all constraints involving `x` to +infinity.
//!
//! # References
//!
//! - Mine 2006, Section 4.7: Transfer functions
//! - Prior art Section 6.2: Assignment transfer

use super::bound::Bound;
use super::dbm::Dbm;
use super::closure::ClosureResult;

/// The kind of expression on the RHS of an assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum OctExpr {
    /// Constant: `x := c`
    Constant(i64),
    /// Variable copy: `x := y`
    Variable(usize),
    /// Variable plus constant: `x := y + c`
    VarPlusConst(usize, i64),
    /// Variable minus constant: `x := y - c`
    VarMinusConst(usize, i64),
    /// Negated variable: `x := -y`
    NegVariable(usize),
    /// Unknown expression: `x := ?` (forget all info about x)
    Unknown,
}

/// Assign `x := expr` in the octagon.
///
/// Steps:
/// 1. Forget all constraints on variable `var_idx`.
/// 2. Add new constraints based on `expr`.
/// 3. Optionally run incremental closure.
///
/// # Arguments
///
/// - `dbm`: The DBM to modify in-place.
/// - `var_idx`: The program variable index (0-based) being assigned.
/// - `expr`: The expression being assigned.
///
/// # Returns
///
/// `ClosureResult::Closed` if assignment succeeded,
/// `ClosureResult::Empty` if assignment created a contradiction.
pub fn assign<B: Bound>(dbm: &mut Dbm<B>, var_idx: usize, expr: &OctExpr) -> ClosureResult {
    let pos = 2 * var_idx;
    let neg = pos + 1;

    // Step 1: Forget all constraints on var_idx
    forget(dbm, var_idx);

    // Step 2: Add new constraints based on expression
    match *expr {
        OctExpr::Constant(c) => {
            // x := c
            // Upper bound: x <= c => m[2v+1, 2v] = 2c
            // Lower bound: x >= c => m[2v, 2v+1] = -2c
            let two_c = B::from_i64(2 * c);
            dbm.set(neg, pos, two_c);
            dbm.set(pos, neg, two_c.neg());
        }
        OctExpr::Variable(y) => {
            // x := y
            // Copy unary bounds from y to x:
            //   m[2x+1, 2x] = m[2y+1, 2y]  (upper bound)
            //   m[2x, 2x+1] = m[2y, 2y+1]  (lower bound)
            let y_pos = 2 * y;
            let y_neg = y_pos + 1;
            dbm.set(neg, pos, dbm.get(y_neg, y_pos));
            dbm.set(pos, neg, dbm.get(y_pos, y_neg));

            // Relational: x - y = 0
            //   m[2y, 2x] = 0 (x - y <= 0)
            //   m[2x, 2y] = 0 (y - x <= 0)
            dbm.set(y_pos, pos, B::zero());
            dbm.set(pos, y_pos, B::zero());

            // Also set cross-negated constraints for coherence:
            //   m[2y+1, 2x+1] = 0 (-x - (-y) <= 0, i.e., y - x <= 0)
            //   m[2x+1, 2y+1] = 0 (-y - (-x) <= 0, i.e., x - y <= 0)
            dbm.set(y_neg, neg, B::zero());
            dbm.set(neg, y_neg, B::zero());
        }
        OctExpr::VarPlusConst(y, c) => {
            // x := y + c
            // Upper bound: m[2x+1, 2x] = m[2y+1, 2y] + 2c
            // Lower bound: m[2x, 2x+1] = m[2y, 2y+1] - 2c
            let y_pos = 2 * y;
            let y_neg = y_pos + 1;
            let two_c = B::from_i64(2 * c);

            let y_upper = dbm.get(y_neg, y_pos);
            if !y_upper.is_pos_infinity() {
                dbm.set(neg, pos, y_upper.closure_add(two_c));
            }

            let y_lower = dbm.get(y_pos, y_neg);
            if !y_lower.is_pos_infinity() {
                dbm.set(pos, neg, y_lower.closure_add(two_c.neg()));
            }

            // Relational: x - y = c
            //   m[2y, 2x] = c  (x - y <= c)
            //   m[2x, 2y] = -c (y - x <= -c)
            let bc = B::from_i64(c);
            dbm.set(y_pos, pos, bc);
            dbm.set(pos, y_pos, bc.neg());

            // Cross-negated relational constraints:
            //   m[2y+1, 2x+1] = -c  (-x + y <= -c, i.e., y - x <= -c)
            //   m[2x+1, 2y+1] = c   (-y + x <= c, i.e., x - y <= c)
            dbm.set(y_neg, neg, bc.neg());
            dbm.set(neg, y_neg, bc);
        }
        OctExpr::VarMinusConst(y, c) => {
            // x := y - c is equivalent to x := y + (-c)
            let y_pos = 2 * y;
            let y_neg = y_pos + 1;
            let two_c = B::from_i64(2 * c);

            let y_upper = dbm.get(y_neg, y_pos);
            if !y_upper.is_pos_infinity() {
                dbm.set(neg, pos, y_upper.closure_add(two_c.neg()));
            }

            let y_lower = dbm.get(y_pos, y_neg);
            if !y_lower.is_pos_infinity() {
                dbm.set(pos, neg, y_lower.closure_add(two_c));
            }

            // Relational: x - y = -c
            let bc = B::from_i64(c);
            dbm.set(y_pos, pos, bc.neg());
            dbm.set(pos, y_pos, bc);

            // Cross-negated:
            dbm.set(y_neg, neg, bc);
            dbm.set(neg, y_neg, bc.neg());
        }
        OctExpr::NegVariable(y) => {
            // x := -y
            // Upper bound: x <= -lower(y) => m[2x+1, 2x] = m[2y, 2y+1]
            //   (since -lower(y) = m[2y, 2y+1] / 2, scaled: m[2y, 2y+1])
            // Lower bound: x >= -upper(y) => m[2x, 2x+1] = m[2y+1, 2y]
            //   (since -upper(y) = -m[2y+1, 2y] / 2, but as negated: m[2y+1, 2y])
            //
            // Actually in the octagon encoding, negation swaps pos/neg:
            //   m[2x+1, 2x] = m[2y, 2y+1]  (upper of x = negated lower of y)
            //   m[2x, 2x+1] = m[2y+1, 2y]  (lower of x = negated upper of y)
            let y_pos = 2 * y;
            let y_neg = y_pos + 1;
            dbm.set(neg, pos, dbm.get(y_pos, y_neg));
            dbm.set(pos, neg, dbm.get(y_neg, y_pos));

            // Relational: x + y = 0
            //   m[2y+1, 2x] = 0  (x + y <= 0 in sum form)
            //   m[2x, 2y+1] = 0
            //   m[2y, 2x+1] = 0
            //   m[2x+1, 2y] = 0
            dbm.set(y_neg, pos, B::zero());
            dbm.set(pos, y_neg, B::zero());
            dbm.set(y_pos, neg, B::zero());
            dbm.set(neg, y_pos, B::zero());
        }
        OctExpr::Unknown => {
            // x := ? -- forget already done above, nothing more to add
        }
    }

    ClosureResult::Closed
}

/// Add a guard/test constraint to the octagon.
///
/// Supported guards:
/// - `x >= c`: Lower bound on x
/// - `x <= c`: Upper bound on x
/// - `x - y <= c`: Difference bound
/// - `x + y <= c`: Sum bound
///
/// # Returns
///
/// `ClosureResult::Empty` if the guard makes the state empty (dead branch).
pub fn guard<B: Bound>(dbm: &mut Dbm<B>, cond: &OctGuard) -> ClosureResult {
    match *cond {
        OctGuard::GtEq(x, c) => {
            // x >= c => -2x <= -2c => m[2x, 2x+1] = min(old, -2c)
            let pos = 2 * x;
            let neg = pos + 1;
            let bound = B::from_i64(-2 * c);
            dbm.set(pos, neg, dbm.get(pos, neg).min(bound));
        }
        OctGuard::LtEq(x, c) => {
            // x <= c => 2x <= 2c => m[2x+1, 2x] = min(old, 2c)
            let pos = 2 * x;
            let neg = pos + 1;
            let bound = B::from_i64(2 * c);
            dbm.set(neg, pos, dbm.get(neg, pos).min(bound));
        }
        OctGuard::Gt(x, c) => {
            // x > c => x >= c + 1 (integer domain) => -2x <= -2(c+1)
            let pos = 2 * x;
            let neg = pos + 1;
            let bound = B::from_i64(-2 * (c + 1));
            dbm.set(pos, neg, dbm.get(pos, neg).min(bound));
        }
        OctGuard::Lt(x, c) => {
            // x < c => x <= c - 1 (integer domain) => 2x <= 2(c-1)
            let pos = 2 * x;
            let neg = pos + 1;
            let bound = B::from_i64(2 * (c - 1));
            dbm.set(neg, pos, dbm.get(neg, pos).min(bound));
        }
        OctGuard::DiffLtEq(x, y, c) => {
            // x - y <= c
            // In the DBM: m[2y, 2x] encodes (+x) - (+y) <= m[2y, 2x]
            // So: m[2y, 2x] = min(old, c)
            let x_pos = 2 * x;
            let y_pos = 2 * y;
            let bound = B::from_i64(c);
            dbm.set(y_pos, x_pos, dbm.get(y_pos, x_pos).min(bound));
        }
        OctGuard::SumLtEq(x, y, c) => {
            // x + y <= c
            // In the DBM: m[2y+1, 2x] encodes (+x) - (-y) <= m[2y+1, 2x]
            //   which is (+x) + (+y) <= m[2y+1, 2x]
            // So: m[2y+1, 2x] = min(old, c)
            let x_pos = 2 * x;
            let y_neg = 2 * y + 1;
            let bound = B::from_i64(c);
            dbm.set(y_neg, x_pos, dbm.get(y_neg, x_pos).min(bound));
        }
    }

    // Check for empty state: if any diagonal is negative, the state is bottom
    let dim = dbm.dim();
    for i in 0..dim {
        let diag = dbm.get(i, i);
        if diag < B::zero() {
            return ClosureResult::Empty;
        }
    }

    ClosureResult::Closed
}

/// Guard/test constraint.
#[derive(Debug, Clone, PartialEq)]
pub enum OctGuard {
    /// x >= c (lower bound)
    GtEq(usize, i64),
    /// x <= c (upper bound)
    LtEq(usize, i64),
    /// x > c (strict lower bound, encoded as x >= c+1 for integers)
    Gt(usize, i64),
    /// x < c (strict upper bound, encoded as x <= c-1 for integers)
    Lt(usize, i64),
    /// x - y <= c (difference)
    DiffLtEq(usize, usize, i64),
    /// x + y <= c (sum)
    SumLtEq(usize, usize, i64),
}

/// Forget all constraints involving variable `var_idx`.
///
/// Sets all DBM entries in the row and column corresponding to
/// both `2*var_idx` and `2*var_idx + 1` to +infinity (except diagonal).
pub fn forget<B: Bound>(dbm: &mut Dbm<B>, var_idx: usize) {
    let dim = dbm.dim();
    let pos = 2 * var_idx;
    let neg = 2 * var_idx + 1;

    for k in 0..dim {
        // Skip self-loops (diagonal entries must stay 0)
        if k != pos {
            dbm.set(pos, k, B::infinity());
            dbm.set(k, pos, B::infinity());
        }
        if k != neg {
            dbm.set(neg, k, B::infinity());
            dbm.set(k, neg, B::infinity());
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assign_constant() {
        // x := 5 should set DBM constraints for x in [5, 5]
        //
        // For variable x (index 0):
        //   x <= 5: m[1, 0] = 10 (i.e., 2*5)
        //   x >= 5: m[0, 1] = -10 (i.e., -(2*5))
        let mut dbm = Dbm::<f64>::new(2);

        assign(&mut dbm, 0, &OctExpr::Constant(5));

        // Upper bound: m[1, 0] / 2 should give x <= 5
        let upper = dbm.get(1, 0);
        assert_eq!(
            upper, 10.0,
            "x := 5 should set upper bound m[1,0] = 10 (2*5); got {upper}"
        );

        // Lower bound: m[0, 1] / 2 negated should give x >= 5
        let lower = dbm.get(0, 1);
        assert_eq!(
            lower, -10.0,
            "x := 5 should set lower bound m[0,1] = -10 (-(2*5)); got {lower}"
        );
    }

    #[test]
    fn test_assign_variable_copy() {
        // x := y should copy relational constraints from y to x.
        //
        // Before: y in [3, 7]
        // After x := y: x in [3, 7] AND x - y = 0 (x == y)
        let mut dbm = Dbm::<f64>::new(2);

        // Set y (index 1) bounds: y in [3, 7]
        dbm.set(3, 2, 14.0);  // y <= 7: m[3,2] = 2*7
        dbm.set(2, 3, -6.0);  // y >= 3: m[2,3] = -(2*3)

        assign(&mut dbm, 0, &OctExpr::Variable(1));

        // x should have same bounds as y
        let x_upper = dbm.get(1, 0);
        assert_eq!(
            x_upper, 14.0,
            "x := y should give x same upper bound as y (14.0); got {x_upper}"
        );

        let x_lower = dbm.get(0, 1);
        assert_eq!(
            x_lower, -6.0,
            "x := y should give x same lower bound as y (-6.0); got {x_lower}"
        );

        // Relational: x - y <= 0 and y - x <= 0 (x == y)
        let x_minus_y = dbm.get(2, 0);
        assert_eq!(
            x_minus_y, 0.0,
            "x := y should set x - y <= 0; got {x_minus_y}"
        );

        let y_minus_x = dbm.get(0, 2);
        assert_eq!(
            y_minus_x, 0.0,
            "x := y should set y - x <= 0; got {y_minus_x}"
        );
    }

    #[test]
    fn test_assign_arithmetic() {
        // x := y + 1 should create constraint x - y = 1
        //
        // Before: y in [3, 7]
        // After x := y + 1: x in [4, 8], x - y = 1
        let mut dbm = Dbm::<f64>::new(2);

        // Set y (index 1) bounds
        dbm.set(3, 2, 14.0);  // y <= 7
        dbm.set(2, 3, -6.0);  // y >= 3

        assign(&mut dbm, 0, &OctExpr::VarPlusConst(1, 1));

        // x should be y + 1: x in [4, 8]
        let x_upper = dbm.get(1, 0);
        assert_eq!(
            x_upper, 16.0,
            "x := y + 1 should give x upper bound 2*8 = 16; got {x_upper}"
        );

        let x_lower = dbm.get(0, 1);
        assert_eq!(
            x_lower, -8.0,
            "x := y + 1 should give x lower bound -(2*4) = -8; got {x_lower}"
        );

        // Relational: x - y <= 1
        let x_minus_y = dbm.get(2, 0);
        assert_eq!(
            x_minus_y, 1.0,
            "x := y + 1 should set x - y <= 1; got {x_minus_y}"
        );

        // y - x <= -1
        let y_minus_x = dbm.get(0, 2);
        assert_eq!(
            y_minus_x, -1.0,
            "x := y + 1 should set y - x <= -1; got {y_minus_x}"
        );
    }

    #[test]
    fn test_assign_forget() {
        // Assignment first forgets old constraints on x.
        //
        // Before: x in [1, 3], y in [5, 10], x - y <= 0
        // After x := 7: x in [7, 7], old x - y constraint is gone.
        let mut dbm = Dbm::<f64>::new(2);

        // Set up initial state with relational constraint
        dbm.set(1, 0, 6.0);   // x <= 3
        dbm.set(0, 1, -2.0);  // x >= 1
        dbm.set(2, 0, 0.0);   // x - y <= 0

        assign(&mut dbm, 0, &OctExpr::Constant(7));

        // Old relational constraint x - y <= 0 should be forgotten
        // After x := 7, x - y depends on y's bounds only
        let x_upper = dbm.get(1, 0);
        assert_eq!(
            x_upper, 14.0,
            "After x := 7, upper bound should be 2*7 = 14; got {x_upper}"
        );

        let x_lower = dbm.get(0, 1);
        assert_eq!(
            x_lower, -14.0,
            "After x := 7, lower bound should be -(2*7) = -14; got {x_lower}"
        );
    }

    #[test]
    fn test_assign_preserves_other_constraints() {
        // Assigning x should not affect y's constraints.
        let mut dbm = Dbm::<f64>::new(2);

        // Set y bounds
        dbm.set(3, 2, 20.0);  // y <= 10
        dbm.set(2, 3, -4.0);  // y >= 2

        assign(&mut dbm, 0, &OctExpr::Constant(5));

        // y constraints should be unchanged
        assert_eq!(
            dbm.get(3, 2),
            20.0,
            "y upper bound should be unchanged after assigning x"
        );
        assert_eq!(
            dbm.get(2, 3),
            -4.0,
            "y lower bound should be unchanged after assigning x"
        );
    }

    // =========================================================================
    // Guard Tests
    // =========================================================================

    #[test]
    fn test_guard_positive() {
        // guard(x > 0) should tighten x's lower bound.
        // For integers: x > 0 => x >= 1 => m[2*0, 2*0+1] = -(2*1) = -2
        let mut dbm = Dbm::<f64>::new(2);

        guard(&mut dbm, &OctGuard::Gt(0, 0));

        // x >= 1 (for integers, x > 0 means x >= 1)
        let lower = dbm.get(0, 1);
        assert!(
            lower <= -2.0,
            "guard(x > 0) should set x >= 1: m[0,1] <= -2; got {lower}"
        );
    }

    #[test]
    fn test_guard_relational() {
        // guard(x <= y) should add constraint x - y <= 0
        let mut dbm = Dbm::<f64>::new(2);

        guard(&mut dbm, &OctGuard::DiffLtEq(0, 1, 0));

        let x_minus_y = dbm.get(2, 0);
        assert!(
            x_minus_y <= 0.0,
            "guard(x <= y) should set x - y <= 0; got {x_minus_y}"
        );
    }

    // =========================================================================
    // Forget Tests
    // =========================================================================

    #[test]
    fn test_forget_variable() {
        // Forgetting x sets all constraints involving x to +infinity
        let mut dbm = Dbm::<f64>::new(2);

        // Set some constraints on x (var 0)
        dbm.set(1, 0, 6.0);   // x <= 3
        dbm.set(0, 1, -2.0);  // x >= 1
        dbm.set(2, 0, 4.0);   // x - y <= 4

        forget(&mut dbm, 0);

        // All entries involving x (indices 0 and 1) should be +infinity
        // except diagonal
        for i in 0..dbm.dim() {
            for j in 0..dbm.dim() {
                if i == j {
                    continue;
                }
                // Check if this entry involves variable 0 (indices 0 or 1)
                let involves_x = (i < 2) || (j < 2);
                if involves_x {
                    assert_eq!(
                        dbm.get(i, j),
                        f64::INFINITY,
                        "After forget(x), m[{i},{j}] should be +infinity; got {}",
                        dbm.get(i, j)
                    );
                }
            }
        }
    }
}
