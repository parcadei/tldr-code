//! Call Graph Layer (L2) - Cross-file call graph construction
//!
//! This module provides call graph building and analysis functionality:
//!
//! - `builder` - Build project-wide call graphs using AST from Phase 2
//! - `resolver` - Module resolution for import tracking
//! - `module_index` - Bidirectional module path <-> file path mapping (Phase 4 migration)
//! - `graph_utils` - Shared graph utilities (forward/reverse graph building)
//! - `type_resolver` - Type resolution for method calls (Phase 7-8)
//! - `interner` - String interning for memory-efficient path storage (Phase 2 migration)
//! - `languages` - Language handler trait and registry (Phase 8)
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::callgraph::build_project_call_graph;
//! use tldr_core::callgraph::graph_utils::{build_forward_graph, build_reverse_graph};
//! use tldr_core::callgraph::type_resolver::{resolve_python_receiver_type, resolve_self_method};
//! use tldr_core::callgraph::interner::{StringInterner, PathInterner, InternedId};
//! use tldr_core::Language;
//!
//! let graph = build_project_call_graph(
//!     Path::new("src"),
//!     Language::Python,
//!     None,
//!     true,
//! )?;
//!
//! println!("Found {} call edges", graph.edge_count());
//!
//! // Build graph representations for analysis
//! let forward = build_forward_graph(&graph);
//! let reverse = build_reverse_graph(&graph);
//!
//! // Type-aware resolution
//! let (receiver_type, confidence) = resolve_python_receiver_type(
//!     source, call_line, "self", Some("Calculator")
//! );
//!
//! // String interning for memory efficiency
//! let mut interner = StringInterner::new();
//! let id1 = interner.intern("src/main.rs");
//! let id2 = interner.intern("src/main.rs"); // Same ID, no duplication
//! assert_eq!(id1, id2);
//! ```

pub mod builder;
pub mod cross_file_types;
pub mod graph_utils;
pub mod import_resolver;
pub mod interner;
pub mod languages;
pub mod module_index;
pub mod resolver;
pub mod type_aware_resolver;
pub mod type_resolver;

// Phase 14: Builder V2 sub-modules (modularization)
mod imports;
mod module_path;
mod resolution;
mod scanner;
mod types;
mod var_types; // call resolution logic (strategies 0-9)

// Phase 14: Builder V2 with parallel processing (canonical)
pub mod builder_v2;

// Phase 15: Serialization with versioning (experimental)
#[cfg(feature = "experimental_callgraph")]
pub mod serialization;

// Phase 16: API Compatibility layer (experimental)
#[cfg(feature = "experimental_callgraph")]
pub mod compat;

// NOTE: builder_v2_test.rs contains TDD tests for phases 14b-14f.
// Enabled for Phase 14b file discovery testing.
#[cfg(feature = "experimental_callgraph")]
#[cfg(test)]
mod builder_v2_test;

// NOTE: serialization_test.rs contains TDD tests for Phase 15.
#[cfg(feature = "experimental_callgraph")]
#[cfg(test)]
mod serialization_test;

// NOTE: compat_test.rs contains TDD tests for Phase 16.
#[cfg(feature = "experimental_callgraph")]
#[cfg(test)]
mod compat_test;

// NOTE: cross_file_test.rs contains TDD tests for cross-file call detection.
// These tests verify the behavioral specification in CROSSFILE_SPEC.md.
#[cfg(feature = "experimental_callgraph")]
#[cfg(test)]
mod cross_file_test;

// NOTE: hypotheses_test.rs contains discriminative tests for Tier-2 Fowler smells.
// These tests validate data availability in the call graph IR for smell detectors.
#[cfg(test)]
mod hypotheses_test;

pub use builder::build_project_call_graph;
pub use cross_file_types::{
    // Phase 3: Container types
    CallGraphIR,
    // Phase 1: Core IR types
    CallSite,
    CallType,
    ClassDef,
    // Phase 7: Cross-file resolution types
    CrossFileCallEdge,
    FileIR,
    FileIRBuilder,
    FuncDef,
    FuncIndexProxy,
    FuncIndexProxyMut,
    ImportDef,
    ImportKind,
    ModuleInfo,
    ProjectCallGraphV2,
    ReExportChain,
    ResolvedImport,
    VarType,
    IR_VERSION,
};
pub use graph_utils::{build_forward_graph, build_reverse_graph, collect_nodes};
pub use import_resolver::{
    CacheStats, ImportResolver, ReExportTracer, TracedReExport, DEFAULT_CACHE_SIZE,
    DEFAULT_MAX_DEPTH,
};
pub use languages::{
    base::{
        determine_call_type, get_node_text, make_import, normalize_path, read_source_safely,
        walk_tree,
    },
    extract_calls_for_language,
    parse_imports_for_language,
    BuildError,
    // Phase 8: Language handler trait and registry
    CallGraphLanguageSupport,
    LanguageRegistry,
    ParseError,
};
pub use module_index::{ModuleIndex, ModuleIndexError};
pub use resolver::ModuleResolver;
pub use type_resolver::{
    // Robustness utilities (Phase 10)
    expand_union_type,
    // Python resolution (Phase 8)
    find_enclosing_class,
    resolve_annotation,
    resolve_go_receiver_type,
    resolve_python_receiver_type,
    resolve_receiver_type,
    resolve_rust_receiver_type,
    resolve_self_method,
    // Multi-language resolution (Phase 9)
    resolve_typescript_receiver_type,
    safe_read_file,
    validate_path_utf8,
    ClassDefinition,
    ResolutionMethod,
    ResolvedType,
    SkipReason,
    TypeResolver,
    MAX_UNION_EXPANSION,
};

// Phase 14: Builder V2 exports (canonical)
pub use builder_v2::{
    build_project_call_graph_v2,
    filter_tldrignored,
    // Phase 14: Builder V2 types
    BuildConfig,
    BuildDiagnostics,
    // Re-export BuildError as BuildErrorV2 to avoid conflict with languages::BuildError
    BuildError as BuildErrorV2,
    BuildResult,
    ClassEntry,
    ClassIndex,
    FuncEntry,
    FuncIndex,
    ParseDiagnostic,
    ResolutionWarning,
    SkipReason as BuildSkipReason,
};

#[cfg(feature = "experimental_callgraph")]
pub use compat::{
    // Phase 16c: Unified entry point
    build_call_graph,
    callgraph_ir_to_old,
    callgraph_ir_to_v1,
    compare_builders,
    format_edges_compatible,
    // Phase 16: Conversion functions
    funcdef_to_functioninfo,
    importdef_to_importinfo,
    normalize_edge,
    project_graph_to_edges,
    CallEdge as CompatCallEdge,
    CallGraphOutput,
    ComparisonResult,
    // Phase 16: Compatibility layer types
    FunctionInfo,
    ImportInfo,
    NormalizedEdge,
};
