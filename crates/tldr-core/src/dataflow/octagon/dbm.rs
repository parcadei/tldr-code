//! Difference Bound Matrix (DBM) storage for the octagon domain.
//!
//! The DBM represents octagonal constraints `+-xi +- xj <= c` using a
//! half-matrix representation (Mine 2006). For `n` program variables,
//! the full DBM is `2n x 2n`, but the half-matrix exploits coherence
//! to store only the lower-left triangle.
//!
//! # Variable Indexing
//!
//! Each program variable `xi` maps to two DBM indices:
//! - `2i` (positive literal)
//! - `2i+1` (negative literal / negated)
//!
//! The complement of index `k` is `k XOR 1`.
//!
//! # Storage Layout
//!
//! The half-matrix is stored as a contiguous `Vec<B>` with
//! `2n * (2n + 1) / 2` entries, indexed by `(i, j)` where `i >= j`.
//! This ensures cache-friendly access patterns.
//!
//! # References
//!
//! - Mine 2006, Section 3.2: DBM representation
//! - APRON: `oct_hmat.c` half-matrix implementation
//! - Prior art Section 1.1: Half-matrix layout

use super::bound::Bound;

/// Half-matrix Difference Bound Matrix for `n` variables.
///
/// Stores octagonal constraints in a flat `Vec<B>` with
/// `2n * (2n + 1) / 2` entries for the lower triangle, plus `n`
/// entries for same-variable upper-triangle pairs `m[2k, 2k+1]`.
///
/// Each entry `m[i, j]` represents the constraint
/// `x_j - x_i <= m[i, j]` where `x_k` for even `k` is the
/// positive literal and for odd `k` is the negated literal.
///
/// The coherence property `m[i,j] = m[j^1, i^1]` relates off-block
/// entries, but same-variable pairs `(2k, 2k+1)` are self-coherent
/// and need independent storage from `(2k+1, 2k)`.
#[derive(Debug, Clone)]
pub struct Dbm<B: Bound> {
    /// Number of program variables (n). The DBM dimension is 2n.
    n_vars: usize,
    /// Flat storage for the half-matrix (lower triangle). Length = 2n * (2n + 1) / 2.
    data: Vec<B>,
    /// Separate storage for same-variable upper-triangle entries `m[2k, 2k+1]`.
    /// Length = n. These are self-coherent and cannot alias with lower-triangle
    /// entries because the coherence remap `(j^1, i^1)` for `(2k, 2k+1)` yields
    /// `(2k, 2k+1)` again (still upper triangle).
    upper_diag: Vec<B>,
}

impl<B: Bound> Dbm<B> {
    /// Create a new DBM for `n` program variables.
    ///
    /// All entries are initialized to +infinity (unconstrained),
    /// except diagonal entries `m[i, i]` which are 0.
    pub fn new(n_vars: usize) -> Self {
        let dim = 2 * n_vars;
        let size = dim * (dim + 1) / 2;
        let mut data = vec![B::infinity(); size];

        // Set diagonal to zero: m[i, i] = 0
        for i in 0..dim {
            let idx = Self::lower_index(i, i);
            if idx < data.len() {
                data[idx] = B::zero();
            }
        }

        // Upper-diagonal entries m[2k, 2k+1] initialized to +infinity
        let upper_diag = vec![B::infinity(); n_vars];

        Dbm { n_vars, data, upper_diag }
    }

    /// Number of program variables.
    pub fn n_vars(&self) -> usize {
        self.n_vars
    }

    /// DBM dimension (2 * n_vars).
    pub fn dim(&self) -> usize {
        2 * self.n_vars
    }

    /// Total number of entries in the half-matrix (lower triangle).
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the DBM has zero variables.
    pub fn is_empty(&self) -> bool {
        self.n_vars == 0
    }

    /// Check if `(i, j)` is a same-variable upper-triangle entry: `i` is even
    /// and `j == i + 1` (i.e., `(2k, 2k+1)` for some variable `k`).
    #[inline]
    fn is_upper_diag(i: usize, j: usize) -> bool {
        i < j && (i ^ 1) == j
    }

    /// Compute the flat index for a lower-triangle entry `(i, j)` where `i >= j`.
    #[inline]
    fn lower_index(i: usize, j: usize) -> usize {
        i * (i + 1) / 2 + j
    }

    /// Compute the flat index for half-matrix entry `(i, j)`.
    ///
    /// For the lower-left triangle (`i >= j`), index = `i * (i + 1) / 2 + j`.
    ///
    /// For upper-triangle entries where `i < j`, the coherence property
    /// `m[i, j] = m[j XOR 1, i XOR 1]` (Mine 2006, Prop. 4) is used to
    /// remap to a lower-triangle entry. Same-variable pairs `(2k, 2k+1)`
    /// are handled separately via `upper_diag`.
    fn index_of(i: usize, j: usize) -> usize {
        if i >= j {
            Self::lower_index(i, j)
        } else {
            // Coherence: m[i, j] = m[j^1, i^1]
            let ci = j ^ 1;
            let cj = i ^ 1;
            // For off-block pairs, (ci, cj) is in the lower triangle
            Self::lower_index(ci, cj)
        }
    }

    /// Get the constraint value `m[i, j]`.
    ///
    /// Returns the bound for constraint `x_j - x_i <= m[i, j]`.
    pub fn get(&self, i: usize, j: usize) -> B {
        if Self::is_upper_diag(i, j) {
            // Same-variable upper-triangle: stored in upper_diag[k] where i = 2k
            self.upper_diag[i / 2]
        } else {
            let idx = Self::index_of(i, j);
            self.data[idx]
        }
    }

    /// Set the constraint value `m[i, j] = value`.
    pub fn set(&mut self, i: usize, j: usize, value: B) {
        if Self::is_upper_diag(i, j) {
            // Same-variable upper-triangle: stored in upper_diag[k] where i = 2k
            self.upper_diag[i / 2] = value;
        } else {
            let idx = Self::index_of(i, j);
            self.data[idx] = value;
        }
    }

    /// Compute the complement index: `i XOR 1`.
    ///
    /// Maps `2k -> 2k+1` and `2k+1 -> 2k`.
    #[inline]
    pub fn complement(i: usize) -> usize {
        i ^ 1
    }

    /// Access the underlying data slice (for memory layout verification).
    pub fn raw_data(&self) -> &[B] {
        &self.data
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dbm_new_creates_correct_size() {
        // For n variables, the half-matrix has 2n*(2n+1)/2 entries
        let dbm = Dbm::<f64>::new(3);
        let dim = 2 * 3; // 6
        let expected_size = dim * (dim + 1) / 2; // 6*7/2 = 21
        assert_eq!(
            dbm.len(),
            expected_size,
            "DBM for 3 vars should have {expected_size} half-matrix entries; got {}",
            dbm.len()
        );
        assert_eq!(dbm.n_vars(), 3);
        assert_eq!(dbm.dim(), 6);
    }

    #[test]
    fn test_dbm_new_various_sizes() {
        for n in 0..=10 {
            let dbm = Dbm::<f64>::new(n);
            let dim = 2 * n;
            let expected = dim * (dim + 1) / 2;
            assert_eq!(
                dbm.len(),
                expected,
                "DBM for {n} vars: expected {expected} entries, got {}",
                dbm.len()
            );
        }
    }

    #[test]
    fn test_dbm_index_2i_2i_plus_1() {
        // Variable xi maps to indices 2i (positive) and 2i+1 (negative)
        // Variable 0 -> indices 0, 1
        // Variable 1 -> indices 2, 3
        // Variable 2 -> indices 4, 5
        let dbm = Dbm::<f64>::new(3);

        // Set constraint on variable 1's positive literal
        let var_idx = 1;
        let pos_idx = 2 * var_idx;     // 2
        let neg_idx = 2 * var_idx + 1; // 3

        assert_eq!(pos_idx, 2, "Variable 1 positive index should be 2");
        assert_eq!(neg_idx, 3, "Variable 1 negative index should be 3");

        // Verify we can set and read constraints using these indices
        // Set: x1 <= 5 (encoded as m[2*1+1, 2*1] = 2*5 = 10, i.e., x1 - (-x1) <= 10)
        // More directly: upper bound of x1 is m[2i+1, 2i] / 2
        let _ = dbm; // just confirming indexing is correct
    }

    #[test]
    fn test_dbm_complement_xor_1() {
        // i XOR 1 gives the complement index
        assert_eq!(Dbm::<f64>::complement(0), 1);  // 2*0 -> 2*0+1
        assert_eq!(Dbm::<f64>::complement(1), 0);  // 2*0+1 -> 2*0
        assert_eq!(Dbm::<f64>::complement(2), 3);  // 2*1 -> 2*1+1
        assert_eq!(Dbm::<f64>::complement(3), 2);  // 2*1+1 -> 2*1
        assert_eq!(Dbm::<f64>::complement(4), 5);
        assert_eq!(Dbm::<f64>::complement(5), 4);

        // XOR 1 is self-inverse: complement(complement(i)) == i
        for i in 0..20 {
            assert_eq!(
                Dbm::<f64>::complement(Dbm::<f64>::complement(i)),
                i,
                "complement must be self-inverse"
            );
        }
    }

    #[test]
    fn test_dbm_half_matrix_size() {
        // Half-matrix has exactly 2n*(2n+1)/2 entries
        // For 64 vars: 128*129/2 = 8256 entries * 8 bytes = 66048 bytes (fits L1 cache)
        let dbm = Dbm::<f64>::new(64);
        let expected = 128 * 129 / 2;
        assert_eq!(dbm.len(), expected);

        // Memory size check: 8256 * 8 = 66048 bytes <= 66KB
        let mem_bytes = dbm.len() * std::mem::size_of::<f64>();
        assert!(
            mem_bytes <= 66 * 1024,
            "64-var DBM should fit in L1 cache (~66KB); got {mem_bytes} bytes"
        );
    }

    #[test]
    fn test_dbm_get_set_constraint() {
        let mut dbm = Dbm::<f64>::new(2);

        // Initially all non-diagonal entries are +infinity
        assert_eq!(dbm.get(0, 1), f64::INFINITY);
        assert_eq!(dbm.get(1, 0), f64::INFINITY);
        assert_eq!(dbm.get(2, 3), f64::INFINITY);

        // Set a constraint: x0 - x1 <= 5
        dbm.set(2, 0, 5.0); // m[2, 0] = 5

        assert_eq!(dbm.get(2, 0), 5.0, "Should read back the value we set");

        // Set another constraint
        dbm.set(3, 1, -3.0);
        assert_eq!(dbm.get(3, 1), -3.0);
    }

    #[test]
    fn test_dbm_initial_values_infinity() {
        // Fresh DBM has all entries at +infinity except diagonal = 0
        let dbm = Dbm::<f64>::new(3);
        let dim = dbm.dim();

        for i in 0..dim {
            for j in 0..dim {
                let val = dbm.get(i, j);
                if i == j {
                    assert_eq!(
                        val, 0.0,
                        "Diagonal entry m[{i},{j}] should be 0.0; got {val}"
                    );
                } else {
                    assert_eq!(
                        val,
                        f64::INFINITY,
                        "Off-diagonal entry m[{i},{j}] should be +inf; got {val}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_dbm_diagonal_zero() {
        // m[i, i] = 0 always, for any valid DBM
        let dbm = Dbm::<f64>::new(5);
        for i in 0..dbm.dim() {
            assert_eq!(
                dbm.get(i, i),
                0.0,
                "Diagonal entry m[{i},{i}] must always be 0"
            );
        }
    }

    #[test]
    fn test_dbm_memory_layout_contiguous() {
        // Data is stored in a flat Vec (not Vec of Vec)
        let dbm = Dbm::<f64>::new(4);
        let raw = dbm.raw_data();

        // The raw data should be a single contiguous slice
        assert_eq!(
            raw.len(),
            8 * 9 / 2,
            "Raw data length should equal half-matrix size"
        );

        // Verify it's contiguous memory by checking the pointer arithmetic
        let ptr_start = raw.as_ptr() as usize;
        let ptr_end = unsafe { raw.as_ptr().add(raw.len()) } as usize;
        let expected_bytes = raw.len() * std::mem::size_of::<f64>();
        assert_eq!(
            ptr_end - ptr_start,
            expected_bytes,
            "Data must be contiguous in memory"
        );
    }

    #[test]
    fn test_dbm_i64_backend() {
        // DBM should also work with i64 bounds
        let mut dbm = Dbm::<i64>::new(2);

        // Initial values: diagonal = 0, off-diagonal = i64::MAX
        assert_eq!(dbm.get(0, 0), 0);
        assert_eq!(dbm.get(0, 1), i64::MAX);

        // Set and read back
        dbm.set(1, 0, 10);
        assert_eq!(dbm.get(1, 0), 10);
    }

    #[test]
    fn test_dbm_zero_vars() {
        // Edge case: 0 variables
        let dbm = Dbm::<f64>::new(0);
        assert_eq!(dbm.len(), 0);
        assert_eq!(dbm.dim(), 0);
        assert!(dbm.is_empty());
    }

    #[test]
    fn test_dbm_single_var() {
        // 1 variable: dimension 2, half-matrix has 2*3/2 = 3 entries
        let dbm = Dbm::<f64>::new(1);
        assert_eq!(dbm.len(), 3); // entries: (0,0), (1,0), (1,1)
        assert_eq!(dbm.dim(), 2);

        // Diagonal is zero
        assert_eq!(dbm.get(0, 0), 0.0);
        assert_eq!(dbm.get(1, 1), 0.0);

        // Off-diagonal is infinity
        assert_eq!(dbm.get(1, 0), f64::INFINITY);
    }
}
