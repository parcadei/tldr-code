//! Static Single Assignment (SSA) Form Module
//!
//! This module provides SSA construction and analysis:
//!
//! - Dominator tree construction (Lengauer-Tarjan algorithm)
//! - Dominance frontier calculation
//! - Phi function placement (Cytron algorithm)
//! - Variable versioning and renaming
//! - Memory SSA for heap operations
//! - SSA-based analyses (SCCP, dead code detection, value numbering)
//!
//! # References
//!
//! - Cytron et al. (1991) - "Efficiently Constructing SSA Form"
//! - Lengauer & Tarjan (1979) - "A Fast Algorithm for Finding Dominators"
//! - Wegman & Zadeck (1991) - "Constant Propagation with Conditional Branches"

pub mod analysis;
pub mod construct;
pub mod dominators;
pub mod format;
pub mod memory;
pub mod types;

pub use analysis::*;
pub use construct::*;
pub use dominators::*;
pub use format::*;
pub use memory::*;
pub use types::*;

#[cfg(test)]
mod ssa_tests;
