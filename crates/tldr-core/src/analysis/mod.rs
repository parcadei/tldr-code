//! Analysis module - Impact, Dead Code, Importers, Architecture, Change Impact, Hubs, References, Clones, Similarity
//!
//! This module provides analysis functions built on top of the call graph:
//!
//! - `impact` - Reverse call graph traversal to find callers
//! - `dead` - Find unreachable/dead code
//! - `importers` - Find files importing a module
//! - `architecture` - Detect architectural layers
//! - `change_impact` - Find tests affected by changed files
//! - `hubs` - Hub detection using centrality measures (in-degree, out-degree)
//! - `references` - Find all references to a symbol across the codebase
//! - `clones` - Code clone detection (Type-1, Type-2, Type-3)
//! - `similarity` - Code similarity analysis (Dice, Jaccard, Cosine)
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::analysis::{impact_analysis, dead_code_analysis, change_impact};
//! use tldr_core::analysis::hubs::{compute_in_degree, compute_out_degree, compute_hub_scores};
//! use tldr_core::callgraph::build_project_call_graph;
//!
//! let graph = build_project_call_graph(...)?;
//! let impact = impact_analysis(&graph, "process_data", 3, None)?;
//! let dead = dead_code_analysis(&graph, &functions, None)?;
//! let changes = change_impact(Path::new("src"), None, Language::Python)?;
//!
//! // Hub detection
//! let forward = build_forward_graph(&graph);
//! let reverse = build_reverse_graph(&graph);
//! let nodes = collect_nodes(&graph);
//! let hub_scores = compute_hub_scores(&nodes, &forward, &reverse);
//! ```

pub mod arch_rules;
pub mod architecture;
pub mod change_impact;
pub mod clones;
pub mod dead;
pub mod deps;
pub mod hubs;
pub mod impact;
pub mod importers;
pub mod refcount;
pub mod references;
pub mod similarity;
pub mod tarjan;
pub mod whatbreaks;

pub use arch_rules::{
    build_import_graph, check_rules, check_transitive_violations, generate_rules, ImportEdge,
    ImportGraph,
};
pub use architecture::{
    architecture_analysis, circular_deps_to_cycle_report, find_circular_dependencies_tarjan,
};
pub use change_impact::{
    change_impact, change_impact_extended, ChangeImpactMetadata, ChangeImpactReport,
    ChangeImpactStatus, DetectionMethod, TestFunction,
};
pub use clones::{
    classify_clone_type, compute_dice_similarity, compute_rolling_hashes, detect_clones,
    find_verified_clones, hash_token, interpret_similarity, is_generated_file, normalize_tokens,
    verify_clone_match, CloneClass, CloneConfig, CloneFragment, ClonePair, CloneStats, CloneType,
    ClonesOptions, ClonesReport, HashEntry, HashIndex, NormalizationMode, NormalizedToken,
    RollingHash, TokenCategory, TokenSequence, UnionFind,
};
pub use dead::{collect_all_functions, dead_code_analysis, dead_code_analysis_refcount};
pub use deps::{
    analyze_dependencies, classify_import, format_deps_dot, format_deps_text, is_go_stdlib,
    is_python_stdlib, is_rust_internal, is_rust_stdlib, is_typescript_external,
    is_typescript_relative, DepCycle, DepEdge, DepKind, DepNode, DepStats, DepsOptions, DepsReport,
};
pub use hubs::{
    compute_hub_report, compute_hub_scores, compute_in_degree, compute_out_degree, HubAlgorithm,
    HubReport, HubScore, RiskLevel,
};
pub use impact::{impact_analysis, impact_analysis_with_ast_fallback};
pub use importers::find_importers;
pub use references::{
    classify_reference_kind, find_references, find_text_candidates, verify_candidates_with_ast,
    Definition, DefinitionKind, Reference, ReferenceKind, ReferenceStats, ReferencesOptions,
    ReferencesReport, SearchScope, TextCandidate, VerifiedReference,
};
pub use similarity::{
    compute_pairwise_similarity, compute_similarity, dice_coefficient, dice_to_jaccard,
    interpret_similarity_score, jaccard_coefficient, jaccard_to_dice, parse_target,
    ComparisonLevel, PairwiseSimilarityEntry, PairwiseSimilarityReport, ParsedTarget,
    SimilarityConfig, SimilarityFragment, SimilarityMetric, SimilarityOptions, SimilarityReport,
    SimilarityScores, TokenBreakdown,
};
pub use tarjan::{detect_cycles, find_sccs, ToSccString};
pub use whatbreaks::{
    detect_target_type, whatbreaks_analysis, SubResult, SubStatus, TargetType, WhatbreaksOptions,
    WhatbreaksReport, WhatbreaksSummary,
};

// Test modules
#[cfg(test)]
mod callgraph_tests;

#[cfg(test)]
mod architecture_tests;

#[cfg(test)]
mod change_impact_tests;

#[cfg(test)]
mod deps_tests;

#[cfg(test)]
mod references_tests;

#[cfg(test)]
mod clones_tests;

#[cfg(test)]
mod dice_tests;

#[cfg(test)]
mod clones_integration_tests;
