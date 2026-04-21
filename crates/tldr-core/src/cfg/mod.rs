//! Control Flow Graph (CFG) module - Layer 3
//!
//! This module provides control flow graph extraction and analysis:
//! - Basic block identification
//! - Control flow edge construction
//! - Loop detection (headers, back edges)
//! - Exception handling modeling
//!
//! # Mitigations Addressed
//! - M7: CFG algorithm divergence - document block boundaries clearly
//! - M24: Recursive generics timeout - add depth limit for nested structures

pub mod extractor;

pub use extractor::get_cfg_context;
