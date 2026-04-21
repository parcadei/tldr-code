//! Remaining commands for TLDR CLI
//!
//! This module implements additional analysis commands:
//! - LOW: todo, explain, secure
//! - MEDIUM: definition, diff
//! - HIGH: api_check, equivalence, vuln
//!
//! # Module Structure
//!
//! - `types`: Shared data types for all remaining commands
//! - `error`: Error types and result aliases
//! - `ast_cache`: AST caching layer for efficient multi-analysis
//! - `graph_utils`: Cycle detection for graph traversal (TIGER-02)
//! - `todo`: Improvement aggregation command
//! - `explain`: Comprehensive function analysis command
//! - `secure`: Security analysis dashboard
//! - `definition`: Go-to-definition command
//! - `diff`: AST-aware structural diff
//! - `equivalence`: GVN-based redundancy detection command
//! - `api_check`: API misuse detection command
//! - `vuln`: Vulnerability detection via taint analysis

pub mod api_check;
pub mod ast_cache;
pub mod difftastic;
// equivalence: archived (T5 deep analysis)
pub mod definition;
pub mod diff;
pub mod vuln;
// diff_impact: archived (superseded by change-impact)
pub mod error;
pub mod explain;
pub mod graph_utils;
pub mod secure;
pub mod todo;
pub mod types;

// Re-export types for convenience
pub use error::{RemainingError, RemainingResult};
pub use types::{
    // Diff Impact types (archived - superseded by change-impact)
    // ChangedFunction, DiffImpactReport, DiffImpactSummary,
    // API Check types
    APICheckReport,
    APICheckSummary,
    APIRule,
    // Diff types
    ASTChange,
    // L8 Architecture-level types
    ArchChangeType,
    ArchDiffSummary,
    ArchLevelChange,
    BaseChanges,
    // Explain types
    CallInfo,
    ChangeType,
    ComplexityInfo,
    // Definition types
    DefinitionResult,
    DiffGranularity,
    DiffReport,
    DiffSummary,
    ExplainReport,
    // Equivalence (GVN) types (from types.rs)
    ExpressionRef,
    // L6 File-level types
    FileLevelChange,
    GVNEquivalence,
    GVNReport,
    GVNSummary,
    // L7 Module-level types
    ImportEdge,
    ImportGraphSummary,
    // Common types
    Location,
    MisuseCategory,
    MisuseFinding,
    MisuseSeverity,
    ModuleLevelChange,
    NodeKind,
    OutputFormat,
    ParamInfo,
    PurityInfo,
    Redundancy,
    // Secure types
    SecureFinding,
    SecureReport,
    SecureSummary,
    Severity,
    SignatureInfo,
    SymbolInfo,
    SymbolKind,
    // Vuln types
    TaintFlow,
    // Todo types
    TodoItem,
    TodoReport,
    TodoSummary,
    VulnFinding,
    VulnReport,
    VulnSummary,
    VulnType,
};

// Re-export graph utilities
pub use graph_utils::{
    CycleDetector, TraversalResult, VisitedSet, MAX_GRAPH_DEPTH, MAX_IMPORT_DEPTH,
};

// Re-export command Args
pub use api_check::ApiCheckArgs;
pub use definition::DefinitionArgs;
pub use diff::DiffArgs;
// DiffImpactArgs: archived (superseded by change-impact)
// EquivalenceArgs: archived (T5 deep analysis)
pub use explain::ExplainArgs;
pub use secure::SecureArgs;
pub use todo::TodoArgs;
pub use vuln::VulnArgs;
