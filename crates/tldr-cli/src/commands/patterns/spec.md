# Pattern Analysis Commands - Rust Port Specification

**Created:** 2026-02-04
**Author:** architect-agent
**Source:** Python v1 at `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr/cli/commands/`
**Target:** `/Users/cosimo/.opc-dev/opc/packages/tldr-code/tldr-rs/crates/tldr-cli/src/commands/patterns/`

## Overview

This specification defines the Rust port of 8 TLDR commands for pattern analysis, including behavioral constraint extraction, cohesion metrics, coupling analysis, mutability tracking, purity analysis, temporal constraint mining, interface extraction, and resource lifecycle analysis.

These commands help users understand code quality, design patterns, and potential issues in their Python codebase. All commands follow established patterns from the contracts module.

## Module Architecture

```
patterns/
├── mod.rs              # Module exports and re-exports
├── types.rs            # Shared data types across all commands
├── error.rs            # PatternsError enum and Result type
├── validation.rs       # Path safety, resource limits (TIGER mitigations)
├── behavioral.rs       # behavioral command - pre/postcondition extraction
├── cohesion.rs         # cohesion command - LCOM4 class cohesion
├── coupling.rs         # coupling command - pairwise module coupling
├── mutability.rs       # mutability command - variable/parameter mutation
├── purity.rs           # purity command - effect/purity analysis
├── temporal.rs         # temporal command - temporal constraint mining
├── interface.rs        # interface command - public API extraction
└── resources.rs        # resources command - resource lifecycle analysis
```

## Shared Types (`types.rs`)

### Confidence Level

```rust
use serde::{Deserialize, Serialize};

/// Confidence level for inferred patterns and analysis results.
///
/// # Serialization
///
/// Serializes to snake_case: `"high"`, `"medium"`, `"low"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// High confidence - direct code evidence
    High,
    /// Medium confidence - inferred from patterns
    Medium,
    /// Low confidence - heuristic or partial evidence
    Low,
}

impl Default for Confidence {
    fn default() -> Self {
        Self::Medium
    }
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
        }
    }
}
```

### Docstring Style

```rust
/// Documentation style detected in source code.
///
/// Used by behavioral analysis to parse docstrings correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocstringStyle {
    /// Google-style docstrings (Args:, Returns:, Raises:)
    Google,
    /// NumPy-style docstrings (Parameters, Returns sections)
    Numpy,
    /// Sphinx/reST style docstrings (:param:, :returns:, :raises:)
    Sphinx,
    /// Plain docstrings without structured sections
    Plain,
}

impl Default for DocstringStyle {
    fn default() -> Self {
        Self::Plain
    }
}

impl std::fmt::Display for DocstringStyle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Google => write!(f, "google"),
            Self::Numpy => write!(f, "numpy"),
            Self::Sphinx => write!(f, "sphinx"),
            Self::Plain => write!(f, "plain"),
        }
    }
}
```

### Effect Type

```rust
/// Type of side effect detected in code.
///
/// Used by purity and behavioral analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectType {
    /// I/O operations (file, network, console)
    Io,
    /// Writing to global variables
    GlobalWrite,
    /// Writing to object attributes (self.x = ...)
    AttributeWrite,
    /// Modifying collections in place (list.append, dict.update)
    CollectionModify,
    /// Calling functions with potential side effects
    Call,
}

impl std::fmt::Display for EffectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io => write!(f, "io"),
            Self::GlobalWrite => write!(f, "global_write"),
            Self::AttributeWrite => write!(f, "attribute_write"),
            Self::CollectionModify => write!(f, "collection_modify"),
            Self::Call => write!(f, "call"),
        }
    }
}
```

### Condition Source

```rust
/// Source of a pre/postcondition constraint.
///
/// Tracks where a constraint was extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionSource {
    /// Guard clause (if x < 0: raise ValueError)
    Guard,
    /// Docstring description
    Docstring,
    /// Type hint annotation
    TypeHint,
    /// Assert statement
    Assertion,
    /// icontract decorator (@require, @ensure)
    Icontract,
    /// deal library decorator
    Deal,
}

impl std::fmt::Display for ConditionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard => write!(f, "guard"),
            Self::Docstring => write!(f, "docstring"),
            Self::TypeHint => write!(f, "type_hint"),
            Self::Assertion => write!(f, "assertion"),
            Self::Icontract => write!(f, "icontract"),
            Self::Deal => write!(f, "deal"),
        }
    }
}
```

### Cohesion Types

```rust
/// Information about a connected component in LCOM4 analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentInfo {
    /// Methods in this component
    pub methods: Vec<String>,
    /// Fields accessed by this component
    pub fields: Vec<String>,
}

/// Cohesion analysis result for a single class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassCohesion {
    /// Class name
    pub class_name: String,
    /// File path where class is defined
    pub file_path: String,
    /// Line number of class definition
    pub line: u32,
    /// LCOM4 value (1 = cohesive, >1 = split candidate)
    pub lcom4: u32,
    /// Number of methods analyzed
    pub method_count: u32,
    /// Number of fields detected
    pub field_count: u32,
    /// Verdict based on LCOM4 value
    pub verdict: CohesionVerdict,
    /// Suggestion for splitting if LCOM4 > 1
    pub split_suggestion: Option<String>,
    /// Connected components (if LCOM4 > 1)
    pub components: Vec<ComponentInfo>,
}

/// Verdict for cohesion analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohesionVerdict {
    /// Class is cohesive (LCOM4 = 1)
    Cohesive,
    /// Class could be split (LCOM4 > 1)
    SplitCandidate,
}

/// Summary of cohesion analysis across multiple classes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionSummary {
    /// Total classes analyzed
    pub total_classes: u32,
    /// Number of cohesive classes
    pub cohesive: u32,
    /// Number of split candidates
    pub split_candidates: u32,
    /// Average LCOM4 value
    pub avg_lcom4: f64,
}

/// Full report from cohesion analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionReport {
    /// Cohesion results per class
    pub classes: Vec<ClassCohesion>,
    /// Summary statistics
    pub summary: CohesionSummary,
}
```

### Coupling Types

```rust
/// A single cross-module function call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossCall {
    /// Function making the call
    pub caller: String,
    /// Function being called
    pub callee: String,
    /// Line number of the call
    pub line: u32,
}

/// Calls from one module to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossCalls {
    /// Individual call sites
    pub calls: Vec<CrossCall>,
    /// Total count of calls
    pub count: u32,
}

/// Coupling verdict based on score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CouplingVerdict {
    /// Low coupling (0.0-0.2)
    Low,
    /// Moderate coupling (0.2-0.4)
    Moderate,
    /// High coupling (0.4-0.6)
    High,
    /// Very high coupling (0.6-1.0)
    VeryHigh,
}

impl CouplingVerdict {
    /// Determine verdict from coupling score.
    pub fn from_score(score: f64) -> Self {
        if score < 0.2 {
            Self::Low
        } else if score < 0.4 {
            Self::Moderate
        } else if score < 0.6 {
            Self::High
        } else {
            Self::VeryHigh
        }
    }
}

/// Coupling analysis between two modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouplingReport {
    /// Path to first module
    pub path_a: String,
    /// Path to second module
    pub path_b: String,
    /// Calls from A to B
    pub a_to_b: CrossCalls,
    /// Calls from B to A
    pub b_to_a: CrossCalls,
    /// Total cross-module calls
    pub total_calls: u32,
    /// Coupling score (0.0-1.0)
    pub coupling_score: f64,
    /// Verdict based on score
    pub verdict: CouplingVerdict,
}
```

### Purity Types

```rust
/// Purity analysis result for a single function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionPurity {
    /// Function name
    pub name: String,
    /// Whether the function is pure (no side effects)
    pub pure: bool,
    /// List of detected effects (empty if pure)
    pub effects: Vec<String>,
    /// Confidence level of the analysis
    pub confidence: Confidence,
}

/// Purity report for a single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilePurityReport {
    /// Source file path
    pub source_file: String,
    /// Purity results per function
    pub functions: Vec<FunctionPurity>,
    /// Count of pure functions
    pub pure_count: u32,
}

/// Full purity report (may include multiple files for directory analysis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurityReport {
    /// Per-file reports
    pub files: Vec<FilePurityReport>,
    /// Total functions analyzed
    pub total_functions: u32,
    /// Total pure functions
    pub total_pure: u32,
}
```

### Temporal Types

```rust
/// Example location for a temporal constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalExample {
    /// File where the pattern was observed
    pub file: String,
    /// Line number
    pub line: u32,
}

/// A temporal constraint (before -> after sequence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalConstraint {
    /// Method that must come first
    pub before: String,
    /// Method that must come after
    pub after: String,
    /// Number of times this pattern was observed
    pub support: u32,
    /// Confidence (support / total sequences containing 'before')
    pub confidence: f64,
    /// Example locations where this pattern appears
    pub examples: Vec<TemporalExample>,
}

/// A trigram (3-method sequence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trigram {
    /// The 3-method sequence
    pub sequence: [String; 3],
    /// Number of observations
    pub support: u32,
    /// Confidence score
    pub confidence: f64,
}

/// Metadata about temporal mining.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalMetadata {
    /// Number of files analyzed
    pub files_analyzed: u32,
    /// Total sequences extracted
    pub sequences_extracted: u32,
    /// Minimum support threshold used
    pub min_support: u32,
    /// Minimum confidence threshold used
    pub min_confidence: f64,
}

/// Full temporal constraint report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemporalReport {
    /// Discovered temporal constraints
    pub constraints: Vec<TemporalConstraint>,
    /// Discovered trigrams (if requested)
    pub trigrams: Vec<Trigram>,
    /// Analysis metadata
    pub metadata: TemporalMetadata,
}
```

### Interface Types

```rust
/// Information about a public function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionInfo {
    /// Function name
    pub name: String,
    /// Full signature (e.g., "def foo(x: int, y: str) -> bool")
    pub signature: String,
    /// Docstring if present
    pub docstring: Option<String>,
    /// Line number of definition
    pub lineno: u32,
    /// Whether the function is async
    pub is_async: bool,
}

/// Information about a public method within a class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodInfo {
    /// Method name
    pub name: String,
    /// Full signature
    pub signature: String,
    /// Whether the method is async
    pub is_async: bool,
}

/// Information about a public class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassInfo {
    /// Class name
    pub name: String,
    /// Line number of definition
    pub lineno: u32,
    /// Base classes
    pub bases: Vec<String>,
    /// Public methods
    pub methods: Vec<MethodInfo>,
    /// Count of private methods (for completeness)
    pub private_method_count: u32,
}

/// Interface (public API) for a single file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterfaceInfo {
    /// File path
    pub file: String,
    /// Contents of __all__ if defined
    pub all_exports: Option<Vec<String>>,
    /// Public functions
    pub functions: Vec<FunctionInfo>,
    /// Public classes
    pub classes: Vec<ClassInfo>,
}
```

### Resource Types

```rust
/// Information about a detected resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// Variable name holding the resource
    pub name: String,
    /// Type of resource (e.g., "file", "socket", "connection")
    pub resource_type: String,
    /// Line where resource was created/opened
    pub line: u32,
    /// Whether the resource is properly closed
    pub closed: bool,
}

/// Information about a potential resource leak.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeakInfo {
    /// Resource that may be leaked
    pub resource: String,
    /// Line where resource was created
    pub line: u32,
    /// Paths to the leak (if --show-paths enabled)
    pub paths: Option<Vec<String>>,
}

/// Information about a double-close issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoubleCloseInfo {
    /// Resource being closed twice
    pub resource: String,
    /// Line of first close
    pub first_close: u32,
    /// Line of second close
    pub second_close: u32,
}

/// Information about use-after-close issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseAfterCloseInfo {
    /// Resource being used after close
    pub resource: String,
    /// Line where resource was closed
    pub close_line: u32,
    /// Line where resource is used after close
    pub use_line: u32,
}

/// Suggestion for using context manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSuggestion {
    /// Resource that should use context manager
    pub resource: String,
    /// Suggested code pattern
    pub suggestion: String,
}

/// LLM-ready constraint from resource analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceConstraint {
    /// The constraint rule
    pub rule: String,
    /// Context where it applies
    pub context: String,
    /// Confidence level
    pub confidence: f64,
}

/// Summary of resource analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSummary {
    /// Total resources detected
    pub resources_detected: u32,
    /// Number of leaks found
    pub leaks_found: u32,
    /// Number of double-close issues
    pub double_closes_found: u32,
    /// Number of use-after-close issues
    pub use_after_closes_found: u32,
}

/// Full resource analysis report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceReport {
    /// File analyzed
    pub file: String,
    /// Language
    pub language: String,
    /// Function analyzed (if specific function requested)
    pub function: Option<String>,
    /// Detected resources
    pub resources: Vec<ResourceInfo>,
    /// Potential leaks
    pub leaks: Vec<LeakInfo>,
    /// Double-close issues
    pub double_closes: Vec<DoubleCloseInfo>,
    /// Use-after-close issues
    pub use_after_closes: Vec<UseAfterCloseInfo>,
    /// Context manager suggestions
    pub suggestions: Vec<ContextSuggestion>,
    /// LLM constraints (if --constraints enabled)
    pub constraints: Vec<ResourceConstraint>,
    /// Summary statistics
    pub summary: ResourceSummary,
    /// Analysis time in milliseconds
    pub analysis_time_ms: u64,
}
```

### Behavioral Types

```rust
/// A precondition on a function parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Precondition {
    /// Parameter name
    pub param: String,
    /// Constraint expression (e.g., "x > 0")
    pub expression: Option<String>,
    /// Human-readable description from docstring
    pub description: Option<String>,
    /// Type hint if present
    pub type_hint: Option<String>,
    /// Source of this condition
    pub source: ConditionSource,
}

/// A postcondition on function return.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Postcondition {
    /// Constraint expression
    pub expression: Option<String>,
    /// Human-readable description
    pub description: Option<String>,
    /// Return type hint
    pub type_hint: Option<String>,
}

/// Information about an exception the function may raise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExceptionInfo {
    /// Exception type (e.g., "ValueError")
    pub exception_type: String,
    /// Description of when it's raised
    pub description: Option<String>,
}

/// Information about yield values (for generators).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YieldInfo {
    /// Type hint for yielded values
    pub type_hint: Option<String>,
    /// Description of yielded values
    pub description: Option<String>,
}

/// Side effect detected in function.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffect {
    /// Type of effect
    pub effect_type: EffectType,
    /// Target of the effect (e.g., "self.count", "global_var")
    pub target: Option<String>,
}

/// Behavioral analysis for a single function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionBehavior {
    /// Function name
    pub function_name: String,
    /// File path
    pub file_path: String,
    /// Line number of function definition
    pub line: u32,
    /// Whether the function is pure
    pub is_pure: bool,
    /// Whether it's a generator
    pub is_generator: bool,
    /// Whether it's an async function
    pub is_async: bool,
    /// Preconditions on parameters
    pub preconditions: Vec<Precondition>,
    /// Postconditions on return
    pub postconditions: Vec<Postcondition>,
    /// Exceptions that may be raised
    pub exceptions: Vec<ExceptionInfo>,
    /// Yield information (if generator)
    pub yields: Vec<YieldInfo>,
    /// Detected side effects
    pub side_effects: Vec<SideEffect>,
}

/// Class invariant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassInvariant {
    /// Invariant expression
    pub expression: String,
}

/// Behavioral analysis for a class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassBehavior {
    /// Class name
    pub class_name: String,
    /// Class invariants
    pub invariants: Vec<ClassInvariant>,
    /// Method behaviors
    pub methods: Vec<FunctionBehavior>,
}

/// Full behavioral analysis report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehavioralReport {
    /// File analyzed
    pub file_path: String,
    /// Detected docstring style
    pub docstring_style: DocstringStyle,
    /// Whether icontract library is used
    pub has_icontract: bool,
    /// Whether deal library is used
    pub has_deal: bool,
    /// Function behaviors
    pub functions: Vec<FunctionBehavior>,
    /// Class behaviors
    pub classes: Vec<ClassBehavior>,
}
```

### Mutability Types

```rust
/// Mutability information for a variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariableMutability {
    /// Variable name
    pub name: String,
    /// Whether the variable is ever reassigned
    pub mutable: bool,
    /// Number of reassignments
    pub reassignments: u32,
    /// Number of in-place mutations
    pub mutations: u32,
}

/// Mutability information for a function parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterMutability {
    /// Parameter name
    pub name: String,
    /// Whether the parameter is mutated
    pub mutated: bool,
    /// Lines where mutation occurs
    pub mutation_sites: Vec<u32>,
}

/// Collection mutation detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionMutation {
    /// Variable being mutated
    pub variable: String,
    /// Operation (e.g., "append", "update", "pop")
    pub operation: String,
    /// Line number
    pub line: u32,
}

/// Mutability analysis for a function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionMutability {
    /// Function name
    pub name: String,
    /// Variable mutability info
    pub variables: Vec<VariableMutability>,
    /// Parameter mutability info
    pub parameters: Vec<ParameterMutability>,
    /// Collection mutations
    pub collection_mutations: Vec<CollectionMutation>,
}

/// Field mutability for a class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMutability {
    /// Field name
    pub name: String,
    /// Whether the field is mutable after __init__
    pub mutable: bool,
    /// Whether the field is only set in __init__
    pub init_only: bool,
}

/// Mutability analysis for a class.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassMutability {
    /// Class name
    pub name: String,
    /// Field mutability info
    pub fields: Vec<FieldMutability>,
}

/// Summary of mutability analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutabilitySummary {
    /// Functions analyzed
    pub functions_analyzed: u32,
    /// Classes analyzed
    pub classes_analyzed: u32,
    /// Total variables
    pub total_variables: u32,
    /// Mutable variables
    pub mutable_variables: u32,
    /// Immutable variables
    pub immutable_variables: u32,
    /// Percentage of immutable variables
    pub immutable_pct: f64,
    /// Parameters analyzed
    pub parameters_analyzed: u32,
    /// Mutated parameters
    pub mutated_parameters: u32,
    /// Percentage of unmutated parameters
    pub unmutated_pct: f64,
    /// Fields analyzed
    pub fields_analyzed: u32,
    /// Mutable fields
    pub mutable_fields: u32,
    /// Constraints generated (if --constraints)
    pub constraints_generated: u32,
}

/// Full mutability report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutabilityReport {
    /// File analyzed
    pub file: String,
    /// Language
    pub language: String,
    /// Function mutability results
    pub functions: Vec<FunctionMutability>,
    /// Class mutability results
    pub classes: Vec<ClassMutability>,
    /// Summary statistics
    pub summary: MutabilitySummary,
    /// Analysis time in milliseconds
    pub analysis_time_ms: u64,
}
```

---

## Error Types (`error.rs`)

```rust
use std::path::PathBuf;
use thiserror::Error;

/// Errors specific to pattern analysis commands.
#[derive(Debug, Error)]
pub enum PatternsError {
    /// Source file not found.
    #[error("file not found: {}", path.display())]
    FileNotFound { path: PathBuf },

    /// Function not found in source file.
    #[error("function '{function}' not found in {}", file.display())]
    FunctionNotFound { function: String, file: PathBuf },

    /// Class not found in source file.
    #[error("class '{class_name}' not found in {}", file.display())]
    ClassNotFound { class_name: String, file: PathBuf },

    /// Parse error in source file.
    #[error("parse error in {}: {message}", file.display())]
    ParseError { file: PathBuf, message: String },

    /// File too large to analyze.
    #[error("file too large: {} ({bytes} bytes, max {max_bytes} bytes)", path.display())]
    FileTooLarge { path: PathBuf, bytes: u64, max_bytes: u64 },

    /// Directory scan limit exceeded.
    #[error("directory scan limit exceeded: {count} files found, max {max_files}")]
    TooManyFiles { count: u32, max_files: u32 },

    /// Analysis depth limit exceeded.
    #[error("analysis depth limit exceeded: depth {depth}, max {max_depth}")]
    DepthLimitExceeded { depth: u32, max_depth: u32 },

    /// Analysis timed out.
    #[error("analysis timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },

    /// Invalid parameter value.
    #[error("invalid parameter: {message}")]
    InvalidParameter { message: String },

    /// Path traversal attempt detected.
    #[error("path traversal blocked: {} attempts to escape project root", path.display())]
    PathTraversal { path: PathBuf },

    /// Unsupported language.
    #[error("unsupported language: {language} (only Python is supported)")]
    UnsupportedLanguage { language: String },

    /// No constraints found (not an error, but special exit code).
    #[error("no constraints found matching criteria")]
    NoConstraintsFound,

    /// Issues found (for resources command).
    #[error("resource issues found: {leaks} leaks, {double_closes} double-closes, {use_after_closes} use-after-close")]
    IssuesFound {
        leaks: u32,
        double_closes: u32,
        use_after_closes: u32,
    },

    /// Generic IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for pattern analysis commands.
pub type PatternsResult<T> = Result<T, PatternsError>;

impl PatternsError {
    /// Create a FileNotFound error.
    pub fn file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Create a FunctionNotFound error.
    pub fn function_not_found(function: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Self::FunctionNotFound {
            function: function.into(),
            file: file.into(),
        }
    }

    /// Create a ParseError.
    pub fn parse_error(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::ParseError {
            file: file.into(),
            message: message.into(),
        }
    }
}
```

---

## Validation (`validation.rs`)

```rust
//! Input validation and path safety utilities for Pattern Analysis commands.
//!
//! Provides security-focused validation functions to mitigate:
//! - **TIGER-02**: Path traversal attacks via malicious file paths
//! - **TIGER-03**: Unbounded recursion in analysis
//! - **TIGER-04**: Memory exhaustion from large files
//! - **TIGER-08**: Stack overflow from deeply nested ASTs
//!
//! All file paths are canonicalized and checked against project boundaries.

use std::fs;
use std::path::{Path, PathBuf};

use super::error::{PatternsError, PatternsResult};

// =============================================================================
// Resource Limits (TIGER Mitigations)
// =============================================================================

/// Maximum file size for analysis (10 MB).
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Warning threshold for file size (1 MB).
pub const WARN_FILE_SIZE: u64 = 1024 * 1024;

/// Maximum files to scan in directory analysis.
pub const MAX_DIRECTORY_FILES: u32 = 1000;

/// Maximum AST traversal depth.
pub const MAX_AST_DEPTH: usize = 100;

/// Maximum recursion depth for analysis algorithms.
pub const MAX_ANALYSIS_DEPTH: usize = 500;

/// Maximum function name length.
pub const MAX_FUNCTION_NAME_LEN: usize = 256;

/// Maximum constraints to report per file.
pub const MAX_CONSTRAINTS_PER_FILE: usize = 500;

// =============================================================================
// Blocked System Directories
// =============================================================================

const BLOCKED_PREFIXES: &[&str] = &[
    "/etc/",
    "/etc/passwd",
    "/etc/shadow",
    "/root/",
    "/sys/",
    "/proc/",
    "/dev/",
    "/var/run/",
    "/var/log/",
    "/private/etc/",
    "C:\\Windows\\",
    "C:\\System32\\",
];

// =============================================================================
// Path Validation
// =============================================================================

/// Validate and canonicalize a file path.
///
/// # Security
///
/// - Canonicalizes path (resolves symlinks, `.`, `..`)
/// - Rejects system directories
/// - Validates UTF-8 encoding
pub fn validate_file_path(path: &Path) -> PatternsResult<PathBuf> {
    if !path.exists() {
        return Err(PatternsError::FileNotFound {
            path: path.to_path_buf(),
        });
    }

    let canonical = fs::canonicalize(path).map_err(|_| PatternsError::FileNotFound {
        path: path.to_path_buf(),
    })?;

    let canonical_str = canonical.to_string_lossy();
    for blocked in BLOCKED_PREFIXES {
        if canonical_str.starts_with(blocked) || canonical_str == blocked.trim_end_matches('/') {
            return Err(PatternsError::PathTraversal {
                path: path.to_path_buf(),
            });
        }
    }

    if canonical.to_str().is_none() {
        return Err(PatternsError::PathTraversal {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Validate a file path ensuring it stays within a project root.
pub fn validate_file_path_in_project(
    path: &Path,
    project_root: &Path,
) -> PatternsResult<PathBuf> {
    let canonical = validate_file_path(path)?;

    let canonical_root = fs::canonicalize(project_root).map_err(|_| PatternsError::FileNotFound {
        path: project_root.to_path_buf(),
    })?;

    if !canonical.starts_with(&canonical_root) {
        return Err(PatternsError::PathTraversal {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Check if a path contains path traversal patterns.
pub fn has_path_traversal_pattern(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    path_str.contains("..") || path_str.contains('\0')
}

// =============================================================================
// Function Name Validation
// =============================================================================

/// Validate a function name for safety.
pub fn validate_function_name(name: &str) -> PatternsResult<()> {
    if name.is_empty() {
        return Err(PatternsError::InvalidParameter {
            message: "function name cannot be empty".to_string(),
        });
    }

    if name.len() > MAX_FUNCTION_NAME_LEN {
        return Err(PatternsError::InvalidParameter {
            message: format!(
                "function name too long ({} chars, max {})",
                name.len(),
                MAX_FUNCTION_NAME_LEN
            ),
        });
    }

    let suspicious_chars = [';', '(', ')', '{', '}', '[', ']', '`', '"', '\'', '\\', '/', '\0'];
    for c in name.chars() {
        if suspicious_chars.contains(&c) {
            return Err(PatternsError::InvalidParameter {
                message: format!("function name contains invalid character: '{}'", c),
            });
        }
    }

    if let Some(first) = name.chars().next() {
        if !first.is_alphabetic() && first != '_' {
            return Err(PatternsError::InvalidParameter {
                message: "function name must start with letter or underscore".to_string(),
            });
        }
    }

    Ok(())
}

// =============================================================================
// Safe File Reading
// =============================================================================

/// Safely read a file with size limits and UTF-8 validation.
pub fn read_file_safe(path: &Path) -> PatternsResult<String> {
    let canonical = validate_file_path(path)?;

    let metadata = fs::metadata(&canonical)?;
    let size = metadata.len();

    if size > MAX_FILE_SIZE {
        return Err(PatternsError::FileTooLarge {
            path: path.to_path_buf(),
            bytes: size,
            max_bytes: MAX_FILE_SIZE,
        });
    }

    let content = fs::read(&canonical)?;

    String::from_utf8(content).map_err(|_| PatternsError::ParseError {
        file: path.to_path_buf(),
        message: "file is not valid UTF-8".to_string(),
    })
}

// =============================================================================
// Depth Checking
// =============================================================================

/// Check if analysis depth limit has been exceeded.
pub fn check_analysis_depth(current_depth: usize) -> PatternsResult<()> {
    if current_depth >= MAX_ANALYSIS_DEPTH {
        Err(PatternsError::DepthLimitExceeded {
            depth: current_depth as u32,
            max_depth: MAX_ANALYSIS_DEPTH as u32,
        })
    } else {
        Ok(())
    }
}

/// Check if file count limit has been exceeded.
pub fn check_file_count(count: u32, max_files: u32) -> PatternsResult<()> {
    if count > max_files {
        Err(PatternsError::TooManyFiles { count, max_files })
    } else {
        Ok(())
    }
}
```

---

## Command Specifications

### 1. behavioral

**Purpose:** Extract behavioral constraints (pre/postconditions, exceptions, side effects) from Python source code.

#### CLI Interface

```
tldr behavioral <file> [function] [OPTIONS]

Arguments:
  <file>      Python source file to analyze (required)
  [function]  Specific function to analyze (optional - analyzes all if not specified)

Options:
  -f, --format <FORMAT>    Output format [default: json] [possible values: json, text]
  --constraints            Generate LLM-ready constraints
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct BehavioralArgs {
    /// Python source file to analyze
    pub file: PathBuf,

    /// Specific function to analyze (optional)
    pub function: Option<String>,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,

    /// Generate LLM-ready constraints
    #[arg(long)]
    pub constraints: bool,
}
```

#### Algorithm

1. Parse source file with tree-sitter-python
2. Detect docstring style (Google, NumPy, Sphinx, plain)
3. Check for icontract/deal imports
4. For each function:
   a. Extract preconditions from:
      - Guard clauses (`if x < 0: raise`)
      - Assertions (`assert isinstance(x, str)`)
      - Type hints
      - Docstring Args/Parameters section
   b. Extract postconditions from:
      - Return type hints
      - Assertions after `result =`
      - Docstring Returns section
   c. Extract exceptions from:
      - Explicit `raise` statements
      - Docstring Raises section
   d. Detect side effects by analyzing writes

#### Output Schema (JSON)

```json
{
  "file_path": "string",
  "docstring_style": "google|numpy|sphinx|plain",
  "has_icontract": false,
  "has_deal": false,
  "functions": [
    {
      "function_name": "string",
      "file_path": "string",
      "line": 10,
      "is_pure": true,
      "is_generator": false,
      "is_async": false,
      "preconditions": [
        {
          "param": "x",
          "expression": "x > 0",
          "description": "must be positive",
          "type_hint": "int",
          "source": "guard"
        }
      ],
      "postconditions": [...],
      "exceptions": [
        {
          "exception_type": "ValueError",
          "description": "if x is negative"
        }
      ],
      "yields": [...],
      "side_effects": [
        {
          "effect_type": "io",
          "target": "stdout"
        }
      ]
    }
  ],
  "classes": [
    {
      "class_name": "MyClass",
      "invariants": [{"expression": "self.count >= 0"}],
      "methods": [...]
    }
  ]
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File not found or parse error |
| 2 | Invalid arguments |

#### Example Usage

```bash
# Analyze all functions in a file
tldr behavioral src/utils.py

# Analyze specific function
tldr behavioral src/utils.py process_data

# Generate LLM constraints
tldr behavioral src/utils.py --constraints

# Text output
tldr behavioral src/utils.py -f text
```

---

### 2. cohesion

**Purpose:** Compute LCOM4 (Lack of Cohesion of Methods) metric for classes.

LCOM4=1 means the class is cohesive. LCOM4>1 suggests the class could be split into multiple classes.

#### CLI Interface

```
tldr cohesion <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  --min-methods <N>        Minimum methods for analysis [default: 2]
  --include-dunder         Include dunder methods (__init__, etc.)
  -f, --format <FORMAT>    Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct CohesionArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Minimum methods for a class to be analyzed
    #[arg(long, default_value = "2")]
    pub min_methods: u32,

    /// Include dunder methods in analysis
    #[arg(long)]
    pub include_dunder: bool,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Algorithm: LCOM4 via Union-Find

1. Parse class, extract methods and field accesses (`self.x`)
2. Build bipartite graph: methods <-> fields they access
3. Add edges for intra-class method calls (`self.method()`)
4. Count connected components via union-find
5. LCOM4 = number of connected components

```rust
/// Compute LCOM4 for a class.
///
/// # Algorithm
///
/// 1. Create a node for each method and each field
/// 2. Add edges: method -> field if method accesses field
/// 3. Add edges: method -> method if one calls the other
/// 4. Count connected components using union-find
fn compute_lcom4(methods: &[MethodAnalysis], fields: &[String]) -> u32 {
    let mut uf = UnionFind::new(methods.len() + fields.len());
    
    // Map method/field names to indices
    let method_idx: HashMap<&str, usize> = methods.iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();
    
    let field_idx: HashMap<&str, usize> = fields.iter()
        .enumerate()
        .map(|(i, f)| (f.as_str(), methods.len() + i))
        .collect();
    
    // Connect methods to fields they access
    for (i, method) in methods.iter().enumerate() {
        for field in &method.field_accesses {
            if let Some(&fi) = field_idx.get(field.as_str()) {
                uf.union(i, fi);
            }
        }
    }
    
    // Connect methods that call each other
    for (i, method) in methods.iter().enumerate() {
        for called in &method.method_calls {
            if let Some(&ci) = method_idx.get(called.as_str()) {
                uf.union(i, ci);
            }
        }
    }
    
    // Count unique roots (connected components)
    let mut roots = HashSet::new();
    for i in 0..methods.len() {
        roots.insert(uf.find(i));
    }
    
    roots.len() as u32
}
```

#### Output Schema (JSON)

```json
{
  "classes": [
    {
      "class_name": "UserManager",
      "file_path": "src/user.py",
      "line": 10,
      "lcom4": 2,
      "method_count": 6,
      "field_count": 4,
      "verdict": "split_candidate",
      "split_suggestion": "Consider splitting into: UserAuth (methods: login, logout, authenticate; fields: password_hash, session) and UserProfile (methods: get_name, update_email; fields: name, email)",
      "components": [
        {"methods": ["login", "logout", "authenticate"], "fields": ["password_hash", "session"]},
        {"methods": ["get_name", "update_email"], "fields": ["name", "email"]}
      ]
    }
  ],
  "summary": {
    "total_classes": 5,
    "cohesive": 3,
    "split_candidates": 2,
    "avg_lcom4": 1.4
  }
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File/directory not found or parse error |

---

### 3. coupling

**Purpose:** Analyze coupling between two modules by tracking cross-module function calls.

#### CLI Interface

```
tldr coupling <path_a> <path_b> [OPTIONS]

Arguments:
  <path_a>  First module to analyze (required)
  <path_b>  Second module to analyze (required)

Options:
  -f, --format <FORMAT>  Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct CouplingArgs {
    /// First module to analyze
    pub path_a: PathBuf,

    /// Second module to analyze
    pub path_b: PathBuf,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Algorithm

1. Parse both modules, extract imports and defined names
2. Find function call sites in each module
3. Match calls to imported names from the other module
4. Compute coupling score as `cross_calls / (total_functions * 2)`
5. Determine verdict based on score threshold

```rust
/// Compute coupling score between two modules.
fn compute_coupling_score(a_to_b: u32, b_to_a: u32, funcs_a: u32, funcs_b: u32) -> f64 {
    let total_funcs = funcs_a + funcs_b;
    if total_funcs == 0 {
        return 0.0;
    }
    let cross_calls = a_to_b + b_to_a;
    (cross_calls as f64) / (total_funcs as f64 * 2.0)
}
```

#### Output Schema (JSON)

```json
{
  "path_a": "src/user.py",
  "path_b": "src/auth.py",
  "a_to_b": {
    "calls": [
      {"caller": "get_user", "callee": "validate_token", "line": 15},
      {"caller": "create_user", "callee": "hash_password", "line": 32}
    ],
    "count": 2
  },
  "b_to_a": {
    "calls": [
      {"caller": "login", "callee": "get_user", "line": 10}
    ],
    "count": 1
  },
  "total_calls": 3,
  "coupling_score": 0.25,
  "verdict": "moderate"
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File not found or parse error |

---

### 4. mutability

**Purpose:** Analyze mutability of variables, parameters, collections, and class fields.

#### CLI Interface

```
tldr mutability <file> [function] [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  [function]  Function to analyze (optional)

Options:
  --lang <LANG>           Language override (python only) [default: auto]
  --include-fields        Include class field mutability analysis (M3)
  --include-aliases       Include alias propagation analysis (M5)
  --no-collections        Skip collection mutation detection (M4)
  --constraints           Generate type constraints (M8)
  --summary               Output summary statistics only
  -f, --format <FORMAT>   Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct MutabilityArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function to analyze (optional)
    pub function: Option<String>,

    /// Language override
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Include class field mutability analysis
    #[arg(long)]
    pub include_fields: bool,

    /// Include alias propagation analysis
    #[arg(long)]
    pub include_aliases: bool,

    /// Skip collection mutation detection
    #[arg(long)]
    pub no_collections: bool,

    /// Generate type constraints
    #[arg(long)]
    pub constraints: bool,

    /// Output summary only
    #[arg(long)]
    pub summary: bool,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Analysis Types

| ID | Analysis | Description |
|----|----------|-------------|
| M1 | Variable mutability | Which variables are reassigned |
| M2 | Parameter mutations | Which parameters are modified |
| M3 | Field mutability | Class fields modified after `__init__` |
| M4 | Collection mutations | `list.append`, `dict.update`, etc. |
| M5 | Alias propagation | Mutation through aliases |
| M8 | Type constraints | Generate immutable type hints |

#### Output Schema (JSON)

```json
{
  "file": "src/utils.py",
  "language": "python",
  "functions": [
    {
      "name": "process",
      "variables": [
        {"name": "count", "mutable": true, "reassignments": 3, "mutations": 0},
        {"name": "result", "mutable": false, "reassignments": 0, "mutations": 0}
      ],
      "parameters": [
        {"name": "data", "mutated": true, "mutation_sites": [15, 22]},
        {"name": "config", "mutated": false, "mutation_sites": []}
      ],
      "collection_mutations": [
        {"variable": "results", "operation": "append", "line": 18}
      ]
    }
  ],
  "classes": [
    {
      "name": "Counter",
      "fields": [
        {"name": "count", "mutable": true, "init_only": false},
        {"name": "name", "mutable": false, "init_only": true}
      ]
    }
  ],
  "summary": {
    "functions_analyzed": 5,
    "classes_analyzed": 2,
    "total_variables": 15,
    "mutable_variables": 6,
    "immutable_variables": 9,
    "immutable_pct": 60.0,
    "parameters_analyzed": 10,
    "mutated_parameters": 2,
    "unmutated_pct": 80.0,
    "fields_analyzed": 8,
    "mutable_fields": 3,
    "constraints_generated": 0
  },
  "analysis_time_ms": 45
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File/function not found or parse error |
| 2 | Invalid arguments |

---

### 5. purity

**Purpose:** Analyze function purity (side-effect free) across a file or directory.

#### CLI Interface

```
tldr purity <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  --no-interprocedural    Disable interprocedural propagation
  --include-tests         Include test files in directory analysis
  -f, --format <FORMAT>   Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct PurityArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Disable interprocedural propagation
    #[arg(long)]
    pub no_interprocedural: bool,

    /// Include test files in directory analysis
    #[arg(long)]
    pub include_tests: bool,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Algorithm

1. For each function, detect direct effects:
   - I/O operations (open, print, read, write)
   - Global variable writes
   - Attribute writes (self.x = ...)
   - Collection mutations
2. If interprocedural enabled:
   - Build call graph
   - Propagate impurity from callees to callers
3. Assign confidence based on analysis completeness

#### Output Schema (JSON)

```json
{
  "files": [
    {
      "source_file": "src/utils.py",
      "functions": [
        {"name": "add", "pure": true, "effects": [], "confidence": "high"},
        {"name": "log", "pure": false, "effects": ["io"], "confidence": "high"},
        {"name": "update_cache", "pure": false, "effects": ["global_write"], "confidence": "medium"}
      ],
      "pure_count": 1
    }
  ],
  "total_functions": 3,
  "total_pure": 1
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File/directory not found or parse error |

---

### 6. temporal

**Purpose:** Mine temporal constraints (method call sequences) from a codebase.

#### CLI Interface

```
tldr temporal <path> [OPTIONS]

Arguments:
  <path>  Directory or file to analyze (required)

Options:
  --min-support <N>        Minimum pattern occurrences [default: 2]
  --min-confidence <F>     Minimum confidence (0.0-1.0) [default: 0.5]
  --query <METHOD>         Filter for specific method
  --lang <LANG>            Language [default: auto]
  --max-files <N>          Maximum files to analyze [default: 1000]
  --include-trigrams       Mine 3-method sequences
  --include-examples <N>   Number of examples per constraint [default: 3]
  -f, --format <FORMAT>    Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct TemporalArgs {
    /// Directory or file to analyze
    pub path: PathBuf,

    /// Minimum occurrences for a pattern
    #[arg(long, default_value = "2")]
    pub min_support: u32,

    /// Minimum confidence threshold (0.0-1.0)
    #[arg(long, default_value = "0.5")]
    pub min_confidence: f64,

    /// Filter for specific method
    #[arg(long)]
    pub query: Option<String>,

    /// Language
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Maximum files to analyze
    #[arg(long, default_value = "1000")]
    pub max_files: u32,

    /// Mine 3-method sequences
    #[arg(long)]
    pub include_trigrams: bool,

    /// Number of examples per constraint
    #[arg(long, default_value = "3")]
    pub include_examples: u32,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Algorithm

1. Extract method call sequences from each function
2. Build frequency table of (before, after) pairs
3. Calculate confidence: `count(A->B) / count(A)`
4. Filter by min_support and min_confidence
5. Optionally mine trigrams (3-method sequences)

#### Output Schema (JSON)

```json
{
  "constraints": [
    {
      "before": "open",
      "after": "close",
      "support": 15,
      "confidence": 0.93,
      "examples": [
        {"file": "src/io.py", "line": 42},
        {"file": "src/db.py", "line": 18}
      ]
    },
    {
      "before": "acquire",
      "after": "release",
      "support": 8,
      "confidence": 0.89,
      "examples": [...]
    }
  ],
  "trigrams": [
    {
      "sequence": ["open", "read", "close"],
      "support": 12,
      "confidence": 0.85
    }
  ],
  "metadata": {
    "files_analyzed": 45,
    "sequences_extracted": 230,
    "min_support": 2,
    "min_confidence": 0.5
  }
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Error (file not found, parse error) |
| 2 | No constraints found (not an error) |

---

### 7. interface

**Purpose:** Extract the public interface (API surface) of Python files.

#### CLI Interface

```
tldr interface <path> [OPTIONS]

Arguments:
  <path>  File or directory to analyze (required)

Options:
  --lang <LANG>          Language [default: python]
  -f, --format <FORMAT>  Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct InterfaceArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Language
    #[arg(long, default_value = "python")]
    pub lang: String,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Algorithm

1. Parse source file
2. Extract `__all__` if defined
3. For each top-level definition:
   - If name starts with `_`, skip (private)
   - If function: extract name, signature, docstring, async status
   - If class: extract name, bases, public methods
4. For directory: aggregate across all `.py` files

#### Output Schema (JSON)

```json
{
  "file": "src/api.py",
  "all_exports": ["Client", "connect", "Config"],
  "functions": [
    {
      "name": "connect",
      "signature": "def connect(host: str, port: int = 8080) -> Connection",
      "docstring": "Connect to the server.",
      "lineno": 15,
      "is_async": false
    }
  ],
  "classes": [
    {
      "name": "Client",
      "lineno": 30,
      "bases": ["BaseClient"],
      "methods": [
        {"name": "send", "signature": "def send(self, data: bytes) -> None", "is_async": true},
        {"name": "receive", "signature": "def receive(self) -> bytes", "is_async": true}
      ],
      "private_method_count": 3
    }
  ]
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | File/directory not found or parse error |

---

### 8. resources

**Purpose:** Analyze resource lifecycle to detect leaks, double-close, and use-after-close.

#### CLI Interface

```
tldr resources <file> [function] [OPTIONS]

Arguments:
  <file>      Source file to analyze (required)
  [function]  Function to analyze (optional)

Options:
  --lang <LANG>           Language override [default: auto]
  --check-leaks           Run leak detection (R2) [default: true]
  --check-double-close    Run double-close detection (R3)
  --check-use-after-close Run use-after-close detection (R4)
  --check-all             Run all checks (R2, R3, R4)
  --suggest-context       Suggest context manager usage (R6)
  --show-paths            Show detailed leak paths (R7)
  --constraints           Generate LLM constraints (R9)
  --summary               Output summary only
  -f, --format <FORMAT>   Output format [default: json]
```

#### Args Struct

```rust
#[derive(Debug, Args)]
pub struct ResourcesArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function to analyze (optional)
    pub function: Option<String>,

    /// Language override
    #[arg(long, default_value = "auto")]
    pub lang: String,

    /// Run leak detection (enabled by default)
    #[arg(long, default_value = "true")]
    pub check_leaks: bool,

    /// Run double-close detection
    #[arg(long)]
    pub check_double_close: bool,

    /// Run use-after-close detection
    #[arg(long)]
    pub check_use_after_close: bool,

    /// Run all checks
    #[arg(long)]
    pub check_all: bool,

    /// Suggest context manager usage
    #[arg(long)]
    pub suggest_context: bool,

    /// Show detailed leak paths
    #[arg(long)]
    pub show_paths: bool,

    /// Generate LLM constraints
    #[arg(long)]
    pub constraints: bool,

    /// Output summary only
    #[arg(long)]
    pub summary: bool,

    /// Output format
    #[arg(long, short = 'f', default_value = "json")]
    pub format: OutputFormat,
}
```

#### Analysis Types

| ID | Analysis | Description |
|----|----------|-------------|
| R1 | Resource detection | Identify resources requiring close |
| R2 | Close verification | All-paths leak detection |
| R3 | Double-close detection | Closing resources twice |
| R4 | Use-after-close | Using closed resources |
| R6 | Context manager suggestions | Suggest `with` statement |
| R7 | Leak path enumeration | Detailed paths to leaks |
| R9 | Constraint generation | LLM-ready constraints |

#### Algorithm (R2 - Leak Detection)

1. Build CFG for function
2. Identify resource creation points (open, socket, connect)
3. For each resource:
   a. Find all paths from creation to function exit
   b. Check if all paths include close
   c. If not, report as potential leak

```rust
/// Detect potential resource leaks using CFG path analysis.
fn detect_leaks(cfg: &Cfg, resources: &[Resource]) -> Vec<LeakInfo> {
    let mut leaks = Vec::new();
    
    for resource in resources {
        let creation_block = cfg.block_containing(resource.creation_line);
        let exit_blocks = cfg.exit_blocks();
        
        // Check all paths from creation to each exit
        for exit in exit_blocks {
            let paths = cfg.all_paths(creation_block, exit);
            for path in paths {
                if !path_has_close(&path, &resource.name) {
                    leaks.push(LeakInfo {
                        resource: resource.name.clone(),
                        line: resource.creation_line,
                        paths: Some(vec![format_path(&path)]),
                    });
                    break; // One leak path is enough
                }
            }
        }
    }
    
    leaks
}
```

#### Output Schema (JSON)

```json
{
  "file": "src/db.py",
  "language": "python",
  "function": "query",
  "resources": [
    {"name": "conn", "type": "connection", "line": 10, "closed": true},
    {"name": "file", "type": "file", "line": 15, "closed": false}
  ],
  "leaks": [
    {
      "resource": "file",
      "line": 15,
      "paths": ["15 -> 18 -> 22 (exception) -> exit"]
    }
  ],
  "double_closes": [
    {"resource": "conn", "first_close": 25, "second_close": 30}
  ],
  "use_after_closes": [
    {"resource": "file", "close_line": 20, "use_line": 22}
  ],
  "suggestions": [
    {"resource": "file", "suggestion": "with open(...) as file:"}
  ],
  "constraints": [
    {"rule": "file must be closed on all paths", "context": "query function", "confidence": 0.95}
  ],
  "summary": {
    "resources_detected": 2,
    "leaks_found": 1,
    "double_closes_found": 1,
    "use_after_closes_found": 1
  },
  "analysis_time_ms": 120
}
```

#### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success, no issues |
| 1 | File/function not found or parse error |
| 2 | Invalid arguments |
| 3 | Issues found (leaks, double-close, etc.) |

---

## Testing Requirements

### Unit Tests

Each command should have tests for:

1. **Happy path**: Valid input produces expected output
2. **Edge cases**: Empty files, single function, no matches
3. **Error handling**: File not found, parse errors, invalid args
4. **Resource limits**: Large files, deep nesting

### Integration Tests

1. **End-to-end CLI tests**: Run command and verify JSON output
2. **Cross-command consistency**: Ensure types are consistent
3. **Performance tests**: Verify reasonable performance on large files

### Test Coverage Target

- Minimum 80% line coverage
- 100% coverage on error paths
- All TIGER mitigations tested

---

## Security Mitigations

### TIGER (Critical)

| ID | Risk | Mitigation |
|----|------|------------|
| T2 | Path traversal | `validate_file_path`, `validate_file_path_in_project` |
| T3 | Unbounded loops | `MAX_ANALYSIS_DEPTH`, `check_analysis_depth` |
| T4 | Memory exhaustion | `MAX_FILE_SIZE`, `MAX_DIRECTORY_FILES` |
| T8 | Stack overflow | `MAX_AST_DEPTH` |

### ELEPHANT (Important)

| ID | Risk | Mitigation |
|----|------|------------|
| E1 | Timeouts | Add timeout parameter for long analyses |
| E2 | Resource limits | `check_file_count`, max files in directory |
| E3 | Partial failure | Graceful handling of parse errors |
| E4 | Unicode handling | UTF-8 validation in `read_file_safe` |

### PAPER_TIGER (Low)

| ID | Risk | Mitigation |
|----|------|------------|
| P1 | Error messages | Don't leak internal paths |
| P2 | UX | Clear progress for directory scans |
| P3 | Documentation | Explain confidence levels |

---

## Implementation Phases

### Phase 1: Foundation
**Files to create:** `mod.rs`, `types.rs`, `error.rs`, `validation.rs`

**Acceptance:**
- [ ] All types compile
- [ ] Error enum complete
- [ ] Validation functions tested

**Estimated effort:** Small

### Phase 2: Simple Commands
**Files to create:** `cohesion.rs`, `coupling.rs`, `interface.rs`, `purity.rs`

**Dependencies:** Phase 1

**Acceptance:**
- [ ] `tldr cohesion` computes LCOM4
- [ ] `tldr coupling` analyzes module pairs
- [ ] `tldr interface` extracts API
- [ ] `tldr purity` detects effects

**Estimated effort:** Medium

### Phase 3: Medium Commands
**Files to create:** `temporal.rs`, `behavioral.rs`, `mutability.rs`

**Dependencies:** Phase 2

**Acceptance:**
- [ ] `tldr temporal` mines constraints
- [ ] `tldr behavioral` extracts pre/postconditions
- [ ] `tldr mutability` tracks mutations

**Estimated effort:** Medium

### Phase 4: Complex Command
**Files to create:** `resources.rs`

**Dependencies:** Phase 3

**Acceptance:**
- [ ] `tldr resources` detects leaks
- [ ] Double-close detection works
- [ ] Use-after-close detection works

**Estimated effort:** Large

### Phase 5: Testing & Documentation

**Coverage target:** 80%

---

## Success Criteria

1. All 8 commands pass `--help` and produce valid JSON
2. `tldr cohesion` correctly computes LCOM4 on test classes
3. `tldr coupling` correctly identifies cross-module calls
4. `tldr purity` correctly identifies pure functions
5. `tldr temporal` discovers `open->close` patterns
6. `tldr behavioral` extracts guard clause preconditions
7. `tldr mutability` tracks parameter mutations
8. `tldr resources` detects resource leaks in test cases
9. Error messages are actionable and specific
10. All TIGER mitigations pass security review
