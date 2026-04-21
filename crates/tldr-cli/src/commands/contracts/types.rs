//! Shared types for Contracts & Flow commands
//!
//! This module defines all data types used across the contracts and flow analysis
//! commands. Types are designed for JSON serialization with serde.
//!
//! # Schema Version
//!
//! All report types include implicit schema versioning through the module's
//! SCHEMA_VERSION constant. Consumers should check schema compatibility.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

// =============================================================================
// Confidence Level
// =============================================================================

/// Confidence level for inferred contracts and invariants.
///
/// Confidence is determined by the source of the inference:
/// - **High**: Direct code evidence (guard clause, assertion, explicit raise)
/// - **Medium**: Inferred from patterns or consistent behavior
/// - **Low**: Derived from type hints or annotations only
///
/// # Serialization
///
/// Serializes to snake_case: `"high"`, `"medium"`, `"low"`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Direct code evidence (guard clause, assertion)
    High,
    /// Inferred from patterns or types
    #[default]
    Medium,
    /// Derived from type hints only
    Low,
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

// =============================================================================
// Condition (Contract Element)
// =============================================================================

/// A single contract condition (precondition, postcondition, or invariant).
///
/// Represents a constraint on a variable that was detected in the source code.
///
/// # Example
///
/// ```json
/// {
///   "variable": "x",
///   "constraint": "x >= 0",
///   "source_line": 10,
///   "confidence": "high"
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    /// Variable name this condition applies to
    pub variable: String,

    /// Human-readable constraint expression (e.g., "x > 0", "isinstance(x, str)")
    pub constraint: String,

    /// Source line where condition was detected (1-indexed)
    pub source_line: u32,

    /// Confidence level of this condition
    pub confidence: Confidence,
}

impl Condition {
    /// Create a new condition with High confidence (from guard clause/assert)
    pub fn high(variable: impl Into<String>, constraint: impl Into<String>, line: u32) -> Self {
        Self {
            variable: variable.into(),
            constraint: constraint.into(),
            source_line: line,
            confidence: Confidence::High,
        }
    }

    /// Create a new condition with Medium confidence (from patterns)
    pub fn medium(variable: impl Into<String>, constraint: impl Into<String>, line: u32) -> Self {
        Self {
            variable: variable.into(),
            constraint: constraint.into(),
            source_line: line,
            confidence: Confidence::Medium,
        }
    }

    /// Create a new condition with Low confidence (from type hints)
    pub fn low(variable: impl Into<String>, constraint: impl Into<String>, line: u32) -> Self {
        Self {
            variable: variable.into(),
            constraint: constraint.into(),
            source_line: line,
            confidence: Confidence::Low,
        }
    }
}

// =============================================================================
// Invariant Types
// =============================================================================

/// Types of invariants that can be inferred from test traces.
///
/// Based on Daikon-style invariant templates.
///
/// # Serialization
///
/// Serializes to snake_case: `"type"`, `"non_null"`, `"non_negative"`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvariantKind {
    /// Type invariant (e.g., "x: int")
    Type,
    /// Non-null invariant (no None values observed)
    NonNull,
    /// Non-negative numeric (all values >= 0)
    NonNegative,
    /// Positive numeric (all values > 0)
    Positive,
    /// Range constraint (min <= x <= max)
    Range,
    /// Ordering relation between parameters (e.g., start < end)
    Relation,
    /// Length constraint (e.g., len(x) > 0)
    Length,
}

impl std::fmt::Display for InvariantKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Type => write!(f, "type"),
            Self::NonNull => write!(f, "non_null"),
            Self::NonNegative => write!(f, "non_negative"),
            Self::Positive => write!(f, "positive"),
            Self::Range => write!(f, "range"),
            Self::Relation => write!(f, "relation"),
            Self::Length => write!(f, "length"),
        }
    }
}

/// An inferred invariant from test execution traces.
///
/// Invariants are derived from observing function behavior across multiple
/// test executions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invariant {
    /// Variable name this invariant applies to
    pub variable: String,

    /// Kind of invariant
    pub kind: InvariantKind,

    /// Human-readable expression (e.g., "x >= 0", "isinstance(x, int)")
    pub expression: String,

    /// Confidence level based on observation count
    pub confidence: Confidence,

    /// Number of observations supporting this invariant
    pub observations: u32,

    /// Number of counterexamples observed (should be 0 for valid invariants)
    pub counterexample_count: u32,
}

// =============================================================================
// Spec Types (from test extraction)
// =============================================================================

/// Input/Output specification from a test assertion.
///
/// Extracted from patterns like `assert func(args) == expected`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputOutputSpec {
    /// Function being tested
    pub function: String,

    /// Input arguments (as JSON values)
    pub inputs: Vec<serde_json::Value>,

    /// Expected output (as JSON value)
    pub output: serde_json::Value,

    /// Name of the test function where this was found
    pub test_function: String,

    /// Line number in the test file
    pub line: u32,

    /// Confidence level
    pub confidence: Confidence,
}

/// Exception specification from pytest.raises.
///
/// Extracted from patterns like `with pytest.raises(ValueError)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExceptionSpec {
    /// Function being tested
    pub function: String,

    /// Input arguments that trigger the exception
    pub inputs: Vec<serde_json::Value>,

    /// Exception type expected (e.g., "ValueError")
    pub exception_type: String,

    /// Optional match pattern for exception message
    pub match_pattern: Option<String>,

    /// Name of the test function where this was found
    pub test_function: String,

    /// Line number in the test file
    pub line: u32,

    /// Confidence level
    pub confidence: Confidence,
}

/// Property specification from test assertions.
///
/// Extracted from patterns like `assert isinstance(f(x), T)` or `assert len(f(x)) == n`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropertySpec {
    /// Function being tested
    pub function: String,

    /// Type of property: "type", "length", "bounds", "boolean", "membership", "not_none"
    pub property_type: String,

    /// Human-readable constraint
    pub constraint: String,

    /// Name of the test function where this was found
    pub test_function: String,

    /// Line number in the test file
    pub line: u32,

    /// Confidence level
    pub confidence: Confidence,
}

// =============================================================================
// Interval (for bounds analysis)
// =============================================================================

/// Numeric interval [lo, hi] for bounds analysis.
///
/// Represents the range of possible values for a numeric variable.
/// Uses f64 for flexibility (handles both int and float).
///
/// # Special Values
///
/// - `top()`: [NEG_INFINITY, INFINITY] - any value possible
/// - `bottom()`: [INFINITY, NEG_INFINITY] - no valid values (unreachable)
/// - `const_val(n)`: [n, n] - exactly one value
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// Lower bound (f64::NEG_INFINITY for unbounded below)
    pub lo: f64,

    /// Upper bound (f64::INFINITY for unbounded above)
    pub hi: f64,
}

// Custom serialization to handle infinity values as strings
impl Serialize for Interval {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Interval", 2)?;

        // Serialize lo
        if self.lo == f64::NEG_INFINITY {
            state.serialize_field("lo", "-inf")?;
        } else if self.lo == f64::INFINITY {
            state.serialize_field("lo", "+inf")?;
        } else if self.lo.is_nan() {
            state.serialize_field("lo", "NaN")?;
        } else {
            state.serialize_field("lo", &self.lo)?;
        }

        // Serialize hi
        if self.hi == f64::NEG_INFINITY {
            state.serialize_field("hi", "-inf")?;
        } else if self.hi == f64::INFINITY {
            state.serialize_field("hi", "+inf")?;
        } else if self.hi.is_nan() {
            state.serialize_field("hi", "NaN")?;
        } else {
            state.serialize_field("hi", &self.hi)?;
        }

        state.end()
    }
}

// Custom deserialization to handle infinity values as strings
impl<'de> Deserialize<'de> for Interval {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct IntervalHelper {
            lo: serde_json::Value,
            hi: serde_json::Value,
        }

        let helper = IntervalHelper::deserialize(deserializer)?;

        fn parse_bound(v: serde_json::Value) -> Result<f64, String> {
            match v {
                serde_json::Value::Number(n) => {
                    n.as_f64().ok_or_else(|| "invalid number".to_string())
                }
                serde_json::Value::String(s) => match s.as_str() {
                    "-inf" | "-Infinity" => Ok(f64::NEG_INFINITY),
                    "+inf" | "inf" | "Infinity" => Ok(f64::INFINITY),
                    "NaN" => Ok(f64::NAN),
                    _ => s.parse::<f64>().map_err(|e| e.to_string()),
                },
                serde_json::Value::Null => Ok(f64::INFINITY), // null defaults to infinity
                _ => Err("expected number or string".to_string()),
            }
        }

        let lo = parse_bound(helper.lo).map_err(serde::de::Error::custom)?;
        let hi = parse_bound(helper.hi).map_err(serde::de::Error::custom)?;

        Ok(Interval { lo, hi })
    }
}

impl Interval {
    /// Create an interval containing exactly one value.
    pub fn const_val(n: f64) -> Self {
        Self { lo: n, hi: n }
    }

    /// Create the top element (any value possible).
    pub fn top() -> Self {
        Self {
            lo: f64::NEG_INFINITY,
            hi: f64::INFINITY,
        }
    }

    /// Create the bottom element (no valid values - unreachable).
    pub fn bottom() -> Self {
        Self {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY,
        }
    }

    /// Check if this interval is bottom (empty/unreachable).
    pub fn is_bottom(&self) -> bool {
        self.lo > self.hi
    }

    /// Check if this interval is top (any value possible).
    pub fn is_top(&self) -> bool {
        self.lo == f64::NEG_INFINITY && self.hi == f64::INFINITY
    }

    /// Check if this interval contains a specific value.
    pub fn contains(&self, n: f64) -> bool {
        !self.is_bottom() && self.lo <= n && n <= self.hi
    }

    /// Check if this interval contains zero (important for division-by-zero detection).
    pub fn contains_zero(&self) -> bool {
        self.contains(0.0)
    }

    /// Compute the join (least upper bound) of two intervals.
    ///
    /// `[a,b] | [c,d] = [min(a,c), max(b,d)]`
    pub fn join(&self, other: &Self) -> Self {
        if self.is_bottom() {
            return *other;
        }
        if other.is_bottom() {
            return *self;
        }
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    /// Compute the meet (greatest lower bound) of two intervals.
    ///
    /// `[a,b] & [c,d] = [max(a,c), min(b,d)]` or bottom if empty
    pub fn meet(&self, other: &Self) -> Self {
        if self.is_bottom() || other.is_bottom() {
            return Self::bottom();
        }
        // Result may be bottom if intervals don't overlap.
        Self {
            lo: self.lo.max(other.lo),
            hi: self.hi.min(other.hi),
        }
    }

    /// Widen this interval based on new observations.
    ///
    /// Used to ensure convergence in fixpoint iteration.
    /// `[a,b] W [c,d] = [c<a ? -inf : a, d>b ? +inf : b]`
    pub fn widen(&self, other: &Self) -> Self {
        if self.is_bottom() {
            return *other;
        }
        if other.is_bottom() {
            return *self;
        }
        Self {
            lo: if other.lo < self.lo {
                f64::NEG_INFINITY
            } else {
                self.lo
            },
            hi: if other.hi > self.hi {
                f64::INFINITY
            } else {
                self.hi
            },
        }
    }

    /// Add two intervals: [a,b] + [c,d] = [a+c, b+d]
    pub fn add(&self, other: &Self) -> Self {
        if self.is_bottom() || other.is_bottom() {
            return Self::bottom();
        }
        Self {
            lo: self.lo + other.lo,
            hi: self.hi + other.hi,
        }
    }

    /// Subtract two intervals: [a,b] - [c,d] = [a-d, b-c]
    pub fn sub(&self, other: &Self) -> Self {
        if self.is_bottom() || other.is_bottom() {
            return Self::bottom();
        }
        Self {
            lo: self.lo - other.hi,
            hi: self.hi - other.lo,
        }
    }

    /// Multiply two intervals.
    ///
    /// Handles sign combinations correctly.
    pub fn mul(&self, other: &Self) -> Self {
        if self.is_bottom() || other.is_bottom() {
            return Self::bottom();
        }
        // Compute all four products and take min/max
        let products = [
            self.lo * other.lo,
            self.lo * other.hi,
            self.hi * other.lo,
            self.hi * other.hi,
        ];
        Self {
            lo: products.iter().cloned().fold(f64::INFINITY, f64::min),
            hi: products.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        }
    }

    /// Divide two intervals.
    ///
    /// Returns (result, may_divide_by_zero) where may_divide_by_zero is true
    /// if the divisor interval contains zero.
    pub fn div(&self, other: &Self) -> (Self, bool) {
        if self.is_bottom() || other.is_bottom() {
            return (Self::bottom(), false);
        }

        let may_div_zero = other.contains_zero();

        // If divisor is exactly [0,0], result is bottom (undefined)
        if other.lo == 0.0 && other.hi == 0.0 {
            return (Self::bottom(), true);
        }

        // Handle cases where divisor contains zero
        if may_div_zero {
            // Conservative: return top
            return (Self::top(), true);
        }

        // Safe division: divisor doesn't contain zero
        let products = [
            self.lo / other.lo,
            self.lo / other.hi,
            self.hi / other.lo,
            self.hi / other.hi,
        ];
        (
            Self {
                lo: products.iter().cloned().fold(f64::INFINITY, f64::min),
                hi: products.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            },
            false,
        )
    }

    /// Negate an interval: -[a,b] = [-b, -a]
    pub fn neg(&self) -> Self {
        if self.is_bottom() {
            return Self::bottom();
        }
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
}

impl Default for Interval {
    fn default() -> Self {
        Self::top()
    }
}

impl std::fmt::Display for Interval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_bottom() {
            write!(f, "bottom")
        } else if self.is_top() {
            write!(f, "(-inf, +inf)")
        } else if self.lo == self.hi {
            write!(f, "[{}]", self.lo)
        } else {
            let lo_str = if self.lo == f64::NEG_INFINITY {
                "-inf".to_string()
            } else {
                self.lo.to_string()
            };
            let hi_str = if self.hi == f64::INFINITY {
                "+inf".to_string()
            } else {
                self.hi.to_string()
            };
            write!(f, "[{}, {}]", lo_str, hi_str)
        }
    }
}

// =============================================================================
// Dead Store Detection
// =============================================================================

/// A dead store detected in SSA form.
///
/// A definition is dead if it has no uses (empty use_sites).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadStore {
    /// Original variable name (without SSA version suffix)
    pub variable: String,

    /// SSA versioned name (e.g., "x_2")
    pub ssa_name: String,

    /// Line number of the dead assignment (1-indexed)
    pub line: u32,

    /// Block ID where assignment occurs
    pub block_id: u32,

    /// Whether this is a phi function definition
    pub is_phi: bool,
}

impl DeadStore {
    /// Convert to JSON value for compatibility with spec
    pub fn to_dict(&self) -> serde_json::Value {
        serde_json::json!({
            "variable": self.variable,
            "ssa_name": self.ssa_name,
            "line": self.line,
            "block_id": self.block_id,
            "is_phi": self.is_phi,
        })
    }
}

// =============================================================================
// Chop Result (Slice Intersection)
// =============================================================================

/// Result of a chop operation (slice intersection).
///
/// Chop computes `forward_slice(source) AND backward_slice(target)` to find
/// all statements on any path from source to target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChopResult {
    /// Lines on the dependency path (sorted)
    pub lines: Vec<u32>,

    /// Number of lines on the path
    pub count: u32,

    /// Source line (where data flows FROM)
    pub source_line: u32,

    /// Target line (where data flows TO)
    pub target_line: u32,

    /// True if source_line is in backward_slice(target_line)
    pub path_exists: bool,

    /// Function name containing the analysis
    pub function: String,

    /// Human-readable explanation of the result
    pub explanation: Option<String>,
}

impl ChopResult {
    /// Create a result for when source and target are the same line.
    pub fn same_line(line: u32, function: impl Into<String>) -> Self {
        Self {
            lines: vec![line],
            count: 1,
            source_line: line,
            target_line: line,
            path_exists: true,
            function: function.into(),
            explanation: Some(format!("Source and target are the same line ({}).", line)),
        }
    }

    /// Create a result for when no path exists.
    pub fn no_path(source: u32, target: u32, function: impl Into<String>) -> Self {
        Self {
            lines: vec![],
            count: 0,
            source_line: source,
            target_line: target,
            path_exists: false,
            function: function.into(),
            explanation: Some(format!(
                "No dependency path from line {} to line {}. \
                 The source line does not affect the target line.",
                source, target
            )),
        }
    }
}

// =============================================================================
// Interval Warning
// =============================================================================

/// Warning from interval/bounds analysis.
///
/// Generated when analysis detects potential runtime issues like division by zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntervalWarning {
    /// Line number where the warning applies
    pub line: u32,

    /// Kind of warning: "division_by_zero", "out_of_bounds", "overflow"
    pub kind: String,

    /// Variable involved
    pub variable: String,

    /// Current bounds for the variable
    pub bounds: Interval,

    /// Human-readable warning message
    pub message: String,
}

// =============================================================================
// Report Types
// =============================================================================

/// Report from the contracts command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractsReport {
    /// Function analyzed
    pub function: String,

    /// File path
    pub file: PathBuf,

    /// Detected preconditions
    pub preconditions: Vec<Condition>,

    /// Detected postconditions
    pub postconditions: Vec<Condition>,

    /// Detected invariants
    pub invariants: Vec<Condition>,
}

/// Invariants for a single function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionInvariants {
    /// Function name
    pub function_name: String,

    /// Inferred preconditions
    pub preconditions: Vec<Invariant>,

    /// Inferred postconditions
    pub postconditions: Vec<Invariant>,

    /// Total observations used for inference
    pub observation_count: u32,
}

/// Summary of invariant inference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvariantsSummary {
    /// Total test observations across all functions
    pub total_observations: u32,

    /// Total invariants inferred
    pub total_invariants: u32,

    /// Count by invariant kind
    pub by_kind: HashMap<String, u32>,
}

/// Full report from the invariants command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvariantsReport {
    /// Invariants by function
    pub functions: Vec<FunctionInvariants>,

    /// Summary statistics
    pub summary: InvariantsSummary,
}

/// Specs for a single function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionSpecs {
    /// Function name
    pub function_name: String,

    /// Human-readable summary (e.g., "3 input/output, 1 raises")
    pub summary: String,

    /// Number of test functions that test this function
    pub test_count: u32,

    /// Input/output specifications
    pub input_output_specs: Vec<InputOutputSpec>,

    /// Exception specifications
    pub exception_specs: Vec<ExceptionSpec>,

    /// Property specifications
    pub property_specs: Vec<PropertySpec>,
}

/// Counts by spec type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecsByType {
    /// Input/output spec count
    pub input_output: u32,

    /// Exception spec count
    pub exception: u32,

    /// Property spec count
    pub property: u32,
}

/// Summary of specs extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpecsSummary {
    /// Total specs extracted
    pub total_specs: u32,

    /// Counts by type
    pub by_type: SpecsByType,

    /// Number of test functions scanned
    pub test_functions_scanned: u32,

    /// Number of test files scanned
    pub test_files_scanned: u32,

    /// Number of unique functions found
    pub functions_found: u32,
}

/// Full report from the specs command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecsReport {
    /// Specs by function
    pub functions: Vec<FunctionSpecs>,

    /// Summary statistics
    pub summary: SpecsSummary,
}

/// Result from the bounds command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundsResult {
    /// Function analyzed
    pub function: String,

    /// Interval bounds: line -> variable -> interval
    pub bounds: HashMap<u32, HashMap<String, Interval>>,

    /// Warnings (e.g., potential division by zero)
    pub warnings: Vec<IntervalWarning>,

    /// Whether analysis converged
    pub converged: bool,

    /// Number of fixpoint iterations
    pub iterations: u32,
}

/// Report from the dead-stores command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeadStoresReport {
    /// Function analyzed
    pub function: String,

    /// File path
    pub file: PathBuf,

    /// Dead stores detected via SSA analysis
    pub dead_stores_ssa: Vec<DeadStore>,

    /// Count of dead stores
    pub count: u32,

    /// Optional: dead stores via live-vars analysis (if --compare flag used)
    pub dead_stores_live_vars: Option<Vec<DeadStore>>,

    /// Optional: count from live-vars analysis
    pub live_vars_count: Option<u32>,
}

/// Status of a sub-analysis in verify command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAnalysisStatus {
    /// Analysis completed successfully
    Success,
    /// Analysis completed with partial results (some files failed)
    Partial,
    /// Analysis failed completely
    Failed,
    /// Analysis was skipped (e.g., in quick mode)
    Skipped,
}

impl SubAnalysisStatus {
    /// Returns true if the analysis succeeded (fully or partially)
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success | Self::Partial)
    }
}

/// Result from a sub-analysis in verify command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubAnalysisResult {
    /// Name of the sub-analysis
    pub name: String,

    /// Status of the analysis
    pub status: SubAnalysisStatus,

    /// Number of items found (contracts, specs, warnings, etc.)
    pub items_found: u32,

    /// Time taken in milliseconds
    pub elapsed_ms: u64,

    /// Error message if failed or partial
    pub error: Option<String>,

    /// Analysis data (command-specific)
    pub data: Option<serde_json::Value>,
}

impl SubAnalysisResult {
    /// Returns true if the analysis succeeded (for backward compatibility)
    pub fn success(&self) -> bool {
        self.status.is_success()
    }
}

/// Coverage information from verify command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageInfo {
    /// Number of functions with at least one constraint
    pub constrained_functions: u32,

    /// Total functions analyzed
    pub total_functions: u32,

    /// Coverage percentage (0.0 - 100.0)
    pub coverage_pct: f64,
}

/// Summary from verify command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifySummary {
    /// Specs extracted from tests
    pub spec_count: u32,

    /// Invariants inferred
    pub invariant_count: u32,

    /// Contracts inferred
    pub contract_count: u32,

    /// Annotated[T] constraints found
    pub annotated_count: u32,

    /// Behavioral models extracted
    pub behavioral_count: u32,

    /// Patterns detected
    pub pattern_count: u32,

    /// High-confidence patterns
    pub pattern_high_confidence: u32,

    /// Function coverage information
    pub coverage: CoverageInfo,
}

/// Full report from verify command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Path analyzed
    pub path: PathBuf,

    /// Results from each sub-analysis
    pub sub_results: HashMap<String, SubAnalysisResult>,

    /// Summary statistics
    pub summary: VerifySummary,

    /// Total time taken in milliseconds
    pub total_elapsed_ms: u64,

    /// Number of files analyzed
    pub files_analyzed: u32,

    /// Number of files that failed to analyze
    pub files_failed: u32,

    /// Whether some results are partial (some files or analyses failed)
    pub partial_results: bool,
}

// =============================================================================
// Output Format
// =============================================================================

/// Output format for command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OutputFormat {
    /// JSON output (default)
    #[default]
    Json,

    /// Human-readable text output
    Text,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Text => write!(f, "text"),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Confidence Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_confidence_enum_serialization() {
        assert_eq!(
            serde_json::to_string(&Confidence::High).unwrap(),
            "\"high\""
        );
        assert_eq!(
            serde_json::to_string(&Confidence::Medium).unwrap(),
            "\"medium\""
        );
        assert_eq!(serde_json::to_string(&Confidence::Low).unwrap(), "\"low\"");
    }

    #[test]
    fn test_confidence_enum_deserialization() {
        assert_eq!(
            serde_json::from_str::<Confidence>("\"high\"").unwrap(),
            Confidence::High
        );
        assert_eq!(
            serde_json::from_str::<Confidence>("\"medium\"").unwrap(),
            Confidence::Medium
        );
        assert_eq!(
            serde_json::from_str::<Confidence>("\"low\"").unwrap(),
            Confidence::Low
        );
    }

    #[test]
    fn test_confidence_default() {
        assert_eq!(Confidence::default(), Confidence::Medium);
    }

    // -------------------------------------------------------------------------
    // Condition Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_condition_struct_fields() {
        let cond = Condition::high("x", "x >= 0", 10);
        assert_eq!(cond.variable, "x");
        assert_eq!(cond.constraint, "x >= 0");
        assert_eq!(cond.source_line, 10);
        assert_eq!(cond.confidence, Confidence::High);
    }

    #[test]
    fn test_condition_serialization() {
        let cond = Condition::high("x", "x >= 0", 10);
        let json = serde_json::to_string(&cond).unwrap();
        assert!(json.contains("\"variable\":\"x\""));
        assert!(json.contains("\"confidence\":\"high\""));
    }

    // -------------------------------------------------------------------------
    // Interval Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_interval_const_val() {
        let i = Interval::const_val(5.0);
        assert_eq!(i.lo, 5.0);
        assert_eq!(i.hi, 5.0);
        assert!(i.contains(5.0));
        assert!(!i.contains(4.0));
        assert!(!i.contains(6.0));
    }

    #[test]
    fn test_interval_basic_operations() {
        let i = Interval { lo: 0.0, hi: 10.0 };
        assert!(i.contains(0.0));
        assert!(i.contains(5.0));
        assert!(i.contains(10.0));
        assert!(!i.contains(-1.0));
        assert!(!i.contains(11.0));
    }

    #[test]
    fn test_interval_bottom_top_detection() {
        assert!(Interval::bottom().is_bottom());
        assert!(!Interval::top().is_bottom());
        assert!(Interval::top().is_top());
        assert!(!Interval::bottom().is_top());
        assert!(!Interval::const_val(5.0).is_bottom());
        assert!(!Interval::const_val(5.0).is_top());
    }

    #[test]
    fn test_interval_contains_zero() {
        assert!(Interval { lo: -5.0, hi: 5.0 }.contains_zero());
        assert!(Interval { lo: 0.0, hi: 10.0 }.contains_zero());
        assert!(Interval { lo: -10.0, hi: 0.0 }.contains_zero());
        assert!(!Interval { lo: 1.0, hi: 10.0 }.contains_zero());
        assert!(!Interval {
            lo: -10.0,
            hi: -1.0
        }
        .contains_zero());
    }

    #[test]
    fn test_interval_join() {
        let a = Interval { lo: 0.0, hi: 5.0 };
        let b = Interval { lo: 3.0, hi: 10.0 };
        let joined = a.join(&b);
        assert_eq!(joined.lo, 0.0);
        assert_eq!(joined.hi, 10.0);
    }

    #[test]
    fn test_interval_meet() {
        let a = Interval { lo: 0.0, hi: 5.0 };
        let b = Interval { lo: 3.0, hi: 10.0 };
        let met = a.meet(&b);
        assert_eq!(met.lo, 3.0);
        assert_eq!(met.hi, 5.0);
    }

    #[test]
    fn test_interval_add() {
        let a = Interval { lo: 1.0, hi: 5.0 };
        let b = Interval { lo: 2.0, hi: 3.0 };
        let sum = a.add(&b);
        assert_eq!(sum.lo, 3.0);
        assert_eq!(sum.hi, 8.0);
    }

    #[test]
    fn test_interval_sub() {
        let a = Interval { lo: 5.0, hi: 10.0 };
        let b = Interval { lo: 1.0, hi: 3.0 };
        let diff = a.sub(&b);
        assert_eq!(diff.lo, 2.0); // 5 - 3
        assert_eq!(diff.hi, 9.0); // 10 - 1
    }

    #[test]
    fn test_interval_mul() {
        let a = Interval { lo: 2.0, hi: 3.0 };
        let b = Interval { lo: 4.0, hi: 5.0 };
        let prod = a.mul(&b);
        assert_eq!(prod.lo, 8.0);
        assert_eq!(prod.hi, 15.0);
    }

    #[test]
    fn test_interval_mul_negative() {
        let a = Interval { lo: -2.0, hi: 3.0 };
        let b = Interval { lo: -1.0, hi: 2.0 };
        let prod = a.mul(&b);
        // Products: 2, -4, -3, 6 -> min=-4, max=6
        assert_eq!(prod.lo, -4.0);
        assert_eq!(prod.hi, 6.0);
    }

    #[test]
    fn test_interval_div() {
        let a = Interval { lo: 10.0, hi: 20.0 };
        let b = Interval { lo: 2.0, hi: 5.0 };
        let (quot, div_zero) = a.div(&b);
        assert!(!div_zero);
        assert_eq!(quot.lo, 2.0); // 10 / 5
        assert_eq!(quot.hi, 10.0); // 20 / 2
    }

    #[test]
    fn test_interval_div_by_zero() {
        let a = Interval { lo: 10.0, hi: 20.0 };
        let b = Interval { lo: -1.0, hi: 1.0 }; // Contains zero
        let (_, div_zero) = a.div(&b);
        assert!(div_zero);
    }

    #[test]
    fn test_interval_widen() {
        let a = Interval { lo: 0.0, hi: 10.0 };
        let b = Interval { lo: -5.0, hi: 15.0 };
        let widened = a.widen(&b);
        assert_eq!(widened.lo, f64::NEG_INFINITY);
        assert_eq!(widened.hi, f64::INFINITY);
    }

    // -------------------------------------------------------------------------
    // Dead Store Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_dead_store_struct() {
        let ds = DeadStore {
            variable: "x".to_string(),
            ssa_name: "x_2".to_string(),
            line: 10,
            block_id: 1,
            is_phi: false,
        };
        assert_eq!(ds.variable, "x");
        assert_eq!(ds.ssa_name, "x_2");
        assert_eq!(ds.line, 10);
        assert!(!ds.is_phi);
    }

    #[test]
    fn test_dead_store_serialization() {
        let ds = DeadStore {
            variable: "x".to_string(),
            ssa_name: "x_2".to_string(),
            line: 10,
            block_id: 1,
            is_phi: false,
        };
        let json = serde_json::to_string(&ds).unwrap();
        assert!(json.contains("\"variable\":\"x\""));
        assert!(json.contains("\"ssa_name\":\"x_2\""));
    }

    // -------------------------------------------------------------------------
    // Chop Result Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_chop_result_struct() {
        let result = ChopResult {
            lines: vec![2, 3, 4],
            count: 3,
            source_line: 2,
            target_line: 4,
            path_exists: true,
            function: "example".to_string(),
            explanation: Some("Found path".to_string()),
        };
        assert_eq!(result.count, 3);
        assert!(result.path_exists);
    }

    #[test]
    fn test_chop_result_same_line() {
        let result = ChopResult::same_line(5, "test_func");
        assert_eq!(result.lines, vec![5]);
        assert_eq!(result.count, 1);
        assert!(result.path_exists);
    }

    #[test]
    fn test_chop_result_no_path() {
        let result = ChopResult::no_path(2, 10, "test_func");
        assert!(result.lines.is_empty());
        assert_eq!(result.count, 0);
        assert!(!result.path_exists);
    }

    // -------------------------------------------------------------------------
    // Contracts Report Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_contracts_report_struct() {
        let report = ContractsReport {
            function: "process_data".to_string(),
            file: PathBuf::from("test.py"),
            preconditions: vec![Condition::high("x", "x >= 0", 10)],
            postconditions: vec![],
            invariants: vec![],
        };
        assert_eq!(report.function, "process_data");
        assert_eq!(report.preconditions.len(), 1);
    }

    // -------------------------------------------------------------------------
    // InvariantKind Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_invariant_kind_serialization() {
        assert_eq!(
            serde_json::to_string(&InvariantKind::Type).unwrap(),
            "\"type\""
        );
        assert_eq!(
            serde_json::to_string(&InvariantKind::NonNull).unwrap(),
            "\"non_null\""
        );
        assert_eq!(
            serde_json::to_string(&InvariantKind::NonNegative).unwrap(),
            "\"non_negative\""
        );
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_output_format_default() {
        assert_eq!(OutputFormat::default(), OutputFormat::Json);
    }

    #[test]
    fn test_output_format_display() {
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::Text.to_string(), "text");
    }
}
