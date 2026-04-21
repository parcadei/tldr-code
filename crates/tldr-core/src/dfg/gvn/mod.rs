//! Global Value Numbering (GVN) Module
//!
//! This module provides hash-based value numbering with commutativity awareness
//! for detecting redundant expressions in code.
//!
//! # Mitigations Addressed
//! - MIT-HASH-01b: Use HashKey enum instead of string concatenation to prevent
//!   collision issues (e.g., "binop:Add:ab:c" vs "binop:Add:a:bc")
//!
//! # Components
//!
//! - `types`: Core data structures (ExpressionRef, GVNEquivalence, Redundancy, GVNReport)
//! - `hash_key`: Structured HashKey enum for collision-free hashing
//!
//! # Behavioral Contracts (from spec.md)
//!
//! - BC-GVN-1: Commutativity normalization (a + b == b + a)
//! - BC-GVN-2: Alias propagation (x = expr; use x)
//! - BC-GVN-3: Sequential analysis (statement order matters)
//! - BC-GVN-4: Function call conservatism (calls always unique)
//! - BC-GVN-5: Depth limiting (>10 levels get unique VNs)
//! - BC-GVN-6: Redundancy detection (N expressions => N-1 redundancies)

pub mod engine;
pub mod hash_key;
pub mod types;

pub use engine::{compute_gvn, GVNEngine};
pub use hash_key::{is_commutative, normalize_binop, HashKey};
pub use types::{ExpressionRef, GVNEquivalence, GVNReport, Redundancy};
