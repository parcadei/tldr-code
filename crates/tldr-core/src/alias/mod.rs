//! Alias Analysis Module
//!
//! Flow-insensitive Andersen-style points-to analysis for determining when
//! two references may or must refer to the same object.
//!
//! ## Overview
//!
//! This module implements alias analysis with the following capabilities:
//!
//! - **May-alias**: Sound check for potential aliasing (no false negatives)
//! - **Must-alias**: Precise check for definite aliasing (no false positives)
//! - **Points-to sets**: Track what abstract locations variables may reference
//!
//! ## Algorithm
//!
//! Uses Andersen's subset-based analysis (1994):
//! - Flow-insensitive (processes all statements once)
//! - Complexity: O(n^3) time, O(n^2) space
//! - Key property: Inclusion constraints `pts(x) >= pts(y)` for `x = y`
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tldr_core::alias::{compute_alias_from_ssa, AliasInfo};
//!
//! let alias_info = compute_alias_from_ssa(&ssa_function)?;
//!
//! if alias_info.may_alias_check("x_0", "y_0") {
//!     println!("x and y may point to same object");
//! }
//!
//! if alias_info.must_alias_check("x_0", "p_0") {
//!     println!("x definitely aliases p");
//! }
//! ```
//!
//! ## References
//!
//! - Andersen, L. O. (1994). Program Analysis and Specialization for the C
//!   Programming Language. PhD thesis, University of Copenhagen.
//! - See `spec.md` for detailed specification.

// Phase 1: Types (implemented)
mod types;

// Phase 2: Constraint generation (implemented)
mod constraints;

// Phase 3: Fixed-point solver (implemented)
mod solver;

// Phase 7: Output formatting (implemented)
mod format;

// Re-exports
pub use constraints::{Constraint, ConstraintExtractor};
pub use format::AliasOutputFormat;
pub use solver::{AliasSolver, MAX_ITERATIONS};
pub use types::{
    AbstractLocation, AliasError, AliasInfo, Confidence, UncertainAlias, MAX_FIELD_DEPTH,
};

// SSA types needed for the public API
use crate::ssa::types::SsaFunction;

// =============================================================================
// Public API: Main Entry Points (Phase 4)
// =============================================================================

/// Compute alias analysis from SSA form.
///
/// This is the main entry point for alias analysis. It extracts constraints
/// from the SSA function, runs the fixed-point solver, and returns complete
/// alias information.
///
/// # Arguments
/// * `ssa` - SSA form of the function to analyze
///
/// # Returns
/// * `Ok(AliasInfo)` - Complete alias analysis results
/// * `Err(AliasError)` - If analysis fails (e.g., invalid SSA, iteration limit)
///
/// # Algorithm
/// 1. Extract constraints from SSA (phi functions, assignments, calls)
/// 2. Initialize points-to sets from allocations and parameters
/// 3. Run fixed-point iteration until convergence
/// 4. Derive may-alias from points-to overlap
/// 5. Derive must-alias from direct copies (excluding phi targets)
/// 6. Add conservative parameter aliasing
///
/// # Example
/// ```rust,ignore
/// use tldr_core::alias::compute_alias_from_ssa;
/// use tldr_core::ssa::types::SsaFunction;
///
/// let ssa: SsaFunction = /* construct SSA */;
/// let alias_info = compute_alias_from_ssa(&ssa)?;
///
/// // Check if two variables may alias
/// if alias_info.may_alias_check("x_0", "y_0") {
///     println!("x and y may point to same object");
/// }
///
/// // Check what x points to
/// let pts = alias_info.get_points_to("x_0");
/// println!("x points to: {:?}", pts);
/// ```
///
/// # TIGER Mitigations
/// - TIGER-3: Validates all SSA references before processing
/// - TIGER-14: Validates phi source counts match predecessors
/// - TIGER-2: Returns error if fixed-point iteration exceeds limit
/// - TIGER-6: Computes transitive closure for must-alias
pub fn compute_alias_from_ssa(ssa: &SsaFunction) -> Result<AliasInfo, AliasError> {
    // Phase 2: Extract constraints from SSA
    let extractor = ConstraintExtractor::extract_from_ssa(ssa)?;

    // Phase 3: Run fixed-point solver
    let mut solver = AliasSolver::new(&extractor);
    solver.solve()?;

    // Build and return alias info
    Ok(solver.build_alias_info(&ssa.function))
}

/// Compute alias analysis for a function (convenience wrapper).
///
/// This is a convenience wrapper that takes an SSA function reference
/// and returns alias analysis results. Use this when you have an SSA
/// function ready for analysis.
///
/// # Arguments
/// * `ssa` - Reference to SSA function
///
/// # Returns
/// * `Ok(AliasInfo)` - Alias analysis results
/// * `Err(AliasError)` - If analysis fails
pub fn compute_alias(ssa: &SsaFunction) -> Result<AliasInfo, AliasError> {
    compute_alias_from_ssa(ssa)
}

#[cfg(test)]
mod alias_tests;
