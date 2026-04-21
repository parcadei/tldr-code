//! HashKey - Structured Keys for GVN Hashing
//!
//! MIT-HASH-01b Mitigation: Use structured enum instead of string concatenation
//! to prevent collision issues like "binop:Add:ab:c" vs "binop:Add:a:bc".
//!
//! # Problem
//!
//! String-based hash keys like `format!("binop:{}:{}:{}", op, left, right)` can
//! cause false collisions when operand names contain the delimiter. For example:
//! - `binop:Add:a:bc` (a + bc)
//! - `binop:Add:ab:c` (ab + c)
//!
//! These would hash differently with strings but could collide if names are
//! manipulated incorrectly.
//!
//! # Solution
//!
//! Use a structured enum that derives Hash, ensuring each component is hashed
//! separately without delimiter confusion.

use std::cmp::Ordering;

/// Structured hash keys for GVN expressions.
///
/// Use this enum instead of string concatenation to ensure collision-free hashing.
/// The enum derives Hash, Eq, and PartialEq, so it can be used directly as a
/// HashMap key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HashKey {
    /// Constant value with type and representation
    Const {
        /// The type of the constant (e.g., "int", "str", "float").
        type_name: String,
        /// The string representation of the constant value (e.g., "42", "hello").
        repr: String,
    },

    /// Variable reference by value number (already resolved)
    VarVN {
        /// The GVN value number assigned to this variable.
        vn: usize,
    },

    /// Unresolved variable name (for parameters and initial references)
    Name {
        /// The source-level variable name before value numbering.
        name: String,
    },

    /// Binary operation with operator and operands
    /// If commutative=true, operands are normalized (sorted)
    BinOp {
        /// The binary operator (e.g., "Add", "Sub", "Mult").
        op: String,
        /// The left operand of the binary expression.
        left: Box<HashKey>,
        /// The right operand of the binary expression.
        right: Box<HashKey>,
        /// Whether the operands have been normalized for commutativity.
        commutative: bool,
    },

    /// Unary operation (e.g., -x, not x, ~x)
    UnaryOp {
        /// The unary operator (e.g., "USub", "Not", "Invert").
        op: String,
        /// The operand of the unary expression.
        operand: Box<HashKey>,
    },

    /// Boolean operation (and/or with multiple operands)
    BoolOp {
        /// The boolean operator ("And" or "Or").
        op: String,
        /// The operands of the boolean expression.
        operands: Vec<HashKey>,
    },

    /// Comparison expression (e.g., a < b < c)
    /// Parts are stored as strings for simplicity
    Compare {
        /// The comparison chain parts as strings (operators and operands interleaved).
        parts: Vec<String>,
    },

    /// Function/method call - always unique (conservative)
    /// Each call gets a unique ID since calls may have side effects
    Call {
        /// Monotonically increasing ID ensuring each call site is treated as unique.
        unique_id: usize,
    },

    /// Attribute access (obj.attr)
    Attribute {
        /// The base object expression being accessed.
        value: Box<HashKey>,
        /// The attribute name being accessed on the object.
        attr: String,
    },

    /// Subscript access (obj[key])
    Subscript {
        /// The base object expression being subscripted.
        value: Box<HashKey>,
        /// The subscript key expression.
        slice: Box<HashKey>,
    },

    /// Unique marker for expressions that should never be equivalent
    /// Used for depth-limited expressions and other special cases
    Unique {
        /// Monotonically increasing ID ensuring this expression never matches another.
        id: usize,
    },
}

// =============================================================================
// Commutativity Support
// =============================================================================

/// Returns true if the given binary operator is commutative.
///
/// Commutative operators have the property that `a op b == b op a`:
/// - Add: a + b == b + a
/// - Mult: a * b == b * a
/// - BitOr: a | b == b | a
/// - BitAnd: a & b == b & a
/// - BitXor: a ^ b == b ^ a
///
/// Non-commutative operators (Sub, Div, Mod, Pow, LShift, RShift, etc.)
/// do NOT satisfy this property.
pub fn is_commutative(op: &str) -> bool {
    matches!(
        op,
        // Python operator enum names (backward compat)
        "Add" | "Mult" | "BitOr" | "BitAnd" | "BitXor" |
        // Raw operator text (multi-language universal)
        "+" | "*" | "|" | "&" | "^" |
        // Equality (commutative across all languages)
        "==" | "!=" |
        // Boolean (Python-style)
        "and" | "or" |
        // Boolean (C-style)
        "&&" | "||"
    )
}

/// Normalize a binary operation by sorting operands for commutative operators.
///
/// For commutative operations, we sort the operands to ensure that
/// `a + b` and `b + a` produce the same HashKey.
///
/// For non-commutative operations, operand order is preserved.
pub fn normalize_binop(op: &str, left: HashKey, right: HashKey) -> HashKey {
    let commutative = is_commutative(op);

    let (normalized_left, normalized_right) = if commutative {
        // Sort operands using a consistent ordering
        match compare_hash_keys(&left, &right) {
            Ordering::Greater => (right, left),
            _ => (left, right),
        }
    } else {
        (left, right)
    };

    HashKey::BinOp {
        op: op.to_string(),
        left: Box::new(normalized_left),
        right: Box::new(normalized_right),
        commutative,
    }
}

/// Compare two HashKeys for normalization ordering.
///
/// This provides a consistent total ordering for HashKey variants,
/// used to normalize commutative operations.
fn compare_hash_keys(a: &HashKey, b: &HashKey) -> Ordering {
    // Use discriminant first, then compare fields
    match (a, b) {
        (
            HashKey::Const {
                type_name: t1,
                repr: r1,
            },
            HashKey::Const {
                type_name: t2,
                repr: r2,
            },
        ) => t1.cmp(t2).then_with(|| r1.cmp(r2)),
        (HashKey::VarVN { vn: v1 }, HashKey::VarVN { vn: v2 }) => v1.cmp(v2),
        (HashKey::Name { name: n1 }, HashKey::Name { name: n2 }) => n1.cmp(n2),
        (
            HashKey::BinOp {
                op: o1,
                left: l1,
                right: r1,
                ..
            },
            HashKey::BinOp {
                op: o2,
                left: l2,
                right: r2,
                ..
            },
        ) => o1
            .cmp(o2)
            .then_with(|| compare_hash_keys(l1, l2))
            .then_with(|| compare_hash_keys(r1, r2)),
        (
            HashKey::UnaryOp {
                op: o1,
                operand: op1,
            },
            HashKey::UnaryOp {
                op: o2,
                operand: op2,
            },
        ) => o1.cmp(o2).then_with(|| compare_hash_keys(op1, op2)),
        (HashKey::Call { unique_id: u1 }, HashKey::Call { unique_id: u2 }) => u1.cmp(u2),
        (HashKey::Unique { id: u1 }, HashKey::Unique { id: u2 }) => u1.cmp(u2),
        // Fall back to discriminant ordering for different variants
        _ => discriminant_order(a).cmp(&discriminant_order(b)),
    }
}

/// Get a numeric discriminant for ordering different HashKey variants
fn discriminant_order(key: &HashKey) -> u8 {
    match key {
        HashKey::Const { .. } => 0,
        HashKey::VarVN { .. } => 1,
        HashKey::Name { .. } => 2,
        HashKey::BinOp { .. } => 3,
        HashKey::UnaryOp { .. } => 4,
        HashKey::BoolOp { .. } => 5,
        HashKey::Compare { .. } => 6,
        HashKey::Call { .. } => 7,
        HashKey::Attribute { .. } => 8,
        HashKey::Subscript { .. } => 9,
        HashKey::Unique { .. } => 10,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Commutativity Tests
    // =========================================================================

    #[test]
    fn test_is_commutative_add() {
        assert!(is_commutative("Add"));
    }

    #[test]
    fn test_is_commutative_mult() {
        assert!(is_commutative("Mult"));
    }

    #[test]
    fn test_is_commutative_bitor() {
        assert!(is_commutative("BitOr"));
    }

    #[test]
    fn test_is_commutative_bitand() {
        assert!(is_commutative("BitAnd"));
    }

    #[test]
    fn test_is_commutative_bitxor() {
        assert!(is_commutative("BitXor"));
    }

    #[test]
    fn test_is_not_commutative_sub() {
        assert!(!is_commutative("Sub"));
    }

    #[test]
    fn test_is_not_commutative_div() {
        assert!(!is_commutative("Div"));
    }

    #[test]
    fn test_is_not_commutative_mod() {
        assert!(!is_commutative("Mod"));
    }

    #[test]
    fn test_is_not_commutative_pow() {
        assert!(!is_commutative("Pow"));
    }

    #[test]
    fn test_is_not_commutative_lshift() {
        assert!(!is_commutative("LShift"));
    }

    #[test]
    fn test_is_not_commutative_rshift() {
        assert!(!is_commutative("RShift"));
    }

    // =========================================================================
    // P2: Raw Operator Text Commutativity Tests
    // =========================================================================

    #[test]
    fn test_is_commutative_raw_plus() {
        assert!(is_commutative("+"));
    }

    #[test]
    fn test_is_commutative_raw_star() {
        assert!(is_commutative("*"));
    }

    #[test]
    fn test_is_commutative_raw_eq() {
        assert!(is_commutative("=="));
    }

    #[test]
    fn test_is_commutative_raw_neq() {
        assert!(is_commutative("!="));
    }

    #[test]
    fn test_is_commutative_raw_pipe() {
        assert!(is_commutative("|"));
    }

    #[test]
    fn test_is_commutative_raw_ampersand() {
        assert!(is_commutative("&"));
    }

    #[test]
    fn test_is_commutative_raw_caret() {
        assert!(is_commutative("^"));
    }

    #[test]
    fn test_is_commutative_c_style_and() {
        assert!(is_commutative("&&"));
    }

    #[test]
    fn test_is_commutative_c_style_or() {
        assert!(is_commutative("||"));
    }

    #[test]
    fn test_is_not_commutative_raw_minus() {
        assert!(!is_commutative("-"));
    }

    #[test]
    fn test_is_not_commutative_raw_slash() {
        assert!(!is_commutative("/"));
    }

    #[test]
    fn test_is_not_commutative_raw_percent() {
        assert!(!is_commutative("%"));
    }

    // =========================================================================
    // Normalization Tests
    // =========================================================================

    #[test]
    fn test_hash_key_commutative_normalization_add() {
        // x + y and y + x should produce the same HashKey
        let x = HashKey::Name {
            name: "x".to_string(),
        };
        let y = HashKey::Name {
            name: "y".to_string(),
        };

        let xy = normalize_binop("Add", x.clone(), y.clone());
        let yx = normalize_binop("Add", y.clone(), x.clone());

        assert_eq!(xy, yx, "x + y and y + x should be equal");
    }

    #[test]
    fn test_hash_key_commutative_normalization_mult() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("Mult", a.clone(), b.clone());
        let ba = normalize_binop("Mult", b.clone(), a.clone());

        assert_eq!(ab, ba, "a * b and b * a should be equal");
    }

    #[test]
    fn test_hash_key_commutative_normalization_bitor() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("BitOr", a.clone(), b.clone());
        let ba = normalize_binop("BitOr", b.clone(), a.clone());

        assert_eq!(ab, ba, "a | b and b | a should be equal");
    }

    #[test]
    fn test_hash_key_commutative_normalization_bitand() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("BitAnd", a.clone(), b.clone());
        let ba = normalize_binop("BitAnd", b.clone(), a.clone());

        assert_eq!(ab, ba, "a & b and b & a should be equal");
    }

    #[test]
    fn test_hash_key_commutative_normalization_bitxor() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("BitXor", a.clone(), b.clone());
        let ba = normalize_binop("BitXor", b.clone(), a.clone());

        assert_eq!(ab, ba, "a ^ b and b ^ a should be equal");
    }

    #[test]
    fn test_hash_key_non_commutative_order_preserved_sub() {
        // a - b and b - a should be DIFFERENT
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("Sub", a.clone(), b.clone());
        let ba = normalize_binop("Sub", b.clone(), a.clone());

        assert_ne!(ab, ba, "a - b and b - a should be different");
    }

    #[test]
    fn test_hash_key_non_commutative_order_preserved_div() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("Div", a.clone(), b.clone());
        let ba = normalize_binop("Div", b.clone(), a.clone());

        assert_ne!(ab, ba, "a / b and b / a should be different");
    }

    // =========================================================================
    // HashKey Equality Tests
    // =========================================================================

    #[test]
    fn test_hash_key_const_equality() {
        let c1 = HashKey::Const {
            type_name: "int".to_string(),
            repr: "42".to_string(),
        };
        let c2 = HashKey::Const {
            type_name: "int".to_string(),
            repr: "42".to_string(),
        };
        let c3 = HashKey::Const {
            type_name: "int".to_string(),
            repr: "43".to_string(),
        };

        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
    }

    #[test]
    fn test_hash_key_var_vn_equality() {
        let v1 = HashKey::VarVN { vn: 1 };
        let v2 = HashKey::VarVN { vn: 1 };
        let v3 = HashKey::VarVN { vn: 2 };

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn test_hash_key_name_equality() {
        let n1 = HashKey::Name {
            name: "x".to_string(),
        };
        let n2 = HashKey::Name {
            name: "x".to_string(),
        };
        let n3 = HashKey::Name {
            name: "y".to_string(),
        };

        assert_eq!(n1, n2);
        assert_ne!(n1, n3);
    }

    #[test]
    fn test_hash_key_call_always_different() {
        // Calls with different unique_ids should be different
        let c1 = HashKey::Call { unique_id: 1 };
        let c2 = HashKey::Call { unique_id: 2 };
        let c3 = HashKey::Call { unique_id: 1 };

        assert_ne!(c1, c2);
        assert_eq!(c1, c3);
    }

    #[test]
    fn test_hash_key_unique_always_different() {
        let u1 = HashKey::Unique { id: 1 };
        let u2 = HashKey::Unique { id: 2 };

        assert_ne!(u1, u2);
    }

    // =========================================================================
    // Complex Expression Tests
    // =========================================================================

    #[test]
    fn test_nested_commutative_normalization() {
        // (a + b) + c and (b + a) + c should be equal
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };
        let c = HashKey::Name {
            name: "c".to_string(),
        };

        let ab = normalize_binop("Add", a.clone(), b.clone());
        let ba = normalize_binop("Add", b.clone(), a.clone());

        let abc = normalize_binop("Add", ab.clone(), c.clone());
        let bac = normalize_binop("Add", ba.clone(), c.clone());

        // ab and ba are normalized to the same value, so abc == bac
        assert_eq!(abc, bac, "Nested commutative expressions should normalize");
    }

    #[test]
    fn test_different_operators_not_equal() {
        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let add = normalize_binop("Add", a.clone(), b.clone());
        let sub = normalize_binop("Sub", a.clone(), b.clone());
        let mult = normalize_binop("Mult", a.clone(), b.clone());

        assert_ne!(add, sub);
        assert_ne!(add, mult);
        assert_ne!(sub, mult);
    }

    #[test]
    fn test_hash_key_as_hashmap_key() {
        use std::collections::HashMap;

        let mut map: HashMap<HashKey, usize> = HashMap::new();

        let key1 = HashKey::Name {
            name: "x".to_string(),
        };
        let key2 = HashKey::Name {
            name: "x".to_string(),
        };

        map.insert(key1, 1);

        // Same key should retrieve the same value
        assert_eq!(map.get(&key2), Some(&1));
    }

    #[test]
    fn test_commutative_keys_hash_same() {
        use std::collections::HashMap;

        let mut map: HashMap<HashKey, usize> = HashMap::new();

        let a = HashKey::Name {
            name: "a".to_string(),
        };
        let b = HashKey::Name {
            name: "b".to_string(),
        };

        let ab = normalize_binop("Add", a.clone(), b.clone());
        let ba = normalize_binop("Add", b.clone(), a.clone());

        map.insert(ab, 1);

        // ba should find the same entry as ab
        assert_eq!(map.get(&ba), Some(&1));
    }
}
