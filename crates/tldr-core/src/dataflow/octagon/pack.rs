//! Variable packing infrastructure for the octagon domain.
//!
//! Variable packing limits the number of variables tracked in the DBM
//! to maintain performance. Variables beyond the pack limit fall back
//! to interval-only tracking.
//!
//! # Design
//!
//! ```text
//! OctagonState {
//!     pack: Pack         // Top N variables tracked relationally in DBM
//!     overflow: HashMap  // Remaining variables tracked as intervals
//! }
//! ```
//!
//! The default pack limit is 64 variables, chosen so that the half-matrix
//! fits in L1 cache (~66KB).
//!
//! # References
//!
//! - ASTREE: Variable packing with 10-50 variable packs
//! - Prior art Section 3.1: 32-64 variable sweet spot

use std::collections::HashMap;

use super::bound::Bound;
use super::dbm::Dbm;

/// Default maximum number of variables in a pack.
///
/// Chosen so the half-matrix fits in L1 cache:
/// 64 vars -> 128*129/2 = 8256 entries * 8 bytes = ~66KB.
pub const DEFAULT_PACK_LIMIT: usize = 64;

/// A variable pack: maps program variable names to DBM indices.
///
/// Variables within the pack are tracked relationally via the DBM.
/// Variables outside the pack are tracked independently via intervals.
#[derive(Debug, Clone)]
pub struct Pack<B: Bound> {
    /// The DBM for this pack.
    dbm: Dbm<B>,
    /// Map from variable name to pack-local variable index (0-based).
    var_to_idx: HashMap<String, usize>,
    /// Reverse map: pack-local index to variable name.
    idx_to_var: Vec<String>,
    /// Maximum number of variables this pack can hold.
    limit: usize,
}

impl<B: Bound> Pack<B> {
    /// Create a new empty pack with the given variable limit.
    pub fn new(limit: usize) -> Self {
        Pack {
            dbm: Dbm::new(0),
            var_to_idx: HashMap::new(),
            idx_to_var: Vec::new(),
            limit,
        }
    }

    /// Create a new pack with default limit.
    pub fn with_default_limit() -> Self {
        Self::new(DEFAULT_PACK_LIMIT)
    }

    /// Try to add a variable to the pack.
    ///
    /// Returns `Some(idx)` if the variable was added (or already exists),
    /// `None` if the pack is full.
    pub fn add_var(&mut self, name: &str) -> Option<usize> {
        if let Some(&idx) = self.var_to_idx.get(name) {
            return Some(idx);
        }
        if self.idx_to_var.len() >= self.limit {
            return None;
        }
        let idx = self.idx_to_var.len();
        self.var_to_idx.insert(name.to_string(), idx);
        self.idx_to_var.push(name.to_string());

        // Rebuild DBM with new size, copying existing constraints
        let new_n = self.idx_to_var.len();
        let old_dbm = std::mem::replace(&mut self.dbm, Dbm::new(new_n));
        let old_dim = old_dbm.dim();
        for i in 0..old_dim {
            for j in 0..old_dim {
                let val = old_dbm.get(i, j);
                self.dbm.set(i, j, val);
            }
        }

        Some(idx)
    }

    /// Get the pack-local index for a variable name.
    pub fn var_index(&self, name: &str) -> Option<usize> {
        self.var_to_idx.get(name).copied()
    }

    /// Get the variable name for a pack-local index.
    pub fn var_name(&self, idx: usize) -> Option<&str> {
        self.idx_to_var.get(idx).map(|s| s.as_str())
    }

    /// Number of variables currently in the pack.
    pub fn len(&self) -> usize {
        self.idx_to_var.len()
    }

    /// Whether the pack is empty.
    pub fn is_empty(&self) -> bool {
        self.idx_to_var.is_empty()
    }

    /// Whether the pack is full (at capacity).
    pub fn is_full(&self) -> bool {
        self.idx_to_var.len() >= self.limit
    }

    /// Get the pack limit.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Access the underlying DBM.
    pub fn dbm(&self) -> &Dbm<B> {
        &self.dbm
    }

    /// Access the underlying DBM mutably.
    pub fn dbm_mut(&mut self) -> &mut Dbm<B> {
        &mut self.dbm
    }

    /// Get all variable names in the pack, ordered by index.
    pub fn var_names(&self) -> &[String] {
        &self.idx_to_var
    }
}

/// Interval-only value for overflow variables.
///
/// Variables that don't fit in the pack are tracked with independent bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalValue {
    /// Lower bound (None = -infinity).
    pub low: Option<i64>,
    /// Upper bound (None = +infinity).
    pub high: Option<i64>,
}

impl IntervalValue {
    /// Top (unknown): no bounds.
    pub fn top() -> Self {
        IntervalValue {
            low: None,
            high: None,
        }
    }

    /// Exact value: [v, v].
    pub fn exact(v: i64) -> Self {
        IntervalValue {
            low: Some(v),
            high: Some(v),
        }
    }
}

/// Hybrid state: pack (relational) + overflow (interval-only).
#[derive(Debug, Clone)]
pub struct HybridState<B: Bound> {
    /// Relational pack for top-N variables.
    pack: Pack<B>,
    /// Overflow variables tracked as intervals.
    overflow: HashMap<String, IntervalValue>,
}

impl<B: Bound> Default for HybridState<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: Bound> HybridState<B> {
    /// Create a new hybrid state with default pack limit.
    pub fn new() -> Self {
        HybridState {
            pack: Pack::with_default_limit(),
            overflow: HashMap::new(),
        }
    }

    /// Create a new hybrid state with custom pack limit.
    pub fn with_limit(limit: usize) -> Self {
        HybridState {
            pack: Pack::new(limit),
            overflow: HashMap::new(),
        }
    }

    /// Access the pack.
    pub fn pack(&self) -> &Pack<B> {
        &self.pack
    }

    /// Access the pack mutably.
    pub fn pack_mut(&mut self) -> &mut Pack<B> {
        &mut self.pack
    }

    /// Access the overflow map (immutable).
    pub fn overflow(&self) -> &HashMap<String, IntervalValue> {
        &self.overflow
    }

    /// Access the overflow map mutably.
    pub fn overflow_mut(&mut self) -> &mut HashMap<String, IntervalValue> {
        &mut self.overflow
    }

    /// Get the interval for a variable, whether packed or overflow.
    ///
    /// For packed variables, projects the interval from the DBM.
    /// For overflow variables, returns the stored interval.
    /// For unknown variables, returns top (no information).
    pub fn get_interval(&self, name: &str) -> IntervalValue {
        if let Some(idx) = self.pack.var_index(name) {
            // Project from DBM using query::project_interval
            let (lo_f64, hi_f64) = super::query::project_interval(self.pack.dbm(), idx);
            IntervalValue {
                low: lo_f64.map(|v| v as i64),
                high: hi_f64.map(|v| v as i64),
            }
        } else if let Some(iv) = self.overflow.get(name) {
            iv.clone()
        } else {
            IntervalValue::top()
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
    fn test_pack_limit_64_vars() {
        // Pack should hold at most 64 variables by default
        let mut pack = Pack::<f64>::with_default_limit();

        for i in 0..64 {
            let name = format!("v{i}");
            let result = pack.add_var(&name);
            assert!(
                result.is_some(),
                "Should be able to add variable {i} (within limit 64)"
            );
        }

        assert!(pack.is_full(), "Pack should be full after adding 64 vars");

        // 65th variable should fail
        let result = pack.add_var("v64");
        assert!(
            result.is_none(),
            "Adding 65th variable should fail (pack limit is 64)"
        );
    }

    #[test]
    fn test_overflow_to_intervals() {
        // Variables beyond pack limit should fall back to interval tracking
        let mut state = HybridState::<f64>::with_limit(2);

        // Add first 2 variables to pack
        state.pack.add_var("x");
        state.pack.add_var("y");

        // 3rd variable overflows to interval tracking
        assert!(state.pack.add_var("z").is_none(), "z should overflow");
        state.overflow.insert("z".to_string(), IntervalValue::exact(42));

        // z should be retrievable from overflow
        let z_val = state.get_interval("z");
        assert_eq!(
            z_val,
            IntervalValue::exact(42),
            "Overflow variable z should have exact interval [42, 42]"
        );
    }

    #[test]
    fn test_pack_dbm_fits_l1_cache() {
        // 64 vars * half-matrix should fit in L1 cache (~66KB)
        let pack = Pack::<f64>::new(64);

        // Create a DBM with 64 vars to check size
        let dbm = Dbm::<f64>::new(64);
        let entries = dbm.len();
        let bytes = entries * std::mem::size_of::<f64>();

        assert!(
            bytes <= 66 * 1024,
            "64-var DBM should fit in L1 cache (~66KB); actual = {} bytes",
            bytes
        );

        // Also verify pack limit
        assert_eq!(pack.limit(), 64);
    }

    #[test]
    fn test_hybrid_state_get() {
        // Should retrieve from DBM for packed vars, intervals for overflow
        let mut state = HybridState::<f64>::with_limit(2);

        state.pack.add_var("x");
        state.overflow.insert("z".to_string(), IntervalValue::exact(10));

        // Packed var returns projected interval (currently top since DBM projection is stub)
        let x_val = state.get_interval("x");
        // After implementation, this should return the projected interval from DBM.
        // For now, the stub returns top.
        assert_eq!(x_val, IntervalValue::top());

        // Overflow var returns stored interval
        let z_val = state.get_interval("z");
        assert_eq!(z_val, IntervalValue::exact(10));

        // Unknown var returns top
        let unknown = state.get_interval("unknown_var");
        assert_eq!(unknown, IntervalValue::top());
    }

    #[test]
    fn test_pack_add_idempotent() {
        // Adding the same variable twice should return the same index
        let mut pack = Pack::<f64>::new(64);

        let idx1 = pack.add_var("x").unwrap();
        let idx2 = pack.add_var("x").unwrap();

        assert_eq!(idx1, idx2, "Adding same variable twice should return same index");
        assert_eq!(pack.len(), 1, "Pack should only have 1 variable");
    }

    #[test]
    fn test_pack_var_name_roundtrip() {
        let mut pack = Pack::<f64>::new(64);

        pack.add_var("alpha");
        pack.add_var("beta");
        pack.add_var("gamma");

        assert_eq!(pack.var_name(0), Some("alpha"));
        assert_eq!(pack.var_name(1), Some("beta"));
        assert_eq!(pack.var_name(2), Some("gamma"));
        assert_eq!(pack.var_name(3), None);

        assert_eq!(pack.var_index("alpha"), Some(0));
        assert_eq!(pack.var_index("beta"), Some(1));
        assert_eq!(pack.var_index("delta"), None);
    }
}
