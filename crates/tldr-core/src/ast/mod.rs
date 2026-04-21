//! AST extraction and parsing for TLDR
//!
//! This module provides tree-sitter based code parsing and structure extraction.
//! It implements the core Layer 1 (AST) functionality:
//!
//! - `parser` - Tree-sitter parser pool for efficient parsing
//! - `extractor` - Extract code structure (functions, classes, imports)
//! - `extract` - Full module extraction with call graph
//! - `imports` - Language-specific import parsing

pub mod extract;
pub mod extractor;
pub mod function_finder;
pub mod imports;
pub mod parser;

pub use extract::{extract_file, extract_from_tree};
pub use extractor::get_code_structure;
pub use imports::get_imports;
pub use parser::ParserPool;
