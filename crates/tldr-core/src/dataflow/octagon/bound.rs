//! Bound trait and implementations for octagon domain.
//!
//! The `Bound` trait abstracts over numeric types used in the DBM.
//! Two implementations are provided:
//!
//! - `f64`: Uses `f64::INFINITY` / `f64::NEG_INFINITY` as sentinels.
//!   Arithmetic uses `f64::next_up()` for sound over-approximation
//!   (equivalent to rounding toward +infinity per APRON convention).
//!
//! - `i64`: Uses saturating arithmetic. Overflow maps to `i64::MAX`
//!   (treated as +infinity).
//!
//! # References
//!
//! - Mine 2006, Section 4.1: Numeric representations
//! - APRON: `octD` (double), `octI` (long int) backends
//! - Prior art Section 6, Q4: f64 with explicit next_up rounding

use std::fmt::Debug;

/// Trait for numeric bounds used in the DBM.
///
/// Implementations must provide:
/// - Infinity sentinels (positive and negative)
/// - Sound addition (over-approximating for upper bounds)
/// - Comparison operations (min, max)
/// - A zero element
/// - Division by 2 (for strengthening pass)
pub trait Bound: Copy + Clone + Debug + PartialEq + PartialOrd + Send + Sync + 'static {
    /// Positive infinity sentinel.
    fn infinity() -> Self;

    /// Negative infinity sentinel.
    fn neg_infinity() -> Self;

    /// The zero element.
    fn zero() -> Self;

    /// Whether this value represents positive infinity.
    fn is_pos_infinity(self) -> bool;

    /// Whether this value represents negative infinity.
    fn is_neg_infinity(self) -> bool;

    /// Sound addition: a + b, rounding toward +infinity for soundness.
    ///
    /// If either operand is +infinity, result is +infinity.
    /// For `f64`, uses `next_up()` after addition.
    /// For `i64`, uses saturating arithmetic; overflow yields `i64::MAX`.
    fn add(self, other: Self) -> Self;

    /// Minimum of two bounds.
    fn min(self, other: Self) -> Self;

    /// Maximum of two bounds.
    fn max(self, other: Self) -> Self;

    /// Division by 2 (for the strengthening pass in strong closure).
    ///
    /// For integers, uses floor division: `floor(x / 2)`.
    /// For floats, exact division (no rounding needed for /2).
    fn half(self) -> Self;

    /// Negation: -x.
    fn neg(self) -> Self;

    /// Floor operation for tight closure on integers.
    /// For floats, this is a no-op (returns self).
    /// For integers, returns `2 * floor(x / 2)`.
    fn tighten(self) -> Self;

    /// Exact addition for closure algorithms (no rounding).
    ///
    /// Unlike `add()`, this does NOT apply `next_up()` rounding.
    /// Used internally by Floyd-Warshall and incremental closure where
    /// exact shortest-path arithmetic is required (APRON convention:
    /// closure operates on exact values, rounding is applied at the
    /// boundary when constraints are introduced).
    ///
    /// Infinity propagation rules are the same as `add()`.
    fn closure_add(self, other: Self) -> Self;

    /// Convert an `i64` value to this bound type.
    ///
    /// Used by transfer functions to convert integer constants from
    /// `OctExpr` into the DBM's bound representation.
    fn from_i64(val: i64) -> Self;
}

// =============================================================================
// f64 Bound implementation
// =============================================================================

impl Bound for f64 {
    #[inline]
    fn infinity() -> Self {
        f64::INFINITY
    }

    #[inline]
    fn neg_infinity() -> Self {
        f64::NEG_INFINITY
    }

    #[inline]
    fn zero() -> Self {
        0.0
    }

    #[inline]
    fn is_pos_infinity(self) -> bool {
        self == f64::INFINITY
    }

    #[inline]
    fn is_neg_infinity(self) -> bool {
        self == f64::NEG_INFINITY
    }

    #[inline]
    fn add(self, other: Self) -> Self {
        if self.is_pos_infinity() || other.is_pos_infinity() {
            return f64::INFINITY;
        }
        if self.is_neg_infinity() || other.is_neg_infinity() {
            return f64::NEG_INFINITY;
        }
        // Sound rounding: next_up() ensures over-approximation
        // (rounding toward +infinity per APRON convention, Mine 2006 Section 4.1)
        (self + other).next_up()
    }

    #[inline]
    fn min(self, other: Self) -> Self {
        f64::min(self, other)
    }

    #[inline]
    fn max(self, other: Self) -> Self {
        f64::max(self, other)
    }

    #[inline]
    fn half(self) -> Self {
        self / 2.0
    }

    #[inline]
    fn neg(self) -> Self {
        -self
    }

    #[inline]
    fn tighten(self) -> Self {
        // No-op for floats (tight closure only meaningful for integers)
        self
    }

    #[inline]
    fn closure_add(self, other: Self) -> Self {
        if self.is_pos_infinity() || other.is_pos_infinity() {
            return f64::INFINITY;
        }
        if self.is_neg_infinity() || other.is_neg_infinity() {
            return f64::NEG_INFINITY;
        }
        // Exact addition without next_up rounding, for closure algorithms
        self + other
    }

    #[inline]
    fn from_i64(val: i64) -> Self {
        val as f64
    }
}

// =============================================================================
// i64 Bound implementation
// =============================================================================

impl Bound for i64 {
    #[inline]
    fn infinity() -> Self {
        i64::MAX
    }

    #[inline]
    fn neg_infinity() -> Self {
        i64::MIN
    }

    #[inline]
    fn zero() -> Self {
        0
    }

    #[inline]
    fn is_pos_infinity(self) -> bool {
        self == i64::MAX
    }

    #[inline]
    fn is_neg_infinity(self) -> bool {
        self == i64::MIN
    }

    #[inline]
    fn add(self, other: Self) -> Self {
        if self == i64::MAX || other == i64::MAX {
            return i64::MAX;
        }
        if self == i64::MIN || other == i64::MIN {
            return i64::MIN;
        }
        // Saturating arithmetic: overflow -> i64::MAX (treated as +infinity)
        // TODO: implement properly
        self.saturating_add(other)
    }

    #[inline]
    fn min(self, other: Self) -> Self {
        Ord::min(self, other)
    }

    #[inline]
    fn max(self, other: Self) -> Self {
        Ord::max(self, other)
    }

    #[inline]
    fn half(self) -> Self {
        // Floor division for integers
        if self >= 0 {
            self / 2
        } else {
            (self - 1) / 2
        }
    }

    #[inline]
    fn neg(self) -> Self {
        if self == i64::MIN {
            i64::MAX
        } else if self == i64::MAX {
            i64::MIN
        } else {
            -self
        }
    }

    #[inline]
    fn tighten(self) -> Self {
        // For tight closure: 2 * floor(x / 2)
        // This ensures diagonal entries m[2i,2i+1] are even
        if self == i64::MAX || self == i64::MIN {
            return self;
        }
        let h = self.half();
        h.saturating_mul(2)
    }

    #[inline]
    fn closure_add(self, other: Self) -> Self {
        // For i64, closure_add is identical to add (saturating arithmetic)
        if self == i64::MAX || other == i64::MAX {
            return i64::MAX;
        }
        if self == i64::MIN || other == i64::MIN {
            return i64::MIN;
        }
        self.saturating_add(other)
    }

    #[inline]
    fn from_i64(val: i64) -> Self {
        val
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // f64 Bound tests
    // =========================================================================

    #[test]
    fn test_bound_f64_infinity() {
        // f64::INFINITY serves as the +infinity sentinel
        let inf = <f64 as Bound>::infinity();
        assert_eq!(inf, f64::INFINITY);
        assert!(inf.is_pos_infinity());
        assert!(!inf.is_neg_infinity());
        assert!(inf > 1e308);
    }

    #[test]
    fn test_bound_f64_neg_infinity() {
        // f64::NEG_INFINITY serves as the -infinity sentinel
        let neg_inf = <f64 as Bound>::neg_infinity();
        assert_eq!(neg_inf, f64::NEG_INFINITY);
        assert!(neg_inf.is_neg_infinity());
        assert!(!neg_inf.is_pos_infinity());
        assert!(neg_inf < -1e308);
    }

    #[test]
    fn test_bound_f64_zero() {
        let z = <f64 as Bound>::zero();
        assert_eq!(z, 0.0);
    }

    #[test]
    fn test_bound_add_sound_rounding() {
        // Sound rounding: a + b should use next_up() for over-approximation.
        // For now the stub does plain addition; the test defines the TARGET behavior:
        // after proper implementation, (0.1 + 0.2).add() should be >= 0.1 + 0.2
        // (i.e., rounded UP, not truncated).
        let a: f64 = 0.1;
        let b: f64 = 0.2;
        let result = a.add(b);
        // Target behavior: result >= a + b (sound over-approximation via next_up)
        // The exact mathematical sum 0.1 + 0.2 in f64 is 0.30000000000000004
        // With next_up, result should be strictly greater than the plain f64 sum.
        let plain_sum = 0.1_f64 + 0.2_f64;
        assert!(
            result >= plain_sum,
            "Sound rounding: add({a}, {b}) = {result} must be >= plain sum {plain_sum}"
        );
        // After proper implementation with next_up, this should be strictly greater:
        assert!(
            result > plain_sum,
            "Sound rounding with next_up: add({a}, {b}) = {result} must be > plain sum {plain_sum} \
             (next_up provides strict over-approximation)"
        );
    }

    #[test]
    fn test_bound_add_infinity_propagation() {
        // +infinity + x = +infinity
        let inf = <f64 as Bound>::infinity();
        assert!(inf.add(5.0).is_pos_infinity());
        assert!(Bound::add(5.0_f64, inf).is_pos_infinity());
        assert!(inf.add(inf).is_pos_infinity());
    }

    #[test]
    fn test_bound_f64_min_max() {
        assert_eq!(Bound::min(3.0_f64, 5.0_f64), 3.0);
        assert_eq!(Bound::max(3.0_f64, 5.0_f64), 5.0);
        assert_eq!(Bound::min(f64::INFINITY, 5.0), 5.0);
        assert_eq!(Bound::max(f64::NEG_INFINITY, 5.0), 5.0);
    }

    #[test]
    fn test_bound_f64_half() {
        assert_eq!(Bound::half(10.0_f64), 5.0);
        assert_eq!(Bound::half(0.0_f64), 0.0);
        assert_eq!(Bound::half(-6.0_f64), -3.0);
    }

    #[test]
    fn test_bound_f64_neg() {
        assert_eq!(Bound::neg(5.0_f64), -5.0);
        assert_eq!(Bound::neg(-3.0_f64), 3.0);
        assert!(Bound::neg(f64::INFINITY).is_neg_infinity());
        assert!(Bound::neg(f64::NEG_INFINITY).is_pos_infinity());
    }

    #[test]
    fn test_bound_f64_tighten_noop() {
        // Tighten is a no-op for floats
        assert_eq!(Bound::tighten(5.0_f64), 5.0);
        assert_eq!(Bound::tighten(3.7_f64), 3.7);
    }

    // =========================================================================
    // i64 Bound tests
    // =========================================================================

    #[test]
    fn test_bound_i64_infinity() {
        let inf = <i64 as Bound>::infinity();
        assert_eq!(inf, i64::MAX);
        assert!(inf.is_pos_infinity());
    }

    #[test]
    fn test_bound_i64_saturation() {
        // Saturating arithmetic: overflow should not panic
        let a: i64 = i64::MAX - 1;
        let b: i64 = 5;
        let result = a.add(b);
        assert!(
            result.is_pos_infinity(),
            "Overflow in i64 addition should saturate to infinity (i64::MAX); got {result}"
        );
    }

    #[test]
    fn test_bound_i64_overflow_to_infinity() {
        // Large values that would overflow should map to +infinity
        let big = i64::MAX / 2 + 1;
        let result = big.add(big);
        assert!(
            result.is_pos_infinity(),
            "i64 addition overflow must yield +infinity (i64::MAX); got {result}"
        );
    }

    #[test]
    fn test_bound_i64_normal_add() {
        assert_eq!(Bound::add(3_i64, 5_i64), 8);
        assert_eq!(Bound::add(-3_i64, 5_i64), 2);
        assert_eq!(Bound::add(0_i64, 0_i64), 0);
    }

    #[test]
    fn test_bound_i64_neg() {
        assert_eq!(Bound::neg(5_i64), -5);
        assert_eq!(Bound::neg(-3_i64), 3);
        // i64::MIN negation -> i64::MAX (infinity)
        assert_eq!(Bound::neg(i64::MIN), i64::MAX);
        assert_eq!(Bound::neg(i64::MAX), i64::MIN);
    }

    #[test]
    fn test_bound_i64_half_floor() {
        assert_eq!(Bound::half(10_i64), 5);
        assert_eq!(Bound::half(11_i64), 5); // floor(11/2) = 5
        assert_eq!(Bound::half(-5_i64), -3); // floor(-5/2) = -3
        assert_eq!(Bound::half(-6_i64), -3); // floor(-6/2) = -3 (exact, but using floor formula)
        assert_eq!(Bound::half(0_i64), 0);
    }

    #[test]
    fn test_bound_i64_tighten() {
        // tighten(x) = 2 * floor(x / 2) -- makes value even (for tight closure)
        assert_eq!(Bound::tighten(5_i64), 4); // 2 * floor(5/2) = 2*2 = 4
        assert_eq!(Bound::tighten(6_i64), 6); // 2 * floor(6/2) = 2*3 = 6
        assert_eq!(Bound::tighten(0_i64), 0);
        assert_eq!(Bound::tighten(-3_i64), -4); // 2 * floor(-3/2) = 2*(-2) = -4
    }

    #[test]
    fn test_bound_i64_min_max() {
        assert_eq!(Bound::min(3_i64, 5_i64), 3);
        assert_eq!(Bound::max(3_i64, 5_i64), 5);
        assert_eq!(Bound::min(i64::MAX, 5_i64), 5);
        assert_eq!(Bound::max(i64::MIN, 5_i64), 5);
    }
}
