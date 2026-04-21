//! Pattern Analysis commands for TLDR CLI
//!
//! This module provides commands for pattern analysis, including cohesion metrics,
//! coupling analysis, interface extraction, temporal constraint
//! mining, behavioral constraint extraction, and resource
//! lifecycle analysis.
//!
//! # Commands
//!
//! - `cohesion`: Compute LCOM4 (Lack of Cohesion of Methods) metric for classes
//! - `coupling`: Analyze coupling between two modules via cross-module calls
//! - `interface`: Extract the public interface (API surface) of Python files
//! - `temporal`: Mine temporal constraints (method call sequences) from a codebase
//! - `behavioral`: Extract behavioral constraints (pre/postconditions, exceptions)
//! - `resources`: Analyze resource lifecycle to detect leaks and issues
//!
//! # Module Structure
//!
//! ```text
//! patterns/
//! ├── mod.rs              # Module exports and re-exports (this file)
//! ├── types.rs            # Shared data types across all commands
//! ├── error.rs            # PatternsError enum and Result type
//! ├── validation.rs       # Path safety, resource limits (TIGER mitigations)
//! ├── cohesion.rs         # cohesion command implementation
//! ├── coupling.rs         # coupling command implementation
//! ├── interface.rs        # interface command implementation
//! ├── temporal.rs         # temporal command implementation
//! ├── behavioral.rs       # behavioral command implementation
//! └── resources.rs        # resources command implementation
//! ```
//!
//! # Schema Version
//!
//! All JSON output uses types from `types.rs` for consistent serialization.

// behavioral: archived (T5 deep analysis)
pub mod cohesion;
pub mod coupling;
pub mod error;
pub mod interface;
pub mod resources;
pub mod temporal;
pub mod types;
pub mod validation;

// Re-export core types for convenience
pub use error::{PatternsError, PatternsResult};
pub use types::{
    // Behavioral types
    BehavioralReport,
    ClassBehavior,
    // Cohesion types
    ClassCohesion,
    // Interface types
    ClassInfo,
    ClassInvariant,
    CohesionReport,
    CohesionSummary,
    CohesionVerdict,
    ComponentInfo,
    ConditionSource,
    // Enums
    Confidence,
    // Resource types
    ContextSuggestion,
    // Coupling types
    CouplingReport,
    CouplingVerdict,
    CrossCall,
    CrossCalls,
    DocstringStyle,
    DoubleCloseInfo,
    EffectType,
    ExceptionInfo,
    FunctionBehavior,
    FunctionInfo,
    InterfaceInfo,
    LeakInfo,
    MethodInfo,
    OutputFormat,
    Postcondition,
    Precondition,
    ResourceConstraint,
    ResourceInfo,
    ResourceReport,
    ResourceSummary,
    SideEffect,
    // Temporal types
    TemporalConstraint,
    TemporalExample,
    TemporalMetadata,
    TemporalReport,
    Trigram,
    UseAfterCloseInfo,
    YieldInfo,
};

// Re-export Args types for CLI integration
// BehavioralArgs: archived (T5 deep analysis)
pub use cohesion::CohesionArgs;
pub use coupling::CouplingArgs;
pub use interface::InterfaceArgs;
pub use resources::ResourcesArgs;
pub use temporal::TemporalArgs;

/// Schema version for JSON output format.
/// Increment when output schema changes in incompatible ways.
pub const SCHEMA_VERSION: &str = "1.0";

// Phase 2: Re-export validation utilities (TIGER mitigations)
pub use validation::{
    // Depth checking (TIGER-03)
    check_analysis_depth,
    check_ast_depth,
    check_directory_file_count,
    // Path validation (TIGER-01, TIGER-02)
    is_path_traversal_attempt,
    // File validation (TIGER-08)
    read_file_safe,
    // Checked arithmetic (TIGER-03)
    saturating_count_add,
    saturating_depth_increment,
    validate_directory_path,
    validate_file_path,
    validate_file_path_in_project,
    validate_file_size,
    // Function validation
    validate_function_name,
    within_limit,
    // Constants (TIGER-08 mitigations)
    MAX_ANALYSIS_DEPTH,
    MAX_AST_DEPTH,
    MAX_CLASSES_PER_FILE,
    MAX_CLASS_COMPLEXITY,
    MAX_CONSTRAINTS_PER_FILE,
    MAX_DIRECTORY_FILES,
    MAX_FIELDS_PER_CLASS,
    MAX_FILE_SIZE,
    MAX_FUNCTION_NAME_LEN,
    MAX_METHODS_PER_CLASS,
    MAX_PATHS,
    MAX_TRIGRAMS,
    WARN_FILE_SIZE,
};
