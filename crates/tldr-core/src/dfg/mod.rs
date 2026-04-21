//! Data Flow Graph (DFG) module - Layer 4
//!
//! This module provides data flow analysis:
//! - Variable reference extraction (definitions, uses, updates)
//! - Def-use chain construction
//! - Reaching definitions analysis
//! - Global Value Numbering (GVN) for redundancy detection
//!
//! # Mitigations Addressed
//! - M7: DFG algorithm divergence - document def-use chain computation clearly
//!
//! # Algorithm Overview
//!
//! ## Variable References
//! We identify three types of variable references:
//! - **Definition**: `x = value` - variable is assigned a new value
//! - **Update**: `x += value` or `x.method()` - variable is modified in-place
//! - **Use**: `f(x)` or `y = x` - variable's value is read
//!
//! ## Def-Use Chains
//! For each variable use, we find all definitions that could reach it:
//! 1. Build CFG for the function
//! 2. Identify all variable references
//! 3. Apply reaching definitions analysis on CFG
//! 4. Connect each use to its reaching definitions
//!
//! ## Reaching Definitions
//! Classic dataflow analysis using iterative algorithm:
//! - IN[B] = union of OUT[P] for all predecessors P of B
//! - OUT[B] = GEN[B] union (IN[B] - KILL[B])
//! - Iterate until fixed point
//!
//! ## Global Value Numbering (GVN)
//! Hash-based value numbering with commutativity awareness:
//! - Assigns unique value numbers to expressions
//! - Detects redundant computations
//! - Handles commutative operators (a + b == b + a)

pub mod chains;
pub mod extractor;
pub mod format;
pub mod gvn;
pub mod reaching;

pub use chains::{
    BlockReachingDefs, Confidence, DefUseChain, Definition, ReachingDefsReport, ReachingDefsStats,
    UncertainDef, UninitSeverity, UninitializedUse, Use, UseDefChain,
};
pub use extractor::get_dfg_context;
pub use format::{
    filter_reaching_defs_by_variable, format_reaching_defs_json, format_reaching_defs_json_compact,
    format_reaching_defs_text, format_reaching_defs_text_with_options, ReachingDefsFormatOptions,
};
pub use reaching::{
    build_def_use_chains, build_reaching_defs_report, build_use_def_chains,
    compute_reaching_definitions, compute_reaching_definitions_bitvec,
    compute_reaching_definitions_rpo, compute_rpo, create_dense_def_mapping,
    definitions_reaching_line, detect_uninitialized, detect_uninitialized_simple,
    BitVectorReachingDefs, DefId, DenseDefMapping, ReachingDefinitions,
    ReachingDefinitionsWithStats,
};

#[cfg(test)]
mod reaching_tests;

#[cfg(test)]
mod gvn_tests;
