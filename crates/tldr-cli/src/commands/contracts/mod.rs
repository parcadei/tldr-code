//! Contracts & Flow commands for TLDR CLI
//!
//! This module provides commands for contract inference, behavioral specification
//! extraction, and program flow analysis. These commands help users understand
//! function contracts, invariants, and data flow paths through their code.
//!
//! # Commands
//!
//! - `contracts`: Infer pre/postconditions from guard clauses, assertions, isinstance checks
//! - `invariants`: Daikon-lite inference from test execution traces
//! - `specs`: Extract behavioral specifications from pytest test files
//! - `verify`: Aggregated verification dashboard combining multiple analyses
//! - `dead-stores`: SSA-based dead store detection
//! - `bounds`: Interval analysis tracking numeric value ranges
//! - `chop`: Program slice intersection (forward AND backward)
//!
//! # Module Structure
//!
//! ```text
//! contracts/
//! ├── mod.rs              # Module exports and re-exports (this file)
//! ├── types.rs            # Shared data types across all commands
//! ├── error.rs            # ContractsError enum and Result type
//! ├── contracts.rs        # contracts command implementation
//! ├── invariants.rs       # invariants command implementation
//! ├── specs.rs            # specs command implementation
//! ├── verify.rs           # verify command implementation
//! ├── dead_stores.rs      # dead-stores command implementation
//! ├── bounds.rs           # bounds command implementation
//! └── chop.rs             # chop command implementation
//! ```
//!
//! # Schema Version
//!
//! All JSON output includes a schema version for forward compatibility.
//! Current schema version: 1.0

pub mod error;
pub mod types;
pub mod validation;

// Phase 3: contracts command implementation
#[path = "contracts.rs"]
pub mod contracts_cmd;
pub use contracts_cmd as contracts;

// bounds: archived (T5 deep analysis)

// Phase 5: dead-stores command implementation
pub mod dead_stores;

// Phase 6: chop command implementation
pub mod chop;

// Phase 7: specs command implementation
pub mod specs;

// Phase 8: invariants command implementation (Daikon-lite)
pub mod invariants;

// Phase 9: verify command implementation
pub mod verify;

// Re-export core types for convenience
pub use error::{ContractsError, ContractsResult};
pub use types::{
    // Analysis result types
    ChopResult,
    // Confidence and conditions
    Condition,
    Confidence,
    // Report types
    ContractsReport,
    CoverageInfo,
    DeadStore,
    // BoundsResult: archived (T5 deep analysis)
    DeadStoresReport,
    // Spec types
    ExceptionSpec,
    FunctionInvariants,
    FunctionSpecs,
    InputOutputSpec,
    Interval,
    IntervalWarning,
    // Invariant types
    Invariant,
    InvariantKind,
    InvariantsReport,
    InvariantsSummary,
    // Output format
    OutputFormat,
    PropertySpec,
    SpecsByType,
    SpecsReport,
    SpecsSummary,
    SubAnalysisResult,
    SubAnalysisStatus,
    VerifyReport,
    VerifySummary,
};

// Phase 3: Re-export ContractsArgs for CLI integration
pub use contracts_cmd::ContractsArgs;

// BoundsArgs: archived (T5 deep analysis)

// Phase 5: Re-export DeadStoresArgs for CLI integration
pub use dead_stores::DeadStoresArgs;

// Phase 6: Re-export ChopArgs for CLI integration
pub use chop::ChopArgs;

// Phase 7: Re-export SpecsArgs for CLI integration
pub use specs::SpecsArgs;

// Phase 8: Re-export InvariantsArgs for CLI integration
pub use invariants::InvariantsArgs;

// Phase 9: Re-export VerifyArgs for CLI integration
pub use verify::VerifyArgs;

// Re-export validation utilities and constants
pub use validation::{
    check_ast_depth,
    // Depth checking utilities
    check_depth_limit,
    check_ssa_node_limit,
    has_path_traversal_pattern,
    read_file_safe,
    read_file_safe_with_warning,
    // Validation functions
    validate_file_path,
    validate_file_path_in_project,
    validate_function_name,
    validate_line_numbers,
    MAX_AST_DEPTH,
    MAX_CFG_DEPTH,
    MAX_CONDITIONS_PER_FUNCTION,
    // Resource limit constants (TIGER mitigations)
    MAX_FILE_SIZE,
    MAX_FUNCTION_NAME_LEN,
    MAX_SSA_NODES,
    WARN_FILE_SIZE,
};

/// Schema version for JSON output format.
/// Increment when output schema changes in incompatible ways.
pub const SCHEMA_VERSION: &str = "1.0";
