//! Program Dependence Graph (PDG) module - Layer 5
//!
//! This module combines CFG and DFG to enable program slicing:
//! - PDG construction combining control and data dependencies
//! - Backward slicing: find what affects a line
//! - Forward slicing: find what a line affects
//!
//! # Architecture
//!
//! The PDG combines two types of dependencies:
//! - **Control Dependencies**: Line A is control-dependent on B if B's
//!   condition determines whether A executes
//! - **Data Dependencies**: Line A is data-dependent on B if B defines
//!   a variable that A uses
//!
//! # Program Slicing
//!
//! A program slice is the subset of statements that can affect (forward)
//! or be affected by (backward) a specific point in the program.
//!
//! ## Backward Slice
//! Given: return x at line 10
//! Returns: all lines that could affect the value of x at line 10
//!
//! ## Forward Slice
//! Given: x = 0 at line 1
//! Returns: all lines that could be affected by the value of x
//!
//! # Algorithm
//! 1. Build CFG for control dependencies
//! 2. Build DFG for data dependencies
//! 3. Traverse PDG edges in specified direction
//! 4. Collect all visited line numbers

pub mod extractor;
pub mod slice;

pub use extractor::get_pdg_context;
pub use slice::get_slice;
pub use slice::{get_slice_rich, RichSlice, SliceEdge, SliceNode};
