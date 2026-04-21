//! Dataflow Analysis Module
//!
//! This module provides two forward dataflow analyses:
//!
//! 1. **Available Expressions Analysis** (CAP-AE-01 through CAP-AE-12)
//!    - Forward MUST (intersection) dataflow analysis
//!    - Common Subexpression Elimination (CSE) detection
//!    - Commutative expression normalization
//!
//! 2. **Abstract Interpretation** (CAP-AI-01 through CAP-AI-22)
//!    - Forward dataflow with widening for loop termination
//!    - Range tracking for integer variables
//!    - Nullability tracking (NEVER/MAYBE/ALWAYS)
//!    - Division-by-zero detection
//!    - Null dereference detection
//!    - Multi-language support (Python, TypeScript, Go, Rust)
//!
//! ## Module Structure
//!
//! ```text
//! dataflow/
//! ├── mod.rs              # This file - module entry point
//! ├── types.rs            # Shared types (BlockId, predecessors)
//! ├── available.rs        # Available expressions analysis
//! ├── abstract_interp.rs  # Abstract interpretation
//! └── dataflow_tests.rs   # Comprehensive test suite
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tldr_core::dataflow::{
//!     compute_available_exprs, AvailableExprsInfo, Expression,
//!     compute_abstract_interp, AbstractInterpInfo, AbstractValue, Nullability,
//! };
//!
//! // Available Expressions Analysis
//! let cfg = get_cfg_context(path, func, None, None)?;
//! let dfg = get_dfg_context(path, func, None, None)?;
//! let avail = compute_available_exprs(&cfg, &dfg)?;
//! let redundant = avail.redundant_computations();
//!
//! // Abstract Interpretation
//! let interp = compute_abstract_interp(&cfg, &refs, Some(&source), "python")?;
//! for (line, var) in &interp.potential_div_zero {
//!     println!("Warning: potential division by zero at line {} ({})", line, var);
//! }
//! ```
//!
//! ## Specification Reference
//!
//! See `spec.md` in this directory for:
//! - Full capability definitions (34 capabilities)
//! - Algorithm descriptions
//! - Behavioral contracts
//! - JSON output formats
//! - Edge case handling

// =============================================================================
// Submodules
// =============================================================================

// Types submodule (shared types and helpers)
pub mod types;

// Available Expressions Analysis
pub mod available;

// Abstract Interpretation
pub mod abstract_interp;

// Tests
#[cfg(test)]
mod dataflow_tests;

// =============================================================================
// Re-exports (Phase 12: Integration & Public API)
// =============================================================================

// Available Expressions types and functions
pub use available::{
    // Phase 12: Main algorithm
    compute_available_exprs,
    compute_available_exprs_with_source,
    compute_available_exprs_with_source_and_lang,
    extract_binary_exprs_from_ast,
    extract_expressions_full,
    normalize_expression,
    AvailableExprsInfo,
    BlockExpressions,
    Confidence,
    ExprInstance,
    Expression,
    ExtractionResult,
    UncertainFinding,
    COMMUTATIVE_OPS,
};

// Abstract Interpretation types and functions
pub use abstract_interp::{
    // Phase 10: Main algorithm
    compute_abstract_interp,
    init_params,
    transfer_block,
    AbstractInterpInfo,
    AbstractState,
    AbstractValue,
    ConstantValue,
    Nullability,
};

// Shared types and helpers
pub use types::{
    build_predecessors, build_successors, find_back_edges, reachable_blocks, reverse_postorder,
    validate_cfg, BlockId, DataflowError, MAX_BLOCKS, MAX_ITERATIONS,
};
