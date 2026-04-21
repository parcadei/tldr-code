//! CLI command implementations
//!
//! Each command is implemented as a separate module for maintainability.
//! All commands follow the same pattern:
//! 1. Define Args struct with clap derive
//! 2. Implement run() function that calls tldr-core
//! 3. Return Result<(), anyhow::Error>

pub mod calls;
pub mod dead;
pub mod impact;
pub mod structure;
pub mod tree;
// cfg, dfg: archived (T5 deep analysis)
pub mod churn;
pub mod complexity;
pub mod context;
pub mod debt;
pub mod detect_patterns;
pub mod extract;
pub mod health;
pub mod hubs;
pub mod importers;
pub mod imports;
pub mod search;
pub mod slice;
pub mod smells;
pub mod whatbreaks;
// Session 19: Pattern Analysis commands
pub mod change_impact;
pub mod clones;
pub mod deps;
pub mod diagnostics;
pub mod dice;
pub mod doctor;
pub mod inheritance;
pub mod patterns;
pub mod references;
// ssa, dominators, live_vars, alias, abstract_interp: archived (T5 deep analysis)
pub mod available;
pub mod reaching_defs;
pub mod taint;

// Session 15: Metrics commands
pub mod cognitive;
pub mod coverage;
pub mod halstead;
pub mod hotspots;
pub mod loc;

// Session 16: Semantic search commands
#[cfg(feature = "semantic")]
pub mod embed;
#[cfg(feature = "semantic")]
pub mod semantic;
#[cfg(feature = "semantic")]
pub mod similar;

// Daemon subsystem (Phase 1: types and error)
pub mod daemon;

// Daemon router for auto-routing commands through daemon cache
pub mod daemon_router;

// Contracts & Flow commands (Session 18) - behavioral contracts
pub mod contracts;

// API Surface command - structural contracts
pub mod api_surface;

// Remaining commands (todo, explain, secure, definition, diff, api-check)
pub mod remaining;

// Fix - error diagnosis and auto-fix system
pub mod fix;

// Bugbot - automated bug detection on code changes
pub mod bugbot;

// Re-export Args types for convenience
pub use calls::CallsArgs;
pub use dead::DeadArgs;
pub use impact::ImpactArgs;
pub use structure::StructureArgs;
pub use tree::TreeArgs;
// CfgArgs, DfgArgs: archived
pub use change_impact::ChangeImpactArgs;
pub use churn::ChurnArgs;
pub use clones::ClonesArgs;
pub use complexity::ComplexityArgs;
pub use context::ContextArgs;
pub use debt::DebtArgs;
pub use deps::DepsArgs;
pub use detect_patterns::PatternsArgs;
pub use diagnostics::DiagnosticsArgs;
pub use dice::DiceArgs;
pub use doctor::DoctorArgs;
pub use extract::ExtractArgs;
pub use health::HealthArgs;
pub use hubs::HubsArgs;
pub use importers::ImportersArgs;
pub use imports::ImportsArgs;
pub use inheritance::InheritanceArgs;
pub use references::ReferencesArgs;
pub use search::SmartSearchArgs;
pub use slice::SliceArgs;
pub use smells::SmellsArgs;
pub use whatbreaks::WhatbreaksArgs;
// SsaArgs, DominatorsArgs, LiveVarsArgs, AliasArgs, AbstractInterpArgs: archived
pub use available::AvailableArgs;
pub use reaching_defs::ReachingDefsArgs;
pub use taint::TaintArgs;

// Session 15: Metrics commands
pub use cognitive::CognitiveArgs;
pub use coverage::CoverageArgs;
pub use halstead::HalsteadArgs;
pub use hotspots::HotspotsArgs;
pub use loc::LocArgs;

// Session 16: Semantic search commands
#[cfg(feature = "semantic")]
pub use embed::EmbedArgs;
#[cfg(feature = "semantic")]
pub use semantic::SemanticArgs;
#[cfg(feature = "semantic")]
pub use similar::SimilarArgs;

// Daemon subsystem commands (Phase 5-6)
pub use daemon::{
    DaemonNotifyArgs, DaemonQueryArgs, DaemonStartArgs, DaemonStatusArgs, DaemonStopArgs,
};

// Cache commands (Phase 9)
pub use daemon::{CacheClearArgs, CacheStatsArgs};

// Warm and Stats commands (Phase 7-8)
pub use daemon::{StatsArgs, WarmArgs};

// Daemon router for auto-routing commands through daemon cache
pub use daemon_router::{
    is_daemon_running, params_for_dead, params_with_entry_depth, params_with_file,
    params_with_file_function, params_with_file_function_line, params_with_func_depth,
    params_with_module, params_with_path, params_with_path_lang, params_with_pattern,
    try_daemon_route, try_daemon_route_async,
};

// API Surface command Args
pub use api_surface::ApiSurfaceArgs;

// Contracts & Flow types (Session 18, Phase 1-4) - behavioral contracts
pub use contracts::{
    // Phase 6: chop command Args
    ChopArgs,
    ChopResult,
    Condition,
    // Core types
    Confidence,
    // Phase 3: behavioral contracts command Args
    ContractsArgs,
    ContractsError,
    // Report types
    ContractsReport,
    ContractsResult,
    CoverageInfo,
    DeadStore,
    // BoundsArgs: archived (T5 deep analysis)
    // Phase 5: dead-stores command Args
    DeadStoresArgs,
    DeadStoresReport,
    ExceptionSpec,
    FunctionInvariants,
    FunctionSpecs,
    // Spec types
    InputOutputSpec,
    // Analysis types
    Interval,
    IntervalWarning,
    Invariant,
    InvariantKind,
    // Phase 8: invariants command Args
    InvariantsArgs,
    InvariantsReport,
    InvariantsSummary,
    OutputFormat,
    PropertySpec,
    // Phase 7: specs command Args
    SpecsArgs,
    SpecsByType,
    SpecsReport,
    SpecsSummary,
    SubAnalysisResult,
    // Phase 9: verify command Args
    VerifyArgs,
    VerifyReport,
    VerifySummary,
};

// Remaining commands (Phase 4+)
pub use remaining::{
    // Diff Impact types (archived - superseded by change-impact)
    // ChangedFunction, DiffImpactReport, DiffImpactSummary,
    // API Check types
    APICheckReport,
    APICheckSummary,
    APIRule,
    // Diff types
    ASTChange,
    BaseChanges,
    // Explain types
    CallInfo,
    ChangeType,
    ComplexityInfo,
    // Graph utilities (TIGER-02)
    CycleDetector,
    // Definition types
    DefinitionResult,
    DiffGranularity,
    DiffReport,
    DiffSummary,
    ExplainReport,
    // Equivalence (GVN) types
    // Common types
    Location,
    MisuseCategory,
    MisuseFinding,
    MisuseSeverity,
    NodeKind,
    ParamInfo,
    PurityInfo,
    RemainingError,
    RemainingResult,
    // Secure types
    SecureFinding,
    SecureReport,
    SecureSummary,
    Severity,
    SignatureInfo,
    SymbolInfo,
    SymbolKind,
    // Todo types
    TodoItem,
    TodoReport,
    TodoSummary,
    TraversalResult,
};
pub use remaining::{DefinitionArgs, DiffArgs, ExplainArgs, SecureArgs, TodoArgs};

// Fix types
pub use fix::FixArgs;

// Bugbot types
pub use bugbot::BugbotCheckArgs;
