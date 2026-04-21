//! Guard Condition Parser and Interval Narrowing
//!
//! Parses branch condition strings from [`CfgEdge::condition`] into structured
//! [`GuardCondition`] values and applies them to narrow abstract values along
//! true/false branches.
//!
//! ## Parsing (Phase 5.1)
//!
//! The parser handles:
//! - Comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`
//! - Reversed operand order: `0 == x`, `5 < x`
//! - Null checks across languages: Python (`is None`/`is not None`), TypeScript
//!   (`=== null`/`!== null`), Go (`== nil`/`!= nil`), Rust (`.is_some()`/`.is_none()`),
//!   C/C++ (`!= nullptr`/`!= NULL`)
//! - Truthiness: bare identifiers, `!x`, `not x`
//! - Negative integer literals: `x > -1`, `x == -5`
//!
//! Unparseable conditions return `None` (conservative: no narrowing applied).
//!
//! ## Narrowing (Phase 5.2)
//!
//! [`narrow_value`] applies a guard condition to an [`AbstractValue`] to produce
//! a tighter (subset) abstract value. [`narrow_state`] applies narrowing to the
//! relevant variable in an [`AbstractState`].
//!
//! Soundness invariant: narrowing never removes concrete values that could
//! actually occur. When in doubt, the result is conservative (unchanged).

use super::abstract_interp::{AbstractState, AbstractValue, Nullability};

/// A parsed guard condition from a CFG branch edge.
///
/// Represents the structured form of a condition string such as `"x > 0"` or
/// `"x is not None"`. Used by guard-aware transfer functions to narrow abstract
/// values along true/false branches.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardCondition {
    /// `x == value` (equality comparison)
    Eq {
        /// The variable being compared.
        var: String,
        /// The integer constant on the right-hand side.
        value: i64,
    },
    /// `x != value` (inequality comparison)
    Neq {
        /// The variable being compared.
        var: String,
        /// The integer constant on the right-hand side.
        value: i64,
    },
    /// `x < value` (strictly less than)
    Lt {
        /// The variable being compared.
        var: String,
        /// The upper bound (exclusive).
        value: i64,
    },
    /// `x <= value` (less than or equal)
    Le {
        /// The variable being compared.
        var: String,
        /// The upper bound (inclusive).
        value: i64,
    },
    /// `x > value` (strictly greater than)
    Gt {
        /// The variable being compared.
        var: String,
        /// The lower bound (exclusive).
        value: i64,
    },
    /// `x >= value` (greater than or equal)
    Ge {
        /// The variable being compared.
        var: String,
        /// The lower bound (inclusive).
        value: i64,
    },
    /// Variable is not null.
    ///
    /// Recognized patterns:
    /// - Python: `x is not None`
    /// - TypeScript/JS: `x !== null`, `x != null`
    /// - Go/Ruby: `x != nil`
    /// - Rust: `x.is_some()`
    /// - C/C++: `x != nullptr`, `x != NULL`
    NotNull {
        /// The variable being checked for non-nullness.
        var: String,
    },
    /// Variable is null.
    ///
    /// Recognized patterns:
    /// - Python: `x is None`
    /// - TypeScript/JS: `x === null`, `x == null`
    /// - Go/Ruby: `x == nil`
    /// - Rust: `x.is_none()`
    IsNull {
        /// The variable being checked for nullness.
        var: String,
    },
    /// Truthiness check: bare variable `x` evaluates as truthy.
    Truthy {
        /// The variable being evaluated for truthiness.
        var: String,
    },
    /// Falsiness check: `!x` or `not x` evaluates as falsy.
    Falsy {
        /// The variable being evaluated for falsiness.
        var: String,
    },
}

/// Check whether a string is a valid simple identifier.
///
/// A valid identifier starts with an ASCII letter or underscore, followed by
/// zero or more ASCII alphanumeric characters or underscores. Does not accept
/// dotted paths (e.g., `obj.field`).
fn is_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Parse an integer literal, including negative values like `-5`.
fn parse_int_literal(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

/// Flip a comparison operator to its mirror.
///
/// When the constant is on the left side (`5 < x`), we flip the operator
/// to normalize to variable-on-left form (`x > 5`).
fn flip_operator(op: &str) -> Option<&'static str> {
    match op {
        "==" => Some("=="),
        "!=" => Some("!="),
        "<" => Some(">"),
        "<=" => Some(">="),
        ">" => Some("<"),
        ">=" => Some("<="),
        _ => None,
    }
}

/// Parse a condition string from [`CfgEdge::condition`] into a structured
/// [`GuardCondition`].
///
/// Returns `None` for unparseable conditions (conservative: no narrowing).
/// Follows the same text-level string-parsing approach as `parse_rhs_abstract`.
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::dataflow::guard::parse_guard_condition;
///
/// let cond = parse_guard_condition("x > 0");
/// assert_eq!(cond, Some(GuardCondition::Gt { var: "x".into(), value: 0 }));
///
/// let cond = parse_guard_condition("x is not None");
/// assert_eq!(cond, Some(GuardCondition::NotNull { var: "x".into() }));
///
/// let cond = parse_guard_condition("f(x)");
/// assert_eq!(cond, None); // function call, can't parse
/// ```
pub fn parse_guard_condition(condition: &str) -> Option<GuardCondition> {
    let s = condition.trim();

    // Empty or whitespace-only
    if s.is_empty() {
        return None;
    }

    // --- Rust method-call null checks: x.is_some() / x.is_none() ---
    if let Some(rest) = s.strip_suffix(".is_some()") {
        let var = rest.trim();
        if is_identifier(var) {
            return Some(GuardCondition::NotNull {
                var: var.to_string(),
            });
        }
    }
    if let Some(rest) = s.strip_suffix(".is_none()") {
        let var = rest.trim();
        if is_identifier(var) {
            return Some(GuardCondition::IsNull {
                var: var.to_string(),
            });
        }
    }

    // --- Python-style null checks: "x is not None" / "x is None" ---
    if let Some(var_part) = s.strip_suffix(" is not None") {
        let var = var_part.trim();
        if is_identifier(var) {
            return Some(GuardCondition::NotNull {
                var: var.to_string(),
            });
        }
    }
    if let Some(var_part) = s.strip_suffix(" is None") {
        let var = var_part.trim();
        if is_identifier(var) {
            return Some(GuardCondition::IsNull {
                var: var.to_string(),
            });
        }
    }

    // --- Python-style falsy: "not x" ---
    if let Some(rest) = s.strip_prefix("not ") {
        let var = rest.trim();
        if is_identifier(var) {
            return Some(GuardCondition::Falsy {
                var: var.to_string(),
            });
        }
        // "not" followed by something non-identifier: unparseable
        return None;
    }

    // --- Comparison operators (two-operand) ---
    // Try splitting on two-char operators first, then single-char.
    // Order matters: check `!==`, `===`, `>=`, `<=`, `!=`, `==` before `<`, `>`.
    let two_char_ops = ["!==", "===", ">=", "<=", "!=", "=="];
    let one_char_ops = [">", "<"];

    for &op in &two_char_ops {
        if let Some(result) = try_comparison_split(s, op) {
            return Some(result);
        }
    }
    for &op in &one_char_ops {
        if let Some(result) = try_comparison_split(s, op) {
            return Some(result);
        }
    }

    // --- C-style falsy: "!x" ---
    if let Some(rest) = s.strip_prefix('!') {
        let var = rest.trim();
        if is_identifier(var) {
            return Some(GuardCondition::Falsy {
                var: var.to_string(),
            });
        }
        return None;
    }

    // --- Bare identifier → Truthy ---
    if is_identifier(s) {
        return Some(GuardCondition::Truthy {
            var: s.to_string(),
        });
    }

    // --- Unparseable ---
    None
}

/// Null-like keywords recognized across languages.
const NULL_KEYWORDS: &[&str] = &["null", "nil", "nullptr", "NULL", "None"];

/// Try splitting a condition on a comparison operator and parse both sides.
///
/// Handles both `var op literal` and `literal op var` (reversed) forms.
/// For null-keyword comparisons, produces `IsNull`/`NotNull` variants.
fn try_comparison_split(s: &str, op: &str) -> Option<GuardCondition> {
    // Find the operator in the string. We need to be careful with substrings:
    // "!==" contains "!=", so we try longer operators first in the caller.
    let idx = s.find(op)?;

    let lhs = s[..idx].trim();
    let rhs = s[idx + op.len()..].trim();

    if lhs.is_empty() || rhs.is_empty() {
        return None;
    }

    // Determine the "canonical" operator for null checks.
    // `===` and `!==` map to `==` and `!=` semantically for null comparisons.
    let canonical_op = match op {
        "===" => "==",
        "!==" => "!=",
        other => other,
    };

    // Case 1: var <op> null_keyword
    if is_identifier(lhs) && NULL_KEYWORDS.contains(&rhs) {
        return match canonical_op {
            "==" => Some(GuardCondition::IsNull {
                var: lhs.to_string(),
            }),
            "!=" => Some(GuardCondition::NotNull {
                var: lhs.to_string(),
            }),
            _ => None, // `x < null` doesn't make sense
        };
    }

    // Case 2: null_keyword <op> var (reversed null check)
    if NULL_KEYWORDS.contains(&lhs) && is_identifier(rhs) {
        return match canonical_op {
            "==" => Some(GuardCondition::IsNull {
                var: rhs.to_string(),
            }),
            "!=" => Some(GuardCondition::NotNull {
                var: rhs.to_string(),
            }),
            _ => None,
        };
    }

    // Case 3: var <op> int_literal
    if is_identifier(lhs) {
        if let Some(value) = parse_int_literal(rhs) {
            return make_guard(lhs, canonical_op, value);
        }
    }

    // Case 4: int_literal <op> var (reversed operand order)
    if is_identifier(rhs) {
        if let Some(value) = parse_int_literal(lhs) {
            // Flip the operator: `5 < x` means `x > 5`
            let flipped = flip_operator(canonical_op)?;
            return make_guard(rhs, flipped, value);
        }
    }

    None
}

/// Construct a [`GuardCondition`] from a variable name, canonical operator, and
/// integer value.
fn make_guard(var: &str, op: &str, value: i64) -> Option<GuardCondition> {
    let var = var.to_string();
    match op {
        "==" => Some(GuardCondition::Eq { var, value }),
        "!=" => Some(GuardCondition::Neq { var, value }),
        "<" => Some(GuardCondition::Lt { var, value }),
        "<=" => Some(GuardCondition::Le { var, value }),
        ">" => Some(GuardCondition::Gt { var, value }),
        ">=" => Some(GuardCondition::Ge { var, value }),
        _ => None,
    }
}

/// Extract the variable name from a [`GuardCondition`].
///
/// Every guard condition variant contains a `var` field identifying the
/// variable being tested. This helper extracts it.
pub fn guard_variable(guard: &GuardCondition) -> &str {
    match guard {
        GuardCondition::Eq { var, .. }
        | GuardCondition::Neq { var, .. }
        | GuardCondition::Lt { var, .. }
        | GuardCondition::Le { var, .. }
        | GuardCondition::Gt { var, .. }
        | GuardCondition::Ge { var, .. }
        | GuardCondition::NotNull { var }
        | GuardCondition::IsNull { var }
        | GuardCondition::Truthy { var }
        | GuardCondition::Falsy { var } => var.as_str(),
    }
}

/// Narrow an [`AbstractValue`] based on a guard condition and branch polarity.
///
/// `is_true_branch`: `true` means we are on the branch where the condition
/// holds, `false` means we are on the branch where it does NOT hold.
///
/// Returns a narrowed `AbstractValue` that is always a **subset** of the input
/// (soundness invariant: never removes concrete values that could actually
/// occur).
///
/// If narrowing produces an empty range (contradiction), returns
/// [`AbstractValue::bottom()`].
pub fn narrow_value(
    value: &AbstractValue,
    guard: &GuardCondition,
    is_true_branch: bool,
) -> AbstractValue {
    if !is_true_branch {
        // FALSE branch: negate the guard and apply as TRUE branch
        let complement = negate_guard(guard);
        return narrow_value(value, &complement, true);
    }

    // TRUE branch narrowing
    let mut result = value.clone();

    match guard {
        GuardCondition::Eq { value: c, .. } => {
            // x == c: range becomes [c, c] intersected with input
            result.range_ = intersect_range(value.range_, Some((Some(*c), Some(*c))));
            // Clear constant since we are narrowing, not propagating
        }
        GuardCondition::Neq { value: c, .. } => {
            // x != c: narrow when c is at a boundary of the range.
            //
            // Cases handled:
            // 1. Range is exactly [c, c] -> contradiction (bottom)
            // 2. c is the lower bound -> tighten to [c+1, hi]
            // 3. c is the upper bound -> tighten to [lo, c-1]
            // 4. c is interior to range or no range -> conservative (can't split)
            match value.range_ {
                Some((Some(lo), Some(hi))) if lo == *c && hi == *c => {
                    return AbstractValue::bottom();
                }
                Some((Some(lo), hi)) if lo == *c => {
                    if let Some(new_lo) = c.checked_add(1) {
                        result.range_ = Some((Some(new_lo), hi));
                    }
                }
                Some((lo, Some(hi))) if hi == *c => {
                    if let Some(new_hi) = c.checked_sub(1) {
                        result.range_ = Some((lo, Some(new_hi)));
                    }
                }
                _ => {
                    // c is interior to range or no range info: can't split
                    // intervals, leave unchanged (conservative).
                }
            }
        }
        GuardCondition::Lt { value: c, .. } => {
            // x < c: upper bound is c-1 (integer semantics)
            let upper = c.checked_sub(1);
            let guard_range = match upper {
                Some(u) => Some((None, Some(u))),
                None => {
                    // c is i64::MIN, c-1 overflows -> empty (nothing < i64::MIN)
                    return AbstractValue::bottom();
                }
            };
            result.range_ = intersect_range(value.range_, guard_range);
        }
        GuardCondition::Le { value: c, .. } => {
            // x <= c: upper bound is c
            let guard_range = Some((None, Some(*c)));
            result.range_ = intersect_range(value.range_, guard_range);
        }
        GuardCondition::Gt { value: c, .. } => {
            // x > c: lower bound is c+1
            let lower = c.checked_add(1);
            let guard_range = match lower {
                Some(l) => Some((Some(l), None)),
                None => {
                    // c is i64::MAX, c+1 overflows -> empty (nothing > i64::MAX)
                    return AbstractValue::bottom();
                }
            };
            result.range_ = intersect_range(value.range_, guard_range);
        }
        GuardCondition::Ge { value: c, .. } => {
            // x >= c: lower bound is c
            let guard_range = Some((Some(*c), None));
            result.range_ = intersect_range(value.range_, guard_range);
        }
        GuardCondition::NotNull { .. } => {
            result.nullable = Nullability::Never;
        }
        GuardCondition::IsNull { .. } => {
            result.nullable = Nullability::Always;
        }
        GuardCondition::Truthy { .. } => {
            // Truthy: not null AND not zero
            result.nullable = Nullability::Never;
            // Try to exclude 0 from range
            result.range_ = exclude_zero(value.range_);
        }
        GuardCondition::Falsy { .. } => {
            // Falsy: could be null OR zero. Conservative: don't narrow range,
            // set nullable to Maybe (it could be null).
            result.nullable = Nullability::Maybe;
        }
    }

    // Check for bottom after range narrowing
    if is_empty_range(result.range_) {
        return AbstractValue::bottom();
    }

    result
}

/// Narrow an entire [`AbstractState`] by applying a guard condition.
///
/// Only narrows the variable mentioned in the guard condition. Other variables
/// are passed through unchanged. If the variable is not in the state, the
/// state is returned unchanged.
pub fn narrow_state(
    state: &AbstractState,
    guard: &GuardCondition,
    is_true_branch: bool,
) -> AbstractState {
    let var = guard_variable(guard);
    let current = state.get(var);
    let narrowed = narrow_value(&current, guard, is_true_branch);
    state.set(var, narrowed)
}

/// Negate a guard condition (compute its complement).
///
/// Used for FALSE-branch narrowing: if we know the condition does NOT hold,
/// we apply the negated condition as if it DOES hold.
fn negate_guard(guard: &GuardCondition) -> GuardCondition {
    match guard {
        GuardCondition::Eq { var, value } => GuardCondition::Neq {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::Neq { var, value } => GuardCondition::Eq {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::Lt { var, value } => GuardCondition::Ge {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::Le { var, value } => GuardCondition::Gt {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::Gt { var, value } => GuardCondition::Le {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::Ge { var, value } => GuardCondition::Lt {
            var: var.clone(),
            value: *value,
        },
        GuardCondition::NotNull { var } => GuardCondition::IsNull { var: var.clone() },
        GuardCondition::IsNull { var } => GuardCondition::NotNull { var: var.clone() },
        GuardCondition::Truthy { var } => GuardCondition::Falsy { var: var.clone() },
        GuardCondition::Falsy { var } => GuardCondition::Truthy { var: var.clone() },
    }
}

/// Intersect two ranges, producing their overlap.
///
/// Each range is `Option<(Option<i64>, Option<i64>)>` where:
/// - Outer `None` means "no range info" (treated as unbounded `(-inf, +inf)`)
/// - Inner `None` for lo/hi means unbounded in that direction
///
/// Returns `None` only if both inputs are `None`. Otherwise returns
/// `Some((lo, hi))` with the tighter of the two bounds.
fn intersect_range(
    a: Option<(Option<i64>, Option<i64>)>,
    b: Option<(Option<i64>, Option<i64>)>,
) -> Option<(Option<i64>, Option<i64>)> {
    match (a, b) {
        (None, None) => None,
        (None, Some(r)) | (Some(r), None) => Some(r),
        (Some((a_lo, a_hi)), Some((b_lo, b_hi))) => {
            let lo = match (a_lo, b_lo) {
                (None, None) => None,
                (Some(v), None) | (None, Some(v)) => Some(v),
                (Some(a), Some(b)) => Some(a.max(b)),
            };
            let hi = match (a_hi, b_hi) {
                (None, None) => None,
                (Some(v), None) | (None, Some(v)) => Some(v),
                (Some(a), Some(b)) => Some(a.min(b)),
            };
            Some((lo, hi))
        }
    }
}

/// Check if a range is empty (lo > hi).
///
/// An empty range represents a contradiction (unreachable state).
/// Unbounded sides (None) are never empty.
fn is_empty_range(range: Option<(Option<i64>, Option<i64>)>) -> bool {
    match range {
        None => false,
        Some((Some(lo), Some(hi))) => lo > hi,
        _ => false, // unbounded on either side -> not empty
    }
}

/// Exclude zero from a range if possible.
///
/// For `Truthy` narrowing: the value is known to be nonzero.
/// - If range is exactly [0, 0], returns bottom-range (empty).
/// - If lo == 0, bumps lo to 1.
/// - If hi == 0, bumps hi to -1.
/// - Otherwise returns unchanged (conservative: zero may be interior).
fn exclude_zero(range: Option<(Option<i64>, Option<i64>)>) -> Option<(Option<i64>, Option<i64>)> {
    match range {
        None => None, // no range info, can't narrow
        Some((lo, hi)) => {
            let lo_val = lo.unwrap_or(i64::MIN);
            let hi_val = hi.unwrap_or(i64::MAX);

            if lo_val == 0 && hi_val == 0 {
                // Exactly [0, 0] -> empty (contradiction handled by caller)
                Some((Some(1), Some(0))) // empty range: lo > hi
            } else if lo_val == 0 {
                // [0, hi] -> [1, hi]
                Some((Some(1), hi))
            } else if hi_val == 0 {
                // [lo, 0] -> [lo, -1]
                Some((lo, Some(-1)))
            } else {
                // Zero is interior to range, can't split intervals -> conservative
                Some((lo, hi))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Comparison operators with positive values
    // =========================================================================

    #[test]
    fn test_eq_positive() {
        assert_eq!(
            parse_guard_condition("x == 5"),
            Some(GuardCondition::Eq {
                var: "x".into(),
                value: 5
            })
        );
    }

    #[test]
    fn test_neq_positive() {
        assert_eq!(
            parse_guard_condition("x != 0"),
            Some(GuardCondition::Neq {
                var: "x".into(),
                value: 0
            })
        );
    }

    #[test]
    fn test_lt_positive() {
        assert_eq!(
            parse_guard_condition("x < 10"),
            Some(GuardCondition::Lt {
                var: "x".into(),
                value: 10
            })
        );
    }

    #[test]
    fn test_le_positive() {
        assert_eq!(
            parse_guard_condition("x <= 100"),
            Some(GuardCondition::Le {
                var: "x".into(),
                value: 100
            })
        );
    }

    #[test]
    fn test_gt_positive() {
        assert_eq!(
            parse_guard_condition("x > 0"),
            Some(GuardCondition::Gt {
                var: "x".into(),
                value: 0
            })
        );
    }

    #[test]
    fn test_ge_positive() {
        assert_eq!(
            parse_guard_condition("x >= 1"),
            Some(GuardCondition::Ge {
                var: "x".into(),
                value: 1
            })
        );
    }

    // =========================================================================
    // Comparison operators with negative values
    // =========================================================================

    #[test]
    fn test_eq_negative() {
        assert_eq!(
            parse_guard_condition("x == -5"),
            Some(GuardCondition::Eq {
                var: "x".into(),
                value: -5
            })
        );
    }

    #[test]
    fn test_neq_negative() {
        assert_eq!(
            parse_guard_condition("x != -1"),
            Some(GuardCondition::Neq {
                var: "x".into(),
                value: -1
            })
        );
    }

    #[test]
    fn test_lt_negative() {
        assert_eq!(
            parse_guard_condition("x < -3"),
            Some(GuardCondition::Lt {
                var: "x".into(),
                value: -3
            })
        );
    }

    #[test]
    fn test_le_negative() {
        assert_eq!(
            parse_guard_condition("x <= -10"),
            Some(GuardCondition::Le {
                var: "x".into(),
                value: -10
            })
        );
    }

    #[test]
    fn test_gt_negative() {
        assert_eq!(
            parse_guard_condition("x > -1"),
            Some(GuardCondition::Gt {
                var: "x".into(),
                value: -1
            })
        );
    }

    #[test]
    fn test_ge_negative() {
        assert_eq!(
            parse_guard_condition("x >= -100"),
            Some(GuardCondition::Ge {
                var: "x".into(),
                value: -100
            })
        );
    }

    // =========================================================================
    // Reversed operand order (constant on left)
    // =========================================================================

    #[test]
    fn test_reversed_eq() {
        // 0 == x  →  x == 0
        assert_eq!(
            parse_guard_condition("0 == x"),
            Some(GuardCondition::Eq {
                var: "x".into(),
                value: 0
            })
        );
    }

    #[test]
    fn test_reversed_neq() {
        // 5 != x  →  x != 5
        assert_eq!(
            parse_guard_condition("5 != x"),
            Some(GuardCondition::Neq {
                var: "x".into(),
                value: 5
            })
        );
    }

    #[test]
    fn test_reversed_lt() {
        // 5 < x  →  x > 5
        assert_eq!(
            parse_guard_condition("5 < x"),
            Some(GuardCondition::Gt {
                var: "x".into(),
                value: 5
            })
        );
    }

    #[test]
    fn test_reversed_le() {
        // 5 <= x  →  x >= 5
        assert_eq!(
            parse_guard_condition("5 <= x"),
            Some(GuardCondition::Ge {
                var: "x".into(),
                value: 5
            })
        );
    }

    #[test]
    fn test_reversed_gt() {
        // 10 > x  →  x < 10
        assert_eq!(
            parse_guard_condition("10 > x"),
            Some(GuardCondition::Lt {
                var: "x".into(),
                value: 10
            })
        );
    }

    #[test]
    fn test_reversed_ge() {
        // 10 >= x  →  x <= 10
        assert_eq!(
            parse_guard_condition("10 >= x"),
            Some(GuardCondition::Le {
                var: "x".into(),
                value: 10
            })
        );
    }

    // =========================================================================
    // Python null checks
    // =========================================================================

    #[test]
    fn test_python_is_not_none() {
        assert_eq!(
            parse_guard_condition("x is not None"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_python_is_none() {
        assert_eq!(
            parse_guard_condition("x is None"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // TypeScript/JS null checks
    // =========================================================================

    #[test]
    fn test_ts_strict_not_null() {
        assert_eq!(
            parse_guard_condition("x !== null"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_ts_strict_is_null() {
        assert_eq!(
            parse_guard_condition("x === null"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_ts_loose_not_null() {
        assert_eq!(
            parse_guard_condition("x != null"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_ts_loose_is_null() {
        assert_eq!(
            parse_guard_condition("x == null"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // Go/Ruby null checks
    // =========================================================================

    #[test]
    fn test_go_not_nil() {
        assert_eq!(
            parse_guard_condition("x != nil"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_go_is_nil() {
        assert_eq!(
            parse_guard_condition("x == nil"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // Rust null checks
    // =========================================================================

    #[test]
    fn test_rust_is_some() {
        assert_eq!(
            parse_guard_condition("x.is_some()"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_rust_is_none() {
        assert_eq!(
            parse_guard_condition("x.is_none()"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // C/C++ null checks
    // =========================================================================

    #[test]
    fn test_cpp_not_nullptr() {
        assert_eq!(
            parse_guard_condition("x != nullptr"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_c_not_null_macro() {
        assert_eq!(
            parse_guard_condition("x != NULL"),
            Some(GuardCondition::NotNull {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_c_is_null_macro() {
        assert_eq!(
            parse_guard_condition("x == NULL"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // Truthiness / Falsiness
    // =========================================================================

    #[test]
    fn test_truthy_bare_identifier() {
        assert_eq!(
            parse_guard_condition("x"),
            Some(GuardCondition::Truthy {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_falsy_bang() {
        assert_eq!(
            parse_guard_condition("!x"),
            Some(GuardCondition::Falsy {
                var: "x".into()
            })
        );
    }

    #[test]
    fn test_falsy_not_keyword() {
        assert_eq!(
            parse_guard_condition("not x"),
            Some(GuardCondition::Falsy {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // Edge cases
    // =========================================================================

    #[test]
    fn test_empty_string() {
        assert_eq!(parse_guard_condition(""), None);
    }

    #[test]
    fn test_whitespace_only() {
        assert_eq!(parse_guard_condition("   "), None);
    }

    #[test]
    fn test_function_call_unparseable() {
        assert_eq!(parse_guard_condition("f(x)"), None);
    }

    #[test]
    fn test_compound_condition_unparseable() {
        assert_eq!(parse_guard_condition("x > 0 && x < 10"), None);
    }

    #[test]
    fn test_variable_with_underscores() {
        assert_eq!(
            parse_guard_condition("my_var > 0"),
            Some(GuardCondition::Gt {
                var: "my_var".into(),
                value: 0
            })
        );
    }

    #[test]
    fn test_variable_with_dots_unparseable() {
        // Dotted paths like obj.field are not simple identifiers
        assert_eq!(parse_guard_condition("obj.field > 0"), None);
    }

    #[test]
    fn test_negative_literal_gt() {
        assert_eq!(
            parse_guard_condition("x > -1"),
            Some(GuardCondition::Gt {
                var: "x".into(),
                value: -1
            })
        );
    }

    #[test]
    fn test_negative_literal_eq() {
        assert_eq!(
            parse_guard_condition("x == -5"),
            Some(GuardCondition::Eq {
                var: "x".into(),
                value: -5
            })
        );
    }

    #[test]
    fn test_whitespace_around_operator() {
        assert_eq!(
            parse_guard_condition("  x  ==  5  "),
            Some(GuardCondition::Eq {
                var: "x".into(),
                value: 5
            })
        );
    }

    #[test]
    fn test_multiword_identifier_truthy() {
        assert_eq!(
            parse_guard_condition("is_ready"),
            Some(GuardCondition::Truthy {
                var: "is_ready".into()
            })
        );
    }

    #[test]
    fn test_cpp_nullptr_is_null() {
        assert_eq!(
            parse_guard_condition("x == nullptr"),
            Some(GuardCondition::IsNull {
                var: "x".into()
            })
        );
    }

    // =========================================================================
    // is_identifier helper
    // =========================================================================

    #[test]
    fn test_is_identifier_valid() {
        assert!(is_identifier("x"));
        assert!(is_identifier("my_var"));
        assert!(is_identifier("_private"));
        assert!(is_identifier("camelCase"));
        assert!(is_identifier("x123"));
    }

    #[test]
    fn test_is_identifier_invalid() {
        assert!(!is_identifier(""));
        assert!(!is_identifier("123abc"));
        assert!(!is_identifier("a.b"));
        assert!(!is_identifier("a b"));
        assert!(!is_identifier("a-b"));
        assert!(!is_identifier("f(x)"));
    }

    // =========================================================================
    // Phase 5.2: guard_variable tests
    // =========================================================================

    #[test]
    fn test_guard_variable_eq() {
        let g = GuardCondition::Eq {
            var: "x".into(),
            value: 5,
        };
        assert_eq!(guard_variable(&g), "x");
    }

    #[test]
    fn test_guard_variable_neq() {
        let g = GuardCondition::Neq {
            var: "count".into(),
            value: 0,
        };
        assert_eq!(guard_variable(&g), "count");
    }

    #[test]
    fn test_guard_variable_lt() {
        let g = GuardCondition::Lt {
            var: "idx".into(),
            value: 10,
        };
        assert_eq!(guard_variable(&g), "idx");
    }

    #[test]
    fn test_guard_variable_not_null() {
        let g = GuardCondition::NotNull {
            var: "ptr".into(),
        };
        assert_eq!(guard_variable(&g), "ptr");
    }

    #[test]
    fn test_guard_variable_truthy() {
        let g = GuardCondition::Truthy {
            var: "flag".into(),
        };
        assert_eq!(guard_variable(&g), "flag");
    }

    #[test]
    fn test_guard_variable_falsy() {
        let g = GuardCondition::Falsy {
            var: "done".into(),
        };
        assert_eq!(guard_variable(&g), "done");
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Eq guard
    // =========================================================================

    #[test]
    fn test_narrow_eq_true_unbounded() {
        // Eq(x, 5) TRUE on unbounded range -> [5, 5]
        let val = AbstractValue::top();
        let guard = GuardCondition::Eq {
            var: "x".into(),
            value: 5,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_narrow_eq_true_bounded() {
        // Eq(x, 5) TRUE on [0, 10] -> [5, 5]
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Eq {
            var: "x".into(),
            value: 5,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_narrow_eq_true_outside_range() {
        // Eq(x, 15) TRUE on [0, 10] -> bottom (contradiction)
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Eq {
            var: "x".into(),
            value: 15,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    #[test]
    fn test_narrow_eq_false_bounded() {
        // Eq(x, 5) FALSE on [0, 10] -> conservative [0, 10] (can't split)
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Eq {
            var: "x".into(),
            value: 5,
        };
        let result = narrow_value(&val, &guard, false);
        // FALSE branch for Eq -> Neq, which is conservative
        assert_eq!(result.range_, Some((Some(0), Some(10))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Neq guard
    // =========================================================================

    #[test]
    fn test_narrow_neq_true_exact_match() {
        // Neq(x, 0) TRUE on [0, 0] -> bottom (contradiction: x != 0 but x is exactly 0)
        let val = AbstractValue {
            range_: Some((Some(0), Some(0))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Neq {
            var: "x".into(),
            value: 0,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    #[test]
    fn test_narrow_neq_true_wider_range() {
        // Neq(x, 5) TRUE on [0, 10] -> [0, 10] (conservative, can't split)
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Neq {
            var: "x".into(),
            value: 5,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(0), Some(10))));
    }

    #[test]
    fn test_narrow_neq_false() {
        // Neq(x, 5) FALSE -> x == 5, so [5, 5]
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Neq {
            var: "x".into(),
            value: 5,
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.range_, Some((Some(5), Some(5))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Lt guard
    // =========================================================================

    #[test]
    fn test_narrow_lt_true() {
        // Lt(x, 10) TRUE on [0, 100] -> [0, 9]
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Lt {
            var: "x".into(),
            value: 10,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(0), Some(9))));
    }

    #[test]
    fn test_narrow_lt_false() {
        // Lt(x, 10) FALSE -> Ge(x, 10) -> [10, 100]
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Lt {
            var: "x".into(),
            value: 10,
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.range_, Some((Some(10), Some(100))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Le guard
    // =========================================================================

    #[test]
    fn test_narrow_le_true() {
        // Le(x, 0) TRUE on [-5, 5] -> [-5, 0]
        let val = AbstractValue {
            range_: Some((Some(-5), Some(5))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Le {
            var: "x".into(),
            value: 0,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(-5), Some(0))));
    }

    #[test]
    fn test_narrow_le_false() {
        // Le(x, 0) FALSE -> Gt(x, 0) -> [1, 5]
        let val = AbstractValue {
            range_: Some((Some(-5), Some(5))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Le {
            var: "x".into(),
            value: 0,
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.range_, Some((Some(1), Some(5))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Gt guard
    // =========================================================================

    #[test]
    fn test_narrow_gt_true_unbounded() {
        // Gt(x, 0) TRUE on unbounded -> [1, +inf]
        let val = AbstractValue::top();
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 0,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(1), None)));
    }

    #[test]
    fn test_narrow_gt_false() {
        // Gt(x, 50) FALSE on [0, 100] -> Le(x, 50) -> [0, 50]
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 50,
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.range_, Some((Some(0), Some(50))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Ge guard
    // =========================================================================

    #[test]
    fn test_narrow_ge_true() {
        // Ge(x, 1) TRUE on [-5, 5] -> [1, 5]
        let val = AbstractValue {
            range_: Some((Some(-5), Some(5))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Ge {
            var: "x".into(),
            value: 1,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(1), Some(5))));
    }

    #[test]
    fn test_narrow_ge_false() {
        // Ge(x, 1) FALSE -> Lt(x, 1) -> [-5, 0]
        let val = AbstractValue {
            range_: Some((Some(-5), Some(5))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Ge {
            var: "x".into(),
            value: 1,
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.range_, Some((Some(-5), Some(0))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Null guards
    // =========================================================================

    #[test]
    fn test_narrow_not_null_true() {
        // NotNull TRUE: nullable Maybe -> Never
        let val = AbstractValue {
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::NotNull {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Never);
    }

    #[test]
    fn test_narrow_not_null_false() {
        // NotNull FALSE -> IsNull TRUE: nullable -> Always
        let val = AbstractValue {
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::NotNull {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.nullable, Nullability::Always);
    }

    #[test]
    fn test_narrow_is_null_true() {
        // IsNull TRUE: nullable Maybe -> Always
        let val = AbstractValue {
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::IsNull {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Always);
    }

    #[test]
    fn test_narrow_is_null_false() {
        // IsNull FALSE -> NotNull TRUE: nullable -> Never
        let val = AbstractValue {
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::IsNull {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, false);
        assert_eq!(result.nullable, Nullability::Never);
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Truthy/Falsy
    // =========================================================================

    #[test]
    fn test_narrow_truthy_true_excludes_zero() {
        // Truthy TRUE: range including 0 at boundary -> exclude 0, nullable -> Never
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Truthy {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Never);
        // 0 was at lower bound, so it should be excluded: [1, 10]
        assert_eq!(result.range_, Some((Some(1), Some(10))));
    }

    #[test]
    fn test_narrow_truthy_true_zero_at_upper() {
        // Truthy TRUE: range [-5, 0] -> exclude 0 at upper bound: [-5, -1]
        let val = AbstractValue {
            range_: Some((Some(-5), Some(0))),
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Truthy {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Never);
        assert_eq!(result.range_, Some((Some(-5), Some(-1))));
    }

    #[test]
    fn test_narrow_truthy_true_zero_only() {
        // Truthy TRUE on [0, 0] -> contradiction -> bottom
        let val = AbstractValue {
            range_: Some((Some(0), Some(0))),
            nullable: Nullability::Maybe,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Truthy {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    #[test]
    fn test_narrow_falsy_true() {
        // Falsy TRUE: conservative (unchanged range), nullable -> Maybe
        let val = AbstractValue {
            range_: Some((Some(0), Some(10))),
            nullable: Nullability::Never,
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Falsy {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Maybe);
        assert_eq!(result.range_, Some((Some(0), Some(10))));
    }

    // =========================================================================
    // Phase 5.2: narrow_value tests — Edge cases
    // =========================================================================

    #[test]
    fn test_narrow_no_range_gt_creates_range() {
        // top() (no range info) with Gt(x, 0) -> creates [1, +inf]
        let val = AbstractValue::top();
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 0,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(1), None)));
    }

    #[test]
    fn test_narrow_empty_intersection_produces_bottom() {
        // [0, 5] with Gt(x, 100) -> empty -> bottom
        let val = AbstractValue {
            range_: Some((Some(0), Some(5))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 100,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    #[test]
    fn test_narrow_soundness_subset() {
        // Narrowed range [max(lo, guard_lo), min(hi, guard_hi)] is always
        // a subset of the input range
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Ge {
            var: "x".into(),
            value: 10,
        };
        let result = narrow_value(&val, &guard, true);
        // Result [10, 100] is subset of input [0, 100]
        let (r_lo, r_hi) = result.range_.unwrap();
        let (i_lo, i_hi) = val.range_.unwrap();
        assert!(r_lo.unwrap() >= i_lo.unwrap());
        assert!(r_hi.unwrap() <= i_hi.unwrap());
    }

    #[test]
    fn test_narrow_gt_i64_max_overflow() {
        // Gt(x, i64::MAX): c+1 overflows -> bottom (nothing > i64::MAX)
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: i64::MAX,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    #[test]
    fn test_narrow_lt_i64_min_overflow() {
        // Lt(x, i64::MIN): c-1 overflows -> bottom (nothing < i64::MIN)
        let val = AbstractValue {
            range_: Some((Some(0), Some(100))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Lt {
            var: "x".into(),
            value: i64::MIN,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result, AbstractValue::bottom());
    }

    // =========================================================================
    // Phase 5.2: narrow_state tests
    // =========================================================================

    #[test]
    fn test_narrow_state_narrows_mentioned_var() {
        // narrow_state only narrows "x", leaves "y" unchanged
        let state = AbstractState::new()
            .set(
                "x",
                AbstractValue {
                    range_: Some((Some(0), Some(100))),
                    ..AbstractValue::top()
                },
            )
            .set(
                "y",
                AbstractValue {
                    range_: Some((Some(-10), Some(10))),
                    ..AbstractValue::top()
                },
            );

        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 50,
        };
        let narrowed = narrow_state(&state, &guard, true);

        // x should be narrowed to [51, 100]
        assert_eq!(narrowed.get("x").range_, Some((Some(51), Some(100))));
        // y should be unchanged
        assert_eq!(narrowed.get("y").range_, Some((Some(-10), Some(10))));
    }

    #[test]
    fn test_narrow_state_missing_variable() {
        // Variable not in state -> state returned (var gets top() then narrowed)
        let state = AbstractState::new().set(
            "y",
            AbstractValue {
                range_: Some((Some(0), Some(10))),
                ..AbstractValue::top()
            },
        );

        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 0,
        };
        let narrowed = narrow_state(&state, &guard, true);

        // y unchanged
        assert_eq!(narrowed.get("y").range_, Some((Some(0), Some(10))));
        // x was top(), now narrowed to [1, +inf]
        assert_eq!(narrowed.get("x").range_, Some((Some(1), None)));
    }

    #[test]
    fn test_narrow_state_null_guard() {
        // narrow_state with NotNull guard updates nullable for that var
        let state = AbstractState::new().set(
            "ptr",
            AbstractValue {
                nullable: Nullability::Maybe,
                ..AbstractValue::top()
            },
        );

        let guard = GuardCondition::NotNull {
            var: "ptr".into(),
        };
        let narrowed = narrow_state(&state, &guard, true);
        assert_eq!(narrowed.get("ptr").nullable, Nullability::Never);
    }

    // =========================================================================
    // Phase 5.2: Additional edge case tests
    // =========================================================================

    #[test]
    fn test_narrow_ge_with_half_bounded_range() {
        // [None, Some(50)] with Ge(x, 10) -> [10, 50]
        let val = AbstractValue {
            range_: Some((None, Some(50))),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Ge {
            var: "x".into(),
            value: 10,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(10), Some(50))));
    }

    #[test]
    fn test_narrow_le_with_half_bounded_range() {
        // [Some(5), None] with Le(x, 20) -> [5, 20]
        let val = AbstractValue {
            range_: Some((Some(5), None)),
            ..AbstractValue::top()
        };
        let guard = GuardCondition::Le {
            var: "x".into(),
            value: 20,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.range_, Some((Some(5), Some(20))));
    }

    #[test]
    fn test_narrow_truthy_no_range_info() {
        // top() with Truthy -> nullable Never, range unchanged (None)
        let val = AbstractValue::top();
        let guard = GuardCondition::Truthy {
            var: "x".into(),
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.nullable, Nullability::Never);
        // No range info, so exclude_zero returns None (conservative)
        assert_eq!(result.range_, None);
    }

    #[test]
    fn test_narrow_preserves_type() {
        // Narrowing should preserve the type_ field
        let val = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(0), Some(100))),
            nullable: Nullability::Never,
            constant: None,
        };
        let guard = GuardCondition::Gt {
            var: "x".into(),
            value: 50,
        };
        let result = narrow_value(&val, &guard, true);
        assert_eq!(result.type_, Some("int".to_string()));
        assert_eq!(result.range_, Some((Some(51), Some(100))));
    }
}
