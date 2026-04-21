//! TLDR-Core: Core analysis engine for code analysis
//!
//! This crate provides the core analysis functionality for the TLDR tool,
//! organized into analysis layers:
//!
//! - **Layer 1 (AST)**: tree, structure, extract, imports
//! - **Layer 2 (Call Graph)**: calls, impact, dead, importers, arch
//! - **Layer 3 (CFG)**: cfg, complexity
//! - **Layer 4 (DFG)**: dfg
//! - **Layer 5 (PDG)**: pdg, slice, thin_slice
//! - **Layer 6 (SSA)**: ssa, reaching-defs (Session 10)
//! - **Layer 7 (Alias)**: alias analysis (Session 11+)
//! - **Layer 8 (Dataflow)**: available expressions, abstract interpretation (Session 13)
//!
//! Plus search, context, quality, security, and diagnostics modules.
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::{Language, TldrResult};
//! use tldr_core::fs::tree::get_file_tree;
//! use tldr_core::ast::{get_code_structure, extract_file, get_imports};
//!
//! // Detect language from file extension
//! let lang = Language::from_extension(".py");
//! assert_eq!(lang, Some(Language::Python));
//!
//! // Get file tree
//! let tree = get_file_tree(Path::new("src"), None, true, None)?;
//!
//! // Extract code structure
//! let structure = get_code_structure(Path::new("src"), Language::Python, 0, None)?;
//! ```

#![warn(missing_docs)]
#![warn(rustdoc::missing_crate_level_docs)]

pub mod error;
pub mod git;
pub mod types;
pub mod validation;

// Phase 10: Robustness (A32, A33, A34)
pub mod encoding;
pub mod limits;

// Phase 2: AST Layer (L1) - implemented
pub mod ast;
pub mod fs;

// Phase 3: Call Graph Layer (L2) - implemented
pub mod analysis;
pub mod callgraph;

// Phase 4: CFG Layer (L3) - implemented
pub mod cfg;
pub mod metrics;

// Phase 6: Search Layer - implemented
pub mod search;

// Phase 5: DFG Layer (L4) - implemented
pub mod dfg;

// Phase 5: PDG Layer (L5) - implemented
pub mod pdg;

// Session 10: SSA Layer (L6) - in progress
pub mod ssa;

// Session 11+: Alias Analysis Layer (L7) - in progress
pub mod alias;

// Session 13: Dataflow Analysis Layer (L8) - in progress
// Available expressions (CSE detection) and Abstract interpretation (range/nullability)
pub mod dataflow;

// Session 16: Semantic Search Layer - in progress
// Dense embeddings for code search using Snowflake Arctic models
#[cfg(feature = "semantic")]
pub mod semantic;

// API surface extraction for libraries/packages
pub mod surface;

// Error diagnosis and auto-fix system (Phase 2)
pub mod fix;

// GVN Migration: Phase P0 - Base infrastructure for orchestrated analysis
pub mod wrappers;

// Re-export main types for convenience
pub use error::TldrError;
pub use types::*;

// Re-export Layer 1 functions for convenience
pub use ast::{extract_file, get_code_structure, get_imports};
pub use fs::get_file_tree;

// Re-export Layer 2 functions for convenience
pub use analysis::{
    architecture_analysis, change_impact, change_impact_extended, collect_all_functions,
    dead_code_analysis, find_importers, impact_analysis, impact_analysis_with_ast_fallback,
    ChangeImpactMetadata, ChangeImpactReport, DetectionMethod, TestFunction,
};
pub use callgraph::build_project_call_graph;

// Re-export Layer 3 functions for convenience
pub use cfg::get_cfg_context;
pub use metrics::calculate_complexity;

// Re-export Layer 4 (DFG) functions for convenience
pub use dfg::get_dfg_context;

// Re-export Layer 5 (PDG) functions for convenience
pub use pdg::{get_pdg_context, get_slice, get_slice_rich, RichSlice, SliceEdge, SliceNode};

// Re-export Layer 6 (Search) functions for convenience
pub use search::{
    enriched_search, enriched_search_with_callgraph_cache, enriched_search_with_index,
    enriched_search_with_structure_cache, hybrid_search, read_callgraph_cache,
    read_structure_cache, search, search_with_inner, write_structure_cache, Bm25Index, Bm25Result,
    CallGraphLookup, EmbeddingClient, EnrichedResult, EnrichedSearchOptions, EnrichedSearchReport,
    HybridResult, HybridSearchReport, SearchMatch, SearchMode, StructureLookup, Tokenizer,
};

/// Result type alias for all TLDR operations
pub type TldrResult<T> = Result<T, TldrError>;

// Re-export validation functions (P4: DRY validators)
pub use validation::{detect_or_parse_language, validate_file_path};

// Phase 7: Context & Analysis
pub mod context;

// Re-export context module functions
pub use context::{get_relevant_context, FunctionContext, RelevantContext};

// Phase 8: Quality & Security - implemented
pub mod quality;
pub mod security;

// Phase 4-6: Pattern Detection - implemented
pub mod patterns;

// Re-export Quality module functions
pub use quality::{
    analyze_smells_aggregated, detect_smells, SmellFinding, SmellType, SmellsReport,
    ThresholdPreset,
};
pub use quality::{
    maintainability_index, FileMI, HalsteadMetrics, MISummary, MaintainabilityReport,
};

// Re-export Security module functions
pub use security::{
    compute_taint, compute_taint_with_tree, TaintFlow, TaintInfo, TaintSink, TaintSinkType,
    TaintSource, TaintSourceType,
};
pub use security::{scan_secrets, SecretFinding, SecretsReport, Severity};
pub use security::{scan_vulnerabilities, VulnFinding, VulnReport, VulnType};

// Re-export Pattern detection functions
pub use patterns::{detect_patterns, detect_patterns_with_config, PatternConfig, PatternMiner};

// Phase 7-9: Inheritance Analysis - implemented
pub mod inheritance;

// Re-export Inheritance module functions
pub use inheritance::format::{format_dot, format_text};
pub use inheritance::{extract_inheritance, InheritanceOptions};

// Session 6: Diagnostics module - unified type checking and linting
pub mod diagnostics;

// Re-export Diagnostics module types (implementations pending)
pub use diagnostics::{
    Diagnostic, DiagnosticsReport, DiagnosticsSummary, Severity as DiagnosticSeverity,
};

// Alias analysis re-exports will be added when implemented:
// pub use alias::{compute_alias, compute_alias_from_ssa, AliasInfo, AliasError, AbstractLocation};

// Dataflow analysis re-exports (Phase 12: Integration & Public API)
pub use dataflow::{
    // Abstract Interpretation (CAP-AI-01 through CAP-AI-22)
    compute_abstract_interp,
    // Available Expressions (CAP-AE-01 through CAP-AE-12)
    compute_available_exprs,
    compute_available_exprs_with_source_and_lang,
    normalize_expression,
    AbstractInterpInfo,
    AbstractState,
    AbstractValue,
    AvailableExprsInfo,
    ConstantValue,
    Expression,
    Nullability,
};
