//! Context module - LLM-ready context generation
//!
//! This module provides functions for generating token-efficient context
//! from a codebase starting from an entry point function.
//!
//! # Features
//! - BFS traversal from entry point
//! - Configurable depth limit
//! - Optional docstring inclusion
//! - CFG metrics integration
//! - 95% token savings vs full file reading
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::context::get_relevant_context;
//! use tldr_core::Language;
//!
//! let ctx = get_relevant_context(
//!     Path::new("src"),
//!     "main",
//!     2,           // depth
//!     Language::Python,
//!     true,        // include_docstrings
//!     None,        // file_filter (None = search all files)
//! )?;
//!
//! println!("{}", ctx.to_llm_string());
//! ```

pub mod builder;

pub use builder::{get_relevant_context, FunctionContext, RelevantContext};
