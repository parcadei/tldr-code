//! Abstract Interpretation Analysis
//!
//! This module implements forward dataflow analysis with abstract interpretation
//! for tracking variable ranges, nullability, and detecting potential issues.
//!
//! ## Capabilities Implemented (Phase 5)
//!
//! - CAP-AI-01: Nullability enum (Never, Maybe, Always)
//! - CAP-AI-02: AbstractValue struct with type_, range_, nullable, constant
//! - CAP-AI-03: ConstantValue enum for tracked constants
//! - CAP-AI-04: top() and bottom() lattice elements
//! - CAP-AI-05: may_be_zero() for division-by-zero checks
//! - CAP-AI-06: may_be_null() for null dereference checks
//! - CAP-AI-07: AbstractState mapping variables to abstract values
//! - CAP-AI-21: AbstractInterpInfo result struct
//! - CAP-AI-22: to_json() serialization
//!
//! ## TIGER Mitigations
//!
//! - TIGER-PASS1-11: Use saturating arithmetic for range operations
//! - TIGER-PASS2-5: Document JSON infinity representation (null = unbounded)
//! - TIGER-PASS2-8: Track TypeScript undefined vs null as different types

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use serde_json;

use super::types::BlockId;

// =============================================================================
// Feature flags (status indicators consumed by gate2/gate3 corpus tests)
// =============================================================================

/// Whether guard narrowing is enabled in abstract interpretation.
///
/// Guard narrowing is a planned enhancement that uses control-flow guards
/// (e.g. `if x != 0 { ... }`) to refine variable ranges along the guarded
/// branch. This reduces false-positive div-zero / null-deref findings.
///
/// Currently `false` while the feature is being designed; the gate2 corpus
/// scanner reads this constant for A/B reporting between the baseline run
/// and a future enabled run.
pub const ENABLE_GUARD_NARROWING: bool = false;

/// Whether the octagon relational domain is enabled in abstract interpretation.
///
/// The octagon domain (Mine 2006) tracks relational invariants of the form
/// `±x ± y ≤ c`, which is strictly more precise than independent intervals.
/// The implementation lives in `crate::dataflow::octagon` but is not yet
/// wired into the main abstract interpreter; this flag tracks that wiring.
///
/// Currently `false` while the integration is being designed; the gate3
/// A/B corpus scanner reads this constant for reporting.
pub const ENABLE_OCTAGON_DOMAIN: bool = false;

// =============================================================================
// CAP-AI-01: Nullability Enum
// =============================================================================

/// Nullability lattice: NEVER < MAYBE < ALWAYS
///
/// Used to track whether a variable may be null/None at a program point.
///
/// # Lattice Order
///
/// ```text
///        MAYBE (top - unknown)
///       /     \
///   NEVER    ALWAYS
///       \     /
///        (bottom - contradiction, not representable)
/// ```
///
/// # Default
///
/// Defaults to `Maybe` (unknown nullability) per spec CAP-AI-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Nullability {
    /// Definitely not null - safe to dereference
    Never,
    /// Could be null or non-null - requires null check
    #[serde(rename = "maybe")]
    Maybe,
    /// Definitely null - will fail on dereference
    Always,
}

impl Default for Nullability {
    /// CAP-AI-01: Default is Maybe (unknown)
    fn default() -> Self {
        Nullability::Maybe
    }
}

impl Nullability {
    /// Convert to string representation for JSON output
    pub fn as_str(&self) -> &'static str {
        match self {
            Nullability::Never => "never",
            Nullability::Maybe => "maybe",
            Nullability::Always => "always",
        }
    }
}

// =============================================================================
// CAP-AI-03: ConstantValue Enum
// =============================================================================

/// Constant values that can be tracked during abstract interpretation.
///
/// Supports integers, floats, strings, booleans, and null values.
///
/// # JSON Representation
///
/// Values serialize directly to their JSON equivalents:
/// - Int(5) -> 5
/// - Float(3.14) -> 3.14
/// - String("hello") -> "hello"
/// - Bool(true) -> true
/// - Null -> null
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConstantValue {
    /// Integer constant (i64 range)
    Int(i64),
    /// Floating-point constant
    Float(f64),
    /// String constant
    String(String),
    /// Boolean constant
    Bool(bool),
    /// Null/None/nil constant
    Null,
}

// Manual PartialEq to handle float comparison
impl PartialEq for ConstantValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ConstantValue::Int(a), ConstantValue::Int(b)) => a == b,
            (ConstantValue::Float(a), ConstantValue::Float(b)) => {
                // Handle NaN and exact equality
                (a.is_nan() && b.is_nan()) || a == b
            }
            (ConstantValue::String(a), ConstantValue::String(b)) => a == b,
            (ConstantValue::Bool(a), ConstantValue::Bool(b)) => a == b,
            (ConstantValue::Null, ConstantValue::Null) => true,
            _ => false,
        }
    }
}

impl ConstantValue {
    /// Convert to JSON value for serialization
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            ConstantValue::Int(v) => serde_json::json!(v),
            ConstantValue::Float(v) => serde_json::json!(v),
            ConstantValue::String(v) => serde_json::json!(v),
            ConstantValue::Bool(v) => serde_json::json!(v),
            ConstantValue::Null => serde_json::Value::Null,
        }
    }
}

// =============================================================================
// CAP-AI-02 to CAP-AI-06: AbstractValue
// =============================================================================

/// Abstract representation of a variable's value at a program point.
///
/// Tracks four dimensions:
/// - `type_`: Inferred type (str, int, list, etc.) or None if unknown
/// - `range_`: Value range [min, max] for numeric types, None for unbounded
/// - `nullable`: Whether the value can be null/None
/// - `constant`: If value is a known constant, the value itself
///
/// # Range Representation
///
/// The `range_` field uses `Option<(Option<i64>, Option<i64>)>`:
/// - `None` outer: No range information (unknown)
/// - `Some((None, None))`: Unbounded range (-inf, +inf)
/// - `Some((Some(5), Some(5)))`: Exact value [5, 5]
/// - `Some((Some(1), None))`: Lower bound only [1, +inf)
/// - `Some((None, Some(10)))`: Upper bound only (-inf, 10]
///
/// # JSON Infinity Representation (TIGER-PASS2-5)
///
/// In JSON output:
/// - `null` in range array position = infinity (unbounded)
/// - Example: `"range": [null, 10]` means (-inf, 10]
///
/// # TIGER-PASS1-11: Saturating Arithmetic
///
/// All arithmetic operations on ranges use saturating operations to prevent overflow.
/// When overflow would occur, the bound is widened to infinity (None).
#[derive(Debug, Clone)]
pub struct AbstractValue {
    /// Inferred type name (e.g., "int", "str") or None if unknown
    pub type_: Option<String>,

    /// Value range [min, max] for numerics. None bounds mean infinity.
    /// For strings, tracks length.
    pub range_: Option<(Option<i64>, Option<i64>)>,

    /// Nullability status
    pub nullable: Nullability,

    /// Known constant value (used for constant propagation)
    pub constant: Option<ConstantValue>,
}

// Manual PartialEq to handle constant comparison properly
impl PartialEq for AbstractValue {
    fn eq(&self, other: &Self) -> bool {
        self.type_ == other.type_
            && self.range_ == other.range_
            && self.nullable == other.nullable
            && self.constant == other.constant
    }
}

// Note: Eq is implemented even though ConstantValue contains f64
// because we handle NaN comparison in ConstantValue::eq
impl Eq for AbstractValue {}

impl Hash for AbstractValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // CAP-AI-02: AbstractValue must be hashable for use in sets
        self.type_.hash(state);
        self.range_.hash(state);
        self.nullable.hash(state);
        // Note: constant is NOT hashed per spec - equality by structure only
    }
}

impl AbstractValue {
    /// CAP-AI-04: Top of lattice - no information known (most permissive)
    ///
    /// Returns an abstract value representing complete uncertainty:
    /// - Unknown type
    /// - Unknown range
    /// - Maybe nullable
    /// - No constant value
    ///
    /// This is the default for variables with no information.
    pub fn top() -> Self {
        AbstractValue {
            type_: None,
            range_: None,
            nullable: Nullability::Maybe,
            constant: None,
        }
    }

    /// CAP-AI-04: Bottom of lattice - contradiction (unreachable code)
    ///
    /// Returns an abstract value representing impossibility.
    /// Used for unreachable code paths.
    ///
    /// Represented as:
    /// - Type = "<bottom>"
    /// - Range = (None, None) - representing contradiction
    /// - Nullable = Never (contradicts Always)
    /// - No constant
    pub fn bottom() -> Self {
        AbstractValue {
            type_: Some("<bottom>".to_string()),
            range_: Some((None, None)),
            nullable: Nullability::Never,
            constant: None,
        }
    }

    /// CAP-AI-03: Create from known constant value
    ///
    /// Creates an abstract value with precise information from a constant:
    ///
    /// | Constant Type | type_ | range_ | nullable | constant |
    /// |---------------|-------|--------|----------|----------|
    /// | Int(v) | "int" | [v, v] | Never | Some(Int(v)) |
    /// | Float(v) | "float" | None | Never | Some(Float(v)) |
    /// | String(s) | "str" | [len, len] | Never | Some(String(s)) |
    /// | Bool(v) | "bool" | [v as i64, v as i64] | Never | Some(Bool(v)) |
    /// | Null | "NoneType" | None | Always | None |
    ///
    /// # TIGER-PASS2-8: TypeScript undefined vs null
    ///
    /// TypeScript `undefined` is tracked separately from `null`:
    /// - `null` -> type_ = "null"
    /// - `undefined` -> type_ = "undefined"
    ///
    /// Both have nullable = Always.
    pub fn from_constant(value: ConstantValue) -> Self {
        match value {
            ConstantValue::Int(v) => AbstractValue {
                type_: Some("int".to_string()),
                range_: Some((Some(v), Some(v))),
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Int(v)),
            },
            ConstantValue::Float(v) => AbstractValue {
                type_: Some("float".to_string()),
                range_: None, // Float ranges less useful
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Float(v)),
            },
            ConstantValue::String(ref s) => {
                let len = s.len() as i64;
                AbstractValue {
                    type_: Some("str".to_string()),
                    // CAP-AI-18: Track string length in range
                    range_: Some((Some(len), Some(len))),
                    nullable: Nullability::Never,
                    constant: Some(value),
                }
            }
            ConstantValue::Bool(v) => AbstractValue {
                type_: Some("bool".to_string()),
                range_: Some((Some(v as i64), Some(v as i64))),
                nullable: Nullability::Never,
                constant: Some(ConstantValue::Bool(v)),
            },
            ConstantValue::Null => AbstractValue {
                type_: Some("NoneType".to_string()),
                range_: None,
                nullable: Nullability::Always,
                constant: None, // Null constant is represented by nullable=Always
            },
        }
    }

    /// CAP-AI-05: Check if value could be zero (for division check)
    ///
    /// Returns true if the range includes zero, indicating potential
    /// division-by-zero if used as a divisor.
    ///
    /// # Logic
    ///
    /// - Unknown range (None) -> true (conservative)
    /// - Range [low, high] where low <= 0 <= high -> true
    /// - Range [1, 10] -> false (excludes zero)
    /// - Range [-10, -1] -> false (excludes zero)
    pub fn may_be_zero(&self) -> bool {
        match &self.range_ {
            None => true, // Unknown range, conservatively true
            Some((low, high)) => {
                let low = low.unwrap_or(i64::MIN);
                let high = high.unwrap_or(i64::MAX);
                low <= 0 && 0 <= high
            }
        }
    }

    /// CAP-AI-06: Check if value could be null/None
    ///
    /// Returns true if the value might be null, indicating potential
    /// null dereference if used for attribute access.
    ///
    /// # Logic
    ///
    /// - Never -> false (safe to dereference)
    /// - Maybe -> true (might be null)
    /// - Always -> true (definitely null)
    pub fn may_be_null(&self) -> bool {
        self.nullable != Nullability::Never
    }

    /// Check if this is a known constant value
    ///
    /// Returns true if the constant field is set.
    pub fn is_constant(&self) -> bool {
        self.constant.is_some()
    }

    /// Convert to JSON-serializable format
    ///
    /// # JSON Format
    ///
    /// ```json
    /// {
    ///   "type": "int",           // or null if unknown
    ///   "range": [5, 5],         // [low, high], null = infinity
    ///   "nullable": "never",     // "never" | "maybe" | "always"
    ///   "constant": 5            // only if known constant
    /// }
    /// ```
    pub fn to_json_value(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();

        if let Some(ref t) = self.type_ {
            obj.insert("type".to_string(), serde_json::json!(t));
        }

        if let Some((low, high)) = &self.range_ {
            // TIGER-PASS2-5: null in array = infinity
            let range = serde_json::json!([low, high]);
            obj.insert("range".to_string(), range);
        }

        obj.insert(
            "nullable".to_string(),
            serde_json::json!(self.nullable.as_str()),
        );

        if let Some(ref c) = self.constant {
            obj.insert("constant".to_string(), c.to_json_value());
        }

        serde_json::Value::Object(obj)
    }
}

// =============================================================================
// CAP-AI-07: AbstractState
// =============================================================================

/// Abstract state at a program point: mapping from variables to abstract values.
///
/// Represents the known information about all variables at a specific point
/// in the program. This is the dataflow fact for abstract interpretation.
///
/// # Immutable Update Pattern
///
/// AbstractState uses an immutable update pattern where `set()` returns
/// a new state rather than mutating in place. This makes dataflow analysis
/// easier to reason about.
///
/// # Default Values
///
/// Variables not in the map are treated as `top()` (unknown).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AbstractState {
    /// Mapping from variable names to their abstract values
    pub values: HashMap<String, AbstractValue>,
}

impl AbstractState {
    /// Create a new empty state
    pub fn new() -> Self {
        Self::default()
    }

    /// Get abstract value for variable, defaulting to top (unknown)
    ///
    /// # Returns
    ///
    /// The abstract value for the variable if known, otherwise `top()`.
    pub fn get(&self, var: &str) -> AbstractValue {
        self.values
            .get(var)
            .cloned()
            .unwrap_or_else(AbstractValue::top)
    }

    /// Return new state with updated variable value (immutable style)
    ///
    /// Creates a new AbstractState with the variable set to the given value.
    /// The original state is unchanged.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let state1 = AbstractState::new();
    /// let state2 = state1.set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
    /// // state1 is unchanged
    /// // state2 has x = 5
    /// ```
    pub fn set(&self, var: &str, value: AbstractValue) -> Self {
        let mut new_values = self.values.clone();
        new_values.insert(var.to_string(), value);
        AbstractState { values: new_values }
    }

    /// Create a copy of this state
    ///
    /// Equivalent to clone() but with explicit semantics.
    pub fn copy(&self) -> Self {
        self.clone()
    }
}

// =============================================================================
// CAP-AI-21 & CAP-AI-22: AbstractInterpInfo
// =============================================================================

/// Abstract interpretation analysis results for a function.
///
/// Contains the dataflow information at each block entry/exit,
/// plus detected potential issues (div-by-zero, null deref).
///
/// # Query Methods
///
/// The struct provides convenient query methods:
/// - `value_at(block, var)` - Get value at block entry
/// - `value_at_exit(block, var)` - Get value at block exit
/// - `range_at(block, var)` - Get range at block entry
/// - `type_at(block, var)` - Get type at block entry
/// - `is_definitely_not_null(block, var)` - Check if non-null at block entry
/// - `get_constants()` - Get all constant values at function exit
///
/// # JSON Output (CAP-AI-22)
///
/// ```json
/// {
///   "function": "example",
///   "state_in": { "0": { "x": {...} }, ... },
///   "state_out": { "0": { "x": {...} }, ... },
///   "potential_div_zero": [{"line": 10, "var": "y"}],
///   "potential_null_deref": [{"line": 15, "var": "obj"}]
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct AbstractInterpInfo {
    /// Abstract state at entry of each block
    pub state_in: HashMap<BlockId, AbstractState>,

    /// Abstract state at exit of each block
    pub state_out: HashMap<BlockId, AbstractState>,

    /// CAP-AI-10: Potential division-by-zero warnings (line, var)
    pub potential_div_zero: Vec<(usize, String)>,

    /// CAP-AI-11: Potential null dereference warnings (line, var)
    pub potential_null_deref: Vec<(usize, String)>,

    /// Function name
    pub function_name: String,
}

impl AbstractInterpInfo {
    /// Create a new empty result for a function
    pub fn new(function_name: &str) -> Self {
        Self {
            function_name: function_name.to_string(),
            ..Default::default()
        }
    }

    /// Get abstract value of variable at entry to block
    ///
    /// Returns top() if the block is not found or variable is not tracked.
    pub fn value_at(&self, block: BlockId, var: &str) -> AbstractValue {
        self.state_in
            .get(&block)
            .map(|s| s.get(var))
            .unwrap_or_else(AbstractValue::top)
    }

    /// Get abstract value of variable at exit of block
    ///
    /// Returns top() if the block is not found or variable is not tracked.
    pub fn value_at_exit(&self, block: BlockId, var: &str) -> AbstractValue {
        self.state_out
            .get(&block)
            .map(|s| s.get(var))
            .unwrap_or_else(AbstractValue::top)
    }

    /// Get the value range for variable at block entry
    ///
    /// Returns None if the variable has no range information.
    pub fn range_at(&self, block: BlockId, var: &str) -> Option<(Option<i64>, Option<i64>)> {
        self.value_at(block, var).range_
    }

    /// Get the inferred type for variable at block entry
    ///
    /// Returns None if the type is unknown.
    pub fn type_at(&self, block: BlockId, var: &str) -> Option<String> {
        self.value_at(block, var).type_
    }

    /// Check if variable is definitely non-null at block entry
    ///
    /// Returns true only if nullable == Never.
    pub fn is_definitely_not_null(&self, block: BlockId, var: &str) -> bool {
        self.value_at(block, var).nullable == Nullability::Never
    }

    /// CAP-AI-12: Get all variables with known constant values at function exit
    ///
    /// Scans all state_out blocks and collects variables with constant values.
    pub fn get_constants(&self) -> HashMap<String, ConstantValue> {
        let mut constants = HashMap::new();
        for state in self.state_out.values() {
            for (var, val) in &state.values {
                if let Some(c) = &val.constant {
                    constants.insert(var.clone(), c.clone());
                }
            }
        }
        constants
    }

    /// CAP-AI-22: Serialize to JSON-compatible structure
    ///
    /// Output format matches v1 CLI for compatibility.
    pub fn to_json(&self) -> serde_json::Value {
        let state_in: HashMap<String, serde_json::Value> = self
            .state_in
            .iter()
            .map(|(k, state)| {
                let vars: HashMap<String, serde_json::Value> = state
                    .values
                    .iter()
                    .map(|(var, val)| (var.clone(), val.to_json_value()))
                    .collect();
                (k.to_string(), serde_json::json!(vars))
            })
            .collect();

        let state_out: HashMap<String, serde_json::Value> = self
            .state_out
            .iter()
            .map(|(k, state)| {
                let vars: HashMap<String, serde_json::Value> = state
                    .values
                    .iter()
                    .map(|(var, val)| (var.clone(), val.to_json_value()))
                    .collect();
                (k.to_string(), serde_json::json!(vars))
            })
            .collect();

        let div_zero: Vec<_> = self
            .potential_div_zero
            .iter()
            .map(|(line, var)| serde_json::json!({"line": line, "var": var}))
            .collect();

        let null_deref: Vec<_> = self
            .potential_null_deref
            .iter()
            .map(|(line, var)| serde_json::json!({"line": line, "var": var}))
            .collect();

        serde_json::json!({
            "function": self.function_name,
            "state_in": state_in,
            "state_out": state_out,
            "potential_div_zero": div_zero,
            "potential_null_deref": null_deref,
        })
    }
}

// =============================================================================
// CAP-AI-15, CAP-AI-16, CAP-AI-17: Multi-Language Support (Phase 8)
// =============================================================================

/// CAP-AI-15: Get null-like keywords for a language.
///
/// Returns keywords that represent null/nil/None values in the given language.
///
/// # Language Support Table
///
/// | Language | Null Keywords |
/// |----------|--------------|
/// | Python | `["None"]` |
/// | TypeScript/JavaScript | `["null", "undefined"]` |
/// | Go | `["nil"]` |
/// | Rust | `[]` (no null keyword, uses `Option`) |
/// | Java/Kotlin/C# | `["null"]` |
/// | Swift | `["nil"]` |
/// | Unknown | `["null", "nil", "None"]` (fallback) |
///
/// # TIGER-PASS1-13 Mitigation
///
/// Covers all Language enum values with a sensible fallback for unknown languages.
///
/// # TIGER-PASS2-9 Mitigation (Go)
///
/// Note: Go nil detection is limited without type information. Go uses zero
/// values for uninitialized variables (e.g., 0 for int, "" for string), which
/// are distinct from nil. This function only detects explicit `nil` keywords.
///
/// # Examples
///
/// ```rust,ignore
/// let keywords = get_null_keywords("python");
/// assert!(keywords.contains(&"None"));
///
/// let keywords = get_null_keywords("rust");
/// assert!(keywords.is_empty()); // Rust has no null keyword
/// ```
pub fn get_null_keywords(language: &str) -> Vec<&'static str> {
    match language.to_lowercase().as_str() {
        "python" => vec!["None"],
        "typescript" | "javascript" => vec!["null", "undefined"],
        "go" => vec!["nil"],
        "rust" => vec![], // Rust has no null (None is Option::None, not a keyword)
        "java" | "kotlin" | "csharp" | "c#" => vec!["null"],
        "swift" => vec!["nil"],
        _ => vec!["null", "nil", "None"], // Fallback for unknown languages
    }
}

/// CAP-AI-16: Get boolean keywords for a language.
///
/// Returns a mapping from boolean keyword strings to their boolean values.
///
/// # Language Support Table
///
/// | Language | True Keyword | False Keyword |
/// |----------|-------------|---------------|
/// | Python | `True` | `False` |
/// | TypeScript/JavaScript/Go/Rust | `true` | `false` |
/// | Unknown | Both forms (fallback) |
///
/// # Examples
///
/// ```rust,ignore
/// let bools = get_boolean_keywords("python");
/// assert_eq!(bools.get("True"), Some(&true));
/// assert_eq!(bools.get("False"), Some(&false));
///
/// let bools = get_boolean_keywords("typescript");
/// assert_eq!(bools.get("true"), Some(&true));
/// assert_eq!(bools.get("false"), Some(&false));
/// ```
pub fn get_boolean_keywords(language: &str) -> HashMap<&'static str, bool> {
    match language.to_lowercase().as_str() {
        "python" => [("True", true), ("False", false)].into_iter().collect(),
        "typescript" | "javascript" | "go" | "rust" | "java" | "kotlin" | "csharp" | "c#"
        | "swift" => [("true", true), ("false", false)].into_iter().collect(),
        _ => {
            // Fallback: accept both forms for unknown languages
            [
                ("True", true),
                ("False", false),
                ("true", true),
                ("false", false),
            ]
            .into_iter()
            .collect()
        }
    }
}

/// CAP-AI-17: Get single-line comment pattern for a language.
///
/// Returns the string that starts a single-line comment in the given language.
///
/// # Language Support Table
///
/// | Language | Comment Pattern |
/// |----------|----------------|
/// | Python | `#` |
/// | TypeScript/JavaScript/Go/Rust/Java/C#/Kotlin/Swift | `//` |
/// | Unknown | `#` (fallback) |
///
/// # Note
///
/// This only handles single-line comments. Multi-line comments (`/* */`)
/// are not stripped by this pattern. This is a documented MVP limitation
/// (TIGER-PASS1-14).
///
/// # Examples
///
/// ```rust,ignore
/// let pattern = get_comment_pattern("python");
/// assert_eq!(pattern, "#");
///
/// let pattern = get_comment_pattern("typescript");
/// assert_eq!(pattern, "//");
/// ```
pub fn get_comment_pattern(language: &str) -> &'static str {
    match language.to_lowercase().as_str() {
        "python" => "#",
        "typescript" | "javascript" | "go" | "rust" | "java" | "csharp" | "c#" | "kotlin"
        | "swift" => "//",
        _ => "#", // Fallback
    }
}

// =============================================================================
// CAP-AI-14: RHS Parsing for Assignments (Phase 9)
// =============================================================================

/// Strip single-line comment from end of line.
///
/// # TIGER-PASS1-14 Mitigation
///
/// Only handles single-line comments (# for Python, // for most others).
/// Multi-line comments (/* */) are NOT stripped - this is a documented
/// MVP limitation.
///
/// # Arguments
///
/// * `line` - Source line to strip comment from
/// * `language` - Language identifier for comment pattern
///
/// # Returns
///
/// Line with trailing comment removed
///
/// # Examples
///
/// ```rust,ignore
/// let stripped = strip_comment("x = 5  # comment", "python");
/// assert_eq!(stripped, "x = 5  ");
///
/// let stripped = strip_comment("x = 5  // comment", "typescript");
/// assert_eq!(stripped, "x = 5  ");
/// ```
pub fn strip_comment<'a>(line: &'a str, language: &str) -> &'a str {
    let pattern = get_comment_pattern(language);

    // Handle strings: don't strip if comment marker is inside a string
    // This is a simplified check - look for comment marker outside quotes
    let mut in_string = false;
    let mut string_char: Option<char> = None;
    let mut escape_next = false;

    for (i, c) in line.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }

        if c == '\\' {
            escape_next = true;
            continue;
        }

        if in_string {
            if Some(c) == string_char {
                in_string = false;
                string_char = None;
            }
        } else if c == '"' || c == '\'' {
            in_string = true;
            string_char = Some(c);
        } else if line[i..].starts_with(pattern) {
            return &line[..i];
        }
    }

    line
}

/// Replace string literal contents with spaces, preserving positions.
///
/// Walks the line character-by-character. When inside a quoted string
/// (`"`, `'`, or backtick), every character (except the delimiters
/// themselves) is replaced with a space. This prevents text-level scanners
/// (e.g. `find_div_zero`) from matching operators inside string literals.
///
/// Handles escape sequences (`\"`, `\'`, `\\`) and Rust raw strings
/// (`r"..."`, `r#"..."#`, `r##"..."##`, etc.).
pub fn strip_strings(line: &str, language: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        let c = bytes[i];

        // --- Rust raw strings: r"...", r#"..."#, r##"..."##, etc. ---
        if language == "rust" && c == b'r' {
            // Count hashes after 'r'
            let mut hashes = 0;
            let mut j = i + 1;
            while j < len && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < len && bytes[j] == b'"' {
                // This is a raw string: r#"..."#
                // Keep the r, hashes, and opening quote as-is
                for &b in &bytes[i..=j] {
                    result.push(b as char);
                }
                i = j + 1;
                // Now blank everything until closing: "###
                let close_start = b'"';
                loop {
                    if i >= len {
                        break; // Unterminated raw string
                    }
                    if bytes[i] == close_start {
                        // Check if followed by the right number of hashes
                        let mut matched = 0;
                        let mut k = i + 1;
                        while k < len && bytes[k] == b'#' && matched < hashes {
                            matched += 1;
                            k += 1;
                        }
                        if matched == hashes {
                            // Found the closing delimiter
                            for &b in &bytes[i..k] {
                                result.push(b as char);
                            }
                            i = k;
                            break;
                        }
                    }
                    // Inside raw string: blank
                    result.push(' ');
                    i += 1;
                }
                continue;
            }
            // Not a raw string, fall through to normal processing
        }

        // --- Regular strings: "...", '...', `...` ---
        if c == b'"' || c == b'\'' || c == b'`' {
            let delim = c;
            result.push(c as char);
            i += 1;
            while i < len {
                if bytes[i] == b'\\' {
                    // Escape: blank both backslash and next char
                    result.push(' ');
                    i += 1;
                    if i < len {
                        result.push(' ');
                        i += 1;
                    }
                } else if bytes[i] == delim {
                    // Closing delimiter: keep it
                    result.push(delim as char);
                    i += 1;
                    break;
                } else {
                    // Inside string: blank
                    result.push(' ');
                    i += 1;
                }
            }
            continue;
        }

        // --- Normal code: keep as-is ---
        result.push(c as char);
        i += 1;
    }

    result
}

/// Check if a string is a valid identifier (variable name).
///
/// Identifiers start with a letter or underscore, followed by
/// letters, digits, or underscores.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(is_identifier("foo"));
/// assert!(is_identifier("_bar"));
/// assert!(is_identifier("var123"));
/// assert!(!is_identifier("123var"));
/// assert!(!is_identifier("foo.bar"));
/// ```
pub fn is_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Extract RHS from assignment line.
///
/// Handles both regular assignments (`var = expr`) and augmented assignments
/// (`var += expr`, `var -= expr`, etc.).
///
/// # TIGER-PASS2-2 Mitigation
///
/// Augmented assignments are converted to regular assignment form:
/// - `x += 5` becomes `x + 5` (as if from `x = x + 5`)
/// - `x -= 3` becomes `x - 3`
/// - `x *= 2` becomes `x * 2`
///
/// # Arguments
///
/// * `line` - Source line containing the assignment
/// * `var` - Variable being assigned to
///
/// # Returns
///
/// The RHS expression as a string, or None if not found
///
/// # Examples
///
/// ```rust,ignore
/// let rhs = extract_rhs("x = a + b", "x");
/// assert_eq!(rhs, Some("a + b".to_string()));
///
/// let rhs = extract_rhs("x += 5", "x");
/// assert_eq!(rhs, Some("x + 5".to_string()));
/// ```
pub fn extract_rhs(line: &str, var: &str) -> Option<String> {
    let line = line.trim();

    // Check for augmented assignment first: var += val, var -= val, var *= val
    let augmented_ops = &[
        ("+=", '+'),
        ("-=", '-'),
        ("*=", '*'),
        ("/=", '/'),
        ("%=", '%'),
    ];

    for (op_str, op_char) in augmented_ops {
        // Pattern: "var += expr" or "var+= expr" or "var +=expr"
        let pattern_spaced = format!("{} {} ", var, op_str);
        let pattern_left_space = format!("{} {}", var, op_str);
        let pattern_right_space = format!("{}{} ", var, op_str);
        let pattern_no_space = format!("{}{}", var, op_str);

        if let Some(idx) = line.find(&pattern_spaced) {
            if idx == 0
                || !line[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false)
            {
                let rhs_start = idx + pattern_spaced.len();
                let rhs = line[rhs_start..].trim();
                // Convert augmented to: var op rhs
                return Some(format!("{} {} {}", var, op_char, rhs));
            }
        }

        if let Some(idx) = line.find(&pattern_left_space) {
            if idx == 0
                || !line[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false)
            {
                let rhs_start = idx + pattern_left_space.len();
                let rhs = line[rhs_start..].trim();
                return Some(format!("{} {} {}", var, op_char, rhs));
            }
        }

        if let Some(idx) = line.find(&pattern_right_space) {
            if idx == 0
                || !line[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false)
            {
                let rhs_start = idx + pattern_right_space.len();
                let rhs = line[rhs_start..].trim();
                return Some(format!("{} {} {}", var, op_char, rhs));
            }
        }

        if let Some(idx) = line.find(&pattern_no_space) {
            if idx == 0
                || !line[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false)
            {
                let rhs_start = idx + pattern_no_space.len();
                let rhs = line[rhs_start..].trim();
                return Some(format!("{} {} {}", var, op_char, rhs));
            }
        }
    }

    // Regular assignment: var = expr
    // Need to find "var =" or "var=" pattern
    let patterns = [
        format!("{} = ", var),
        format!("{}= ", var),
        format!("{} =", var),
        format!("{}=", var),
    ];

    for pattern in &patterns {
        if let Some(idx) = line.find(pattern) {
            // Make sure we're matching the whole variable name
            // Check that character before (if any) is not alphanumeric
            let valid_start = idx == 0
                || !line[..idx]
                    .chars()
                    .last()
                    .map(|c| c.is_alphanumeric() || c == '_')
                    .unwrap_or(false);

            if valid_start {
                let rhs_start = idx + pattern.len();
                return Some(line[rhs_start..].trim().to_string());
            }
        }
    }

    // Handle walrus operator for Python (:=)
    let walrus_pattern = format!("{} := ", var);
    if let Some(idx) = line.find(&walrus_pattern) {
        let valid_start = idx == 0
            || !line[..idx]
                .chars()
                .last()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);

        if valid_start {
            let rhs_start = idx + walrus_pattern.len();
            return Some(line[rhs_start..].trim().to_string());
        }
    }

    None
}

/// Parse simple arithmetic expression: "var op const" or "const op var"
///
/// # Supported Patterns
///
/// - `a + 1`, `a - 1`, `a * 2`
/// - `1 + a`, `2 * a`
///
/// # Arguments
///
/// * `rhs` - Right-hand side expression string
///
/// # Returns
///
/// Tuple of (variable_name, operator, constant_value) if pattern matches
///
/// # Examples
///
/// ```rust,ignore
/// let result = parse_simple_arithmetic("a + 1");
/// assert_eq!(result, Some(("a".to_string(), '+', 1)));
///
/// let result = parse_simple_arithmetic("3 * x");
/// assert_eq!(result, Some(("x".to_string(), '*', 3)));
/// ```
pub fn parse_simple_arithmetic(rhs: &str) -> Option<(String, char, i64)> {
    let rhs = rhs.trim();

    // Look for arithmetic operators: +, -, *
    // Try to parse patterns like: "var + const" or "const + var"
    for op in ['+', '-', '*'] {
        // Handle both "a + b" and "a+b" formats
        let parts: Vec<&str> = if rhs.contains(&format!(" {} ", op)) {
            rhs.splitn(2, &format!(" {} ", op)).collect()
        } else if rhs.contains(op) {
            rhs.splitn(2, op).collect()
        } else {
            continue;
        };

        if parts.len() != 2 {
            continue;
        }

        let left = parts[0].trim();
        let right = parts[1].trim();

        // Try: var op const
        if is_identifier(left) {
            if let Ok(c) = right.parse::<i64>() {
                return Some((left.to_string(), op, c));
            }
        }

        // Try: const op var (only for commutative ops + and *)
        if op == '+' || op == '*' {
            if let Ok(c) = left.parse::<i64>() {
                if is_identifier(right) {
                    return Some((right.to_string(), op, c));
                }
            }
        }
    }

    None
}

/// Parse RHS of assignment and compute abstract value.
///
/// # CAP-AI-14: RHS Parsing
///
/// Handles the following RHS patterns:
/// - Integer literals: `x = 5` -> `from_constant(Int(5))`
/// - Float literals: `x = 3.14` -> `from_constant(Float(3.14))`
/// - String literals: `x = "hello"` or `x = 'hello'` -> `from_constant(String("hello"))`
/// - Boolean literals: `x = True/true` -> `from_constant(Bool(true))`
/// - Null literals: `x = None/null/nil` -> `from_constant(Null)`
/// - Variable copies: `x = y` -> `state.get("y")`
/// - Simple arithmetic: `x = a + 1` -> `apply_arithmetic(state.get("a"), '+', 1)`
/// - Augmented assignment: `x += 1` treated as `x = x + 1`
///
/// # TIGER Mitigations
///
/// - TIGER-PASS2-2: Augmented assignments (+=, -=, *=) converted to regular assignments
/// - TIGER-PASS1-14: Only single-line comments are stripped
///
/// # Arguments
///
/// * `line` - Source line containing the assignment
/// * `var` - Variable being assigned to
/// * `state` - Current abstract state (for variable lookups)
/// * `language` - Language identifier (for null/boolean keywords)
///
/// # Returns
///
/// Abstract value representing the RHS expression
///
/// # Examples
///
/// ```rust,ignore
/// let state = AbstractState::new();
/// let val = parse_rhs_abstract("x = 5", "x", &state, "python");
/// assert_eq!(val.range_, Some((Some(5), Some(5))));
/// ```
pub fn parse_rhs_abstract(
    line: &str,
    var: &str,
    state: &AbstractState,
    language: &str,
) -> AbstractValue {
    // Strip comments first
    let line = strip_comment(line, language);

    // Extract the RHS expression
    let rhs = match extract_rhs(line, var) {
        Some(r) => r,
        None => return AbstractValue::top(),
    };

    let rhs = rhs.trim();

    // Empty RHS
    if rhs.is_empty() {
        return AbstractValue::top();
    }

    // Integer literal (including negative)
    if let Ok(v) = rhs.parse::<i64>() {
        return AbstractValue::from_constant(ConstantValue::Int(v));
    }

    // Float literal (including negative)
    // Must check after integer to avoid matching "5" as float
    if rhs.contains('.') || rhs.to_lowercase().contains('e') {
        if let Ok(v) = rhs.parse::<f64>() {
            return AbstractValue::from_constant(ConstantValue::Float(v));
        }
    }

    // String literal (double or single quotes)
    if (rhs.starts_with('"') && rhs.ends_with('"') && rhs.len() >= 2)
        || (rhs.starts_with('\'') && rhs.ends_with('\'') && rhs.len() >= 2)
    {
        let s = rhs[1..rhs.len() - 1].to_string();
        return AbstractValue::from_constant(ConstantValue::String(s));
    }

    // Triple-quoted strings (Python)
    if (rhs.starts_with("\"\"\"") && rhs.ends_with("\"\"\"") && rhs.len() >= 6)
        || (rhs.starts_with("'''") && rhs.ends_with("'''") && rhs.len() >= 6)
    {
        let s = rhs[3..rhs.len() - 3].to_string();
        return AbstractValue::from_constant(ConstantValue::String(s));
    }

    // Null keywords (language-specific via CAP-AI-15)
    let null_keywords = get_null_keywords(language);
    if null_keywords.contains(&rhs) {
        // Handle TypeScript undefined specially (TIGER-PASS2-8)
        if rhs == "undefined" {
            return AbstractValue {
                type_: Some("undefined".to_string()),
                range_: None,
                nullable: Nullability::Always,
                constant: None,
            };
        }
        return AbstractValue::from_constant(ConstantValue::Null);
    }

    // Boolean keywords (language-specific via CAP-AI-16)
    let bool_keywords = get_boolean_keywords(language);
    if let Some(&b) = bool_keywords.get(rhs) {
        return AbstractValue::from_constant(ConstantValue::Bool(b));
    }

    // Variable copy: x = y (where y is a simple identifier)
    if is_identifier(rhs) {
        return state.get(rhs);
    }

    // Simple arithmetic: x = a + 1 or x = a - 1 (CAP-AI-13)
    if let Some((operand_var, op, constant)) = parse_simple_arithmetic(rhs) {
        let operand_value = state.get(&operand_var);
        return apply_arithmetic(&operand_value, op, constant);
    }

    // Unknown RHS - return top (unknown)
    AbstractValue::top()
}

// =============================================================================
// CAP-AI-13: Abstract Arithmetic Operations (Phase 7)
// =============================================================================

/// Apply arithmetic operation to abstract value.
///
/// # CRITICAL: TIGER-PASS1-11 Mitigation
///
/// Uses saturating arithmetic to prevent overflow panic.
/// On overflow, the bound is widened to unbounded (None).
///
/// # Supported Operations
///
/// - `'+'`: Addition - adds constant to both bounds
/// - `'-'`: Subtraction - subtracts constant from both bounds
/// - `'*'`: Multiplication - multiplies bounds by constant (handles sign changes)
///
/// # Examples
///
/// ```rust,ignore
/// let val = AbstractValue::from_constant(ConstantValue::Int(5));
/// let result = apply_arithmetic(&val, '+', 3);
/// // result.range_ == Some((Some(8), Some(8)))
///
/// let range_val = AbstractValue {
///     type_: Some("int".to_string()),
///     range_: Some((Some(1), Some(5))),
///     nullable: Nullability::Never,
///     constant: None,
/// };
/// let result = apply_arithmetic(&range_val, '+', 10);
/// // result.range_ == Some((Some(11), Some(15)))
/// ```
///
/// # Overflow Handling
///
/// When saturating arithmetic reaches i64::MAX or i64::MIN, the bound
/// is widened to None (unbounded) to maintain soundness:
///
/// ```rust,ignore
/// let max_val = AbstractValue::from_constant(ConstantValue::Int(i64::MAX));
/// let result = apply_arithmetic(&max_val, '+', 1);
/// // result.range_ contains None bounds - widened to unbounded
/// ```
pub fn apply_arithmetic(operand: &AbstractValue, op: char, constant: i64) -> AbstractValue {
    let new_range = operand.range_.map(|(low, high)| {
        match op {
            '+' => {
                // TIGER-PASS1-11: Use saturating_add
                let new_low = low.and_then(|l| {
                    let result = l.saturating_add(constant);
                    // If saturated to MAX/MIN, widen to unbounded
                    if (constant > 0 && result == i64::MAX && l != i64::MAX - constant)
                        || (constant < 0 && result == i64::MIN && l != i64::MIN - constant)
                    {
                        return None;
                    }
                    Some(result)
                });

                let new_high = high.and_then(|h| {
                    let result = h.saturating_add(constant);
                    // If saturated to MAX/MIN, widen to unbounded
                    if (constant > 0 && result == i64::MAX && h != i64::MAX - constant)
                        || (constant < 0 && result == i64::MIN && h != i64::MIN - constant)
                    {
                        return None;
                    }
                    Some(result)
                });

                (new_low, new_high)
            }
            '-' => {
                // TIGER-PASS1-11: Use saturating_sub
                let new_low = low.and_then(|l| {
                    let result = l.saturating_sub(constant);
                    // If saturated to MAX/MIN, widen to unbounded
                    if (constant > 0 && result == i64::MIN && l != i64::MIN + constant)
                        || (constant < 0 && result == i64::MAX && l != i64::MAX + constant)
                    {
                        return None;
                    }
                    Some(result)
                });

                let new_high = high.and_then(|h| {
                    let result = h.saturating_sub(constant);
                    // If saturated to MAX/MIN, widen to unbounded
                    if (constant > 0 && result == i64::MIN && h != i64::MIN + constant)
                        || (constant < 0 && result == i64::MAX && h != i64::MAX + constant)
                    {
                        return None;
                    }
                    Some(result)
                });

                (new_low, new_high)
            }
            '*' => {
                // TIGER-PASS1-11: Handle sign changes for multiplication
                // When multiplying by negative, low and high swap
                // Use saturating_mul and detect overflow

                let compute_mul = |bound: Option<i64>| -> Option<i64> {
                    bound.and_then(|b| {
                        // Check for overflow before multiplying
                        if constant == 0 {
                            return Some(0);
                        }
                        // Use checked_mul to detect overflow (None = overflow -> unbounded)
                        b.checked_mul(constant)
                    })
                };

                let low_mul = compute_mul(low);
                let high_mul = compute_mul(high);

                // When multiplying by negative constant, bounds swap
                if constant < 0 {
                    (high_mul, low_mul)
                } else if constant == 0 {
                    // Multiplying by zero gives exact [0, 0]
                    (Some(0), Some(0))
                } else {
                    (low_mul, high_mul)
                }
            }
            _ => {
                // Unknown operator -> widen to unbounded
                (None, None)
            }
        }
    });

    // Determine if result is still a constant
    let new_constant = if operand.is_constant() {
        if let Some((Some(l), Some(h))) = new_range {
            if l == h {
                Some(ConstantValue::Int(l))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    AbstractValue {
        type_: operand.type_.clone(),
        range_: new_range,
        nullable: operand.nullable,
        constant: new_constant,
    }
}

// =============================================================================
// CAP-AI-08: Join Operations (Phase 6)
// =============================================================================

/// Join two abstract values at a merge point.
///
/// Combines two abstract values by taking the least upper bound:
/// - Ranges: union bounds -> [min(low1, low2), max(high1, high2)]
/// - Constants: lose if disagree, keep if same
/// - Nullability: NEVER + NEVER = NEVER, else MAYBE
/// - Types: lose if disagree, keep if same
///
/// # Examples
///
/// ```rust,ignore
/// // Range union
/// let val1 = AbstractValue { range_: Some((Some(1), Some(1))), .. };
/// let val2 = AbstractValue { range_: Some((Some(10), Some(10))), .. };
/// let joined = join_values(&val1, &val2);
/// assert_eq!(joined.range_, Some((Some(1), Some(10))));
/// ```
pub fn join_values(a: &AbstractValue, b: &AbstractValue) -> AbstractValue {
    // Range: union (widest bounds)
    let joined_range = match (&a.range_, &b.range_) {
        (None, None) => None,
        (Some(r), None) | (None, Some(r)) => Some(*r),
        (Some((a_low, a_high)), Some((b_low, b_high))) => {
            // Take minimum of lows and maximum of highs
            let low = match (a_low, b_low) {
                (None, _) | (_, None) => None,
                (Some(a), Some(b)) => Some(std::cmp::min(*a, *b)),
            };
            let high = match (a_high, b_high) {
                (None, _) | (_, None) => None,
                (Some(a), Some(b)) => Some(std::cmp::max(*a, *b)),
            };
            Some((low, high))
        }
    };

    // Type: common type or None
    let joined_type = if a.type_ == b.type_ {
        a.type_.clone()
    } else {
        None
    };

    // Nullable: NEVER only if both are NEVER, else MAYBE
    let joined_nullable = match (a.nullable, b.nullable) {
        (Nullability::Never, Nullability::Never) => Nullability::Never,
        (Nullability::Always, Nullability::Always) => Nullability::Always,
        _ => Nullability::Maybe,
    };

    // Constant: only if both have same constant
    let joined_constant = match (&a.constant, &b.constant) {
        (Some(ca), Some(cb)) if ca == cb => Some(ca.clone()),
        _ => None,
    };

    AbstractValue {
        type_: joined_type,
        range_: joined_range,
        nullable: joined_nullable,
        constant: joined_constant,
    }
}

/// Join multiple abstract states at a CFG merge point.
///
/// For each variable present in any input state:
///   result[var] = join of all values for var
///
/// Variables not present in a state are treated as `top()`.
///
/// # Arguments
///
/// * `states` - Slice of references to states to join
///
/// # Returns
///
/// New state containing joined values for all variables
pub fn join_states(states: &[&AbstractState]) -> AbstractState {
    if states.is_empty() {
        return AbstractState::default();
    }
    if states.len() == 1 {
        return states[0].clone();
    }

    // Collect all variable names from all states
    let all_vars: std::collections::HashSet<_> = states
        .iter()
        .flat_map(|s| s.values.keys().cloned())
        .collect();

    let mut result = HashMap::new();
    for var in all_vars {
        // Get values from all states (top() for missing)
        let values: Vec<AbstractValue> = states.iter().map(|s| s.get(&var)).collect();

        // Join all values pairwise
        let mut joined = values[0].clone();
        for val in values.iter().skip(1) {
            joined = join_values(&joined, val);
        }
        result.insert(var, joined);
    }

    AbstractState { values: result }
}

// =============================================================================
// CAP-AI-09: Widening Operations (Phase 6)
// =============================================================================

/// Widen a value to ensure termination on loops.
///
/// Compares old and new values and widens bounds that are growing:
/// - If new.low < old.low, widen low to None (negative infinity)
/// - If new.high > old.high, widen high to None (positive infinity)
/// - Constant information is always lost on widening
///
/// # Arguments
///
/// * `old` - Value from previous iteration
/// * `new` - Value from current iteration
///
/// # Returns
///
/// Widened value that ensures fixpoint convergence
pub fn widen_value(old: &AbstractValue, new: &AbstractValue) -> AbstractValue {
    let widened_range = match (&old.range_, &new.range_) {
        (None, None) => None,
        (None, r) => *r,
        (_, None) => None, // New has unbounded range, keep it
        (Some((old_low, old_high)), Some((new_low, new_high))) => {
            // Widen low: if growing downward (more negative), widen to -inf
            let widened_low = match (old_low, new_low) {
                (None, _) => None,                     // Already widened
                (_, None) => None,                     // Widen to -inf
                (Some(o), Some(n)) if *n < *o => None, // Growing down -> widen
                (_, n) => *n,                          // Not growing, keep new value
            };

            // Widen high: if growing upward (more positive), widen to +inf
            let widened_high = match (old_high, new_high) {
                (None, _) => None,                     // Already widened
                (_, None) => None,                     // Widen to +inf
                (Some(o), Some(n)) if *n > *o => None, // Growing up -> widen
                (_, n) => *n,                          // Not growing, keep new value
            };

            Some((widened_low, widened_high))
        }
    };

    AbstractValue {
        type_: new.type_.clone(),
        range_: widened_range,
        nullable: new.nullable,
        constant: None, // CAP-AI-09: Constant lost after widening
    }
}

/// Widen state at loop headers to ensure termination.
///
/// Applies widening to each variable present in either state.
///
/// # Arguments
///
/// * `old` - State from previous iteration
/// * `new` - State from current iteration
///
/// # Returns
///
/// Widened state
pub fn widen_state(old: &AbstractState, new: &AbstractState) -> AbstractState {
    // Collect all variable names from both states
    let all_vars: std::collections::HashSet<_> = old
        .values
        .keys()
        .chain(new.values.keys())
        .cloned()
        .collect();

    let mut result = HashMap::new();
    for var in all_vars {
        let old_val = old.get(&var);
        let new_val = new.get(&var);
        result.insert(var, widen_value(&old_val, &new_val));
    }

    AbstractState { values: result }
}

// =============================================================================
// CAP-AI: Main Algorithm - compute_abstract_interp (Phase 10)
// =============================================================================

use super::types::{
    build_predecessors, find_back_edges, reverse_postorder, validate_cfg, DataflowError,
};
use crate::types::{CfgInfo, DfgInfo, RefType, VarRef};

/// Initialize parameter values as top() (unknown).
///
/// Parameters are identified from VarRefs as definitions in the entry block
/// that appear without prior use (typical function parameter pattern).
///
/// All parameters start as top() because we don't know the caller's values.
///
/// # Arguments
///
/// * `cfg` - Control flow graph with entry block info
/// * `dfg` - Data flow graph with variable references
///
/// # Returns
///
/// AbstractState with all parameters set to top()
pub fn init_params(cfg: &CfgInfo, dfg: &DfgInfo) -> AbstractState {
    let mut state = AbstractState::new();

    // Find the entry block
    let entry_block = cfg.blocks.iter().find(|b| b.id == cfg.entry_block);

    if let Some(entry) = entry_block {
        // Find all definitions in the entry block
        // Parameters are typically defined at the start of the function
        for var_ref in &dfg.refs {
            // A definition in the entry block with no prior use is likely a parameter
            if var_ref.ref_type == RefType::Definition {
                // Check if this line is within the entry block
                if var_ref.line >= entry.lines.0 && var_ref.line <= entry.lines.1 {
                    // Initialize as top (unknown value from caller)
                    state
                        .values
                        .insert(var_ref.name.clone(), AbstractValue::top());
                }
            }
        }
    }

    state
}

/// Transfer function: update state based on block operations.
///
/// Processes all statements in a block and updates the abstract state.
/// Each assignment updates the corresponding variable's abstract value.
///
/// # Algorithm
///
/// For each VarRef of type Def in the block:
/// 1. Get the source line for this definition
/// 2. Parse the RHS to compute the new abstract value
/// 3. Update the state with the new value
///
/// # Arguments
///
/// * `state` - Abstract state at block entry
/// * `block` - CFG block being processed
/// * `dfg` - Data flow graph with variable references
/// * `source_lines` - Optional source code lines for RHS parsing
/// * `language` - Language identifier for keyword recognition
///
/// # Returns
///
/// New AbstractState at block exit
pub fn transfer_block(
    state: &AbstractState,
    block: &crate::types::CfgBlock,
    dfg: &DfgInfo,
    source_lines: Option<&[&str]>,
    language: &str,
) -> AbstractState {
    let mut current_state = state.clone();

    // Get all definitions in this block, sorted by line
    let mut defs_in_block: Vec<&VarRef> = dfg
        .refs
        .iter()
        .filter(|r| {
            r.ref_type == RefType::Definition && r.line >= block.lines.0 && r.line <= block.lines.1
        })
        .collect();

    // Sort by line number for correct order of operations
    defs_in_block.sort_by_key(|r| (r.line, r.column));

    // Process each definition in order
    for var_ref in defs_in_block {
        // Get source line if available
        let new_value = if let Some(lines) = source_lines {
            // Convert 1-based line to 0-based index
            let line_idx = var_ref.line.saturating_sub(1) as usize;
            if line_idx < lines.len() {
                let line = lines[line_idx];
                parse_rhs_abstract(line, &var_ref.name, &current_state, language)
            } else {
                AbstractValue::top()
            }
        } else {
            // No source available - default to top
            AbstractValue::top()
        };

        // Update state
        current_state = current_state.set(&var_ref.name, new_value);
    }

    current_state
}

// =============================================================================
// Phase 11: Safety Check Detection (CAP-AI-10, CAP-AI-11, CAP-AI-20)
// =============================================================================

/// Find potential division-by-zero based on range analysis.
///
/// CRITICAL: Intra-block precision (TIGER-PASS1-13)
/// For division at line L in block B:
///   1. Find all defs before L in same block
///   2. If divisor defined before L, use state after that def
///   3. Else use state_in[B]
///
/// # Arguments
///
/// * `cfg` - Control flow graph
/// * `dfg` - Data flow graph with variable references
/// * `state_in` - Abstract state at block entries
/// * `source_lines` - Source code lines for division detection
/// * `state_out` - Abstract state at block exits
///
/// # Returns
///
/// Vec<(line, var)> where divisor may_be_zero()
///
/// # Algorithm (CAP-AI-20)
///
/// 1. Scan source lines for division patterns (/, //, %)
/// 2. For each division, extract the divisor variable
/// 3. Find the containing block and compute state at division point:
///    - If divisor is defined before division line in same block, re-compute
///      the state up to that point
///    - Otherwise, use state_in[block]
/// 4. If divisor.may_be_zero(), add warning
///
/// # ELEPHANT-PASS2-5
///
/// Limitation: Only direct variable divisors are tracked.
/// Complex expressions like `1/(x+y)` are NOT detected.
pub fn find_div_zero(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    state_in: &HashMap<BlockId, AbstractState>,
    source_lines: Option<&[&str]>,
    _state_out: &HashMap<BlockId, AbstractState>,
    language: &str,
) -> Vec<(usize, String)> {
    let mut warnings = Vec::new();

    let Some(lines) = source_lines else {
        return warnings;
    };

    // Division operators by language
    let div_patterns: &[&str] = match language {
        "python" => &["/", "//", "%"],
        "rust" | "go" | "typescript" | "javascript" | "java" | "c" | "cpp" => &["/", "%"],
        _ => &["/", "%"],
    };

    // Process each line looking for divisions
    for (line_idx, line) in lines.iter().enumerate() {
        let line_num = line_idx + 1; // 1-based

        // Skip comments, then blank string literal contents so `/` in
        // paths like "/src/main.rs" is not mistaken for division.
        let code_no_comments = strip_comment(line, language);
        let code = strip_strings(code_no_comments, language);

        // Check for division operators
        for &op in div_patterns {
            // Find all occurrences of the division operator
            let mut search_start = 0;
            while let Some(pos) = code[search_start..].find(op) {
                let actual_pos = search_start + pos;

                // Skip if this is // for integer division and we're at first /
                if op == "/" && code.len() > actual_pos + 1 {
                    let next_char = code.chars().nth(actual_pos + 1);
                    if next_char == Some('/') {
                        // This is // (floor division in Python or comment)
                        search_start = actual_pos + 2;
                        continue;
                    }
                    // Check if this is part of // that we should handle
                    if actual_pos > 0 && code.chars().nth(actual_pos - 1) == Some('/') {
                        search_start = actual_pos + 1;
                        continue;
                    }
                }

                // Extract the divisor (RHS of division)
                let after_op = &code[actual_pos + op.len()..];
                let divisor = extract_divisor(after_op.trim());

                if let Some(div_var) = divisor {
                    if is_identifier(&div_var) {
                        // Find which block contains this line
                        let block = cfg
                            .blocks
                            .iter()
                            .find(|b| line_num as u32 >= b.lines.0 && line_num as u32 <= b.lines.1);

                        if let Some(block) = block {
                            // Intra-block precision: compute state at division point
                            let state_at_div = compute_state_at_line(
                                block,
                                dfg,
                                state_in.get(&block.id).cloned().unwrap_or_default(),
                                source_lines,
                                line_num,
                                language,
                            );

                            let divisor_val = state_at_div.get(&div_var);
                            if divisor_val.may_be_zero() {
                                warnings.push((line_num, div_var));
                            }
                        }
                    }
                }

                search_start = actual_pos + op.len();
            }
        }
    }

    // Deduplicate warnings (same line might have multiple divisions)
    warnings.sort();
    warnings.dedup();

    warnings
}

/// Extract divisor variable from expression after division operator.
///
/// Handles simple cases like: `/ x`, `/ y)`, `/ (a + b)`
/// Only returns identifiers (variables), not complex expressions.
fn extract_divisor(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Collect identifier characters
    let mut chars = s.chars().peekable();

    // Skip leading parenthesis if present (we can't handle complex expressions)
    if chars.peek() == Some(&'(') {
        return None;
    }

    let mut ident = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
            chars.next();
        } else {
            break;
        }
    }

    if ident.is_empty() || ident.chars().next().unwrap().is_ascii_digit() {
        // Not a valid identifier (empty or starts with digit)
        // Note: numeric literals are handled conservatively (may_be_zero returns true for unknown)
        None
    } else {
        Some(ident)
    }
}

/// Compute abstract state at a specific line within a block.
///
/// This provides intra-block precision by replaying the transfer function
/// only up to the specified line.
fn compute_state_at_line(
    block: &crate::types::CfgBlock,
    dfg: &DfgInfo,
    state_in: AbstractState,
    source_lines: Option<&[&str]>,
    target_line: usize,
    language: &str,
) -> AbstractState {
    let mut current_state = state_in;

    // Get all definitions in this block, sorted by line
    let mut defs_in_block: Vec<&VarRef> = dfg
        .refs
        .iter()
        .filter(|r| {
            r.ref_type == RefType::Definition
                && r.line >= block.lines.0
                && r.line <= block.lines.1
                && (r.line as usize) < target_line // Only process defs BEFORE target line
        })
        .collect();

    // Sort by line number for correct order of operations
    defs_in_block.sort_by_key(|r| (r.line, r.column));

    // Process each definition in order
    for var_ref in defs_in_block {
        // Get source line if available
        let new_value = if let Some(lines) = source_lines {
            // Convert 1-based line to 0-based index
            let line_idx = var_ref.line.saturating_sub(1) as usize;
            if line_idx < lines.len() {
                let line = lines[line_idx];
                parse_rhs_abstract(line, &var_ref.name, &current_state, language)
            } else {
                AbstractValue::top()
            }
        } else {
            AbstractValue::top()
        };

        // Update state
        current_state = current_state.set(&var_ref.name, new_value);
    }

    current_state
}

/// Find potential null dereferences at attribute access.
///
/// Looks for patterns: var.attr, var.method(), var[idx]
/// Checks if var.may_be_null() at that point.
///
/// # Arguments
///
/// * `cfg` - Control flow graph
/// * `dfg` - Data flow graph with variable references
/// * `state_in` - Abstract state at block entries
/// * `source_lines` - Source code lines for pattern detection
///
/// # Returns
///
/// Vec<(line, var)> where var may be null at dereference point
pub fn find_null_deref(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    state_in: &HashMap<BlockId, AbstractState>,
    source_lines: Option<&[&str]>,
    language: &str,
) -> Vec<(usize, String)> {
    let mut warnings = Vec::new();

    let Some(lines) = source_lines else {
        return warnings;
    };

    // Process each line looking for attribute access patterns
    for (line_idx, line) in lines.iter().enumerate() {
        let line_num = line_idx + 1; // 1-based

        // Skip comments
        let code = strip_comment(line, language);

        // Find all attribute access patterns: identifier followed by .
        // Pattern: word.something or word[something]
        let patterns = extract_deref_patterns(code);

        for var in patterns {
            if is_identifier(&var) {
                // Find which block contains this line
                let block = cfg
                    .blocks
                    .iter()
                    .find(|b| line_num as u32 >= b.lines.0 && line_num as u32 <= b.lines.1);

                if let Some(block) = block {
                    // Intra-block precision: compute state at dereference point
                    let state_at_deref = compute_state_at_line(
                        block,
                        dfg,
                        state_in.get(&block.id).cloned().unwrap_or_default(),
                        source_lines,
                        line_num,
                        language,
                    );

                    let var_val = state_at_deref.get(&var);
                    if var_val.may_be_null() {
                        warnings.push((line_num, var));
                    }
                }
            }
        }
    }

    // Deduplicate warnings
    warnings.sort();
    warnings.dedup();

    warnings
}

/// Extract variables being dereferenced from a line of code.
///
/// Looks for patterns like:
/// - `x.foo` -> returns "x"
/// - `x.method()` -> returns "x"
/// - `x[idx]` -> returns "x"
/// - `obj.attr.nested` -> returns "obj"
fn extract_deref_patterns(code: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Skip non-identifier characters
        while i < len && !chars[i].is_alphabetic() && chars[i] != '_' {
            i += 1;
        }

        if i >= len {
            break;
        }

        // Collect identifier
        let start = i;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }

        let ident: String = chars[start..i].iter().collect();

        // Check if followed by . or [
        if i < len && (chars[i] == '.' || chars[i] == '[') {
            // This is a dereference pattern
            if !ident.is_empty() && !ident.chars().next().unwrap().is_ascii_digit() {
                // Skip keywords that look like dereferences
                let keywords = ["self", "this", "super", "cls"];
                if !keywords.contains(&ident.as_str()) {
                    patterns.push(ident);
                }
            }
        }
    }

    patterns
}

/// Compute abstract interpretation with widening for loop termination.
///
/// # Algorithm
///
/// 1. Initialize entry block with parameters as top()
/// 2. Initialize all other blocks as empty state (unreached)
/// 3. Iterate in reverse postorder until fixpoint:
///    - state_in[b] = join(state_out[p] for p in preds[b])
///    - Apply widening at loop headers (back-edge targets)
///    - state_out[b] = transfer(state_in[b], block[b])
/// 4. Return AbstractInterpInfo
///
/// # TIGER Mitigations
///
/// - TIGER-PASS1-7: Use blocks * 10 + 100 as iteration bound
/// - TIGER-PASS3-2: Both analyses take &DfgInfo (unified interface)
///
/// # Arguments
///
/// * `cfg` - Control flow graph
/// * `dfg` - Data flow graph with variable references
/// * `source_lines` - Optional source code lines for RHS parsing
/// * `language` - Language identifier (e.g., "python", "typescript", "go")
///
/// # Returns
///
/// AbstractInterpInfo containing:
/// - state_in: Abstract state at entry of each block
/// - state_out: Abstract state at exit of each block
/// - potential_div_zero: (line, var) pairs where division by zero is possible
/// - potential_null_deref: (line, var) pairs where null dereference is possible
///
/// # Errors
///
/// Returns DataflowError if:
/// - CFG is empty
/// - CFG exceeds MAX_BLOCKS
///
/// # Example
///
/// ```rust,ignore
/// let result = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), "python")?;
///
/// // Check for potential issues
/// for (line, var) in &result.potential_div_zero {
///     println!("Warning: potential div-by-zero at line {}: {}", line, var);
/// }
/// ```
pub fn compute_abstract_interp(
    cfg: &CfgInfo,
    dfg: &DfgInfo,
    source_lines: Option<&[&str]>,
    language: &str,
) -> Result<AbstractInterpInfo, DataflowError> {
    // Validate CFG
    validate_cfg(cfg)?;

    // Build helper structures
    let predecessors = build_predecessors(cfg);
    let loop_headers = find_back_edges(cfg);
    let block_order = reverse_postorder(cfg);

    // Initialize states
    let mut state_in: HashMap<BlockId, AbstractState> = HashMap::new();
    let mut state_out: HashMap<BlockId, AbstractState> = HashMap::new();

    let entry = cfg.entry_block;

    // Entry block starts with parameters as top
    let init_state = init_params(cfg, dfg);
    state_in.insert(entry, init_state.clone());

    // Process entry block to get initial state_out
    if let Some(entry_block) = cfg.blocks.iter().find(|b| b.id == entry) {
        let entry_out = transfer_block(&init_state, entry_block, dfg, source_lines, language);
        state_out.insert(entry, entry_out);
    } else {
        state_out.insert(entry, init_state);
    }

    // Initialize other blocks as empty (bottom/unreached)
    for block in &cfg.blocks {
        if block.id != entry {
            state_in.insert(block.id, AbstractState::default());
            state_out.insert(block.id, AbstractState::default());
        }
    }

    // TIGER-PASS1-7: Iteration bound
    let max_iterations = cfg.blocks.len() * 10 + 100;
    let mut iteration = 0;
    let mut changed = true;

    // Fixpoint iteration
    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;

        for &block_id in &block_order {
            // Skip entry block (already initialized)
            if block_id == entry {
                continue;
            }

            // Find the block
            let block = match cfg.blocks.iter().find(|b| b.id == block_id) {
                Some(b) => b,
                None => continue,
            };

            // Get predecessors
            let preds = predecessors.get(&block_id).cloned().unwrap_or_default();

            // Compute new state_in as join of all predecessor state_outs
            let mut new_in = if preds.is_empty() {
                AbstractState::default()
            } else {
                // Collect predecessor states
                let pred_states: Vec<&AbstractState> =
                    preds.iter().filter_map(|p| state_out.get(p)).collect();

                if pred_states.is_empty() {
                    AbstractState::default()
                } else {
                    join_states(&pred_states)
                }
            };

            // Apply widening at loop headers (CAP-AI-09)
            if loop_headers.contains(&block_id) {
                if let Some(old_in) = state_in.get(&block_id) {
                    new_in = widen_state(old_in, &new_in);
                }
            }

            // Apply transfer function
            let new_out = transfer_block(&new_in, block, dfg, source_lines, language);

            // Check for changes
            let old_in = state_in.get(&block_id);
            let old_out = state_out.get(&block_id);

            if old_in != Some(&new_in) || old_out != Some(&new_out) {
                changed = true;
                state_in.insert(block_id, new_in);
                state_out.insert(block_id, new_out);
            }
        }
    }

    // Phase 11: Detect potential safety issues
    let potential_div_zero = find_div_zero(cfg, dfg, &state_in, source_lines, &state_out, language);
    let potential_null_deref = find_null_deref(cfg, dfg, &state_in, source_lines, language);

    // Build result
    Ok(AbstractInterpInfo {
        state_in,
        state_out,
        potential_div_zero,
        potential_null_deref,
        function_name: cfg.function.clone(),
    })
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::f64::consts::PI;

    // =========================================================================
    // Nullability Tests (CAP-AI-01)
    // =========================================================================

    #[test]
    fn test_nullability_enum_has_three_values() {
        // CAP-AI-01: Nullability has exactly three values
        let _never = Nullability::Never;
        let _maybe = Nullability::Maybe;
        let _always = Nullability::Always;

        // Test string representation
        assert_eq!(Nullability::Never.as_str(), "never");
        assert_eq!(Nullability::Maybe.as_str(), "maybe");
        assert_eq!(Nullability::Always.as_str(), "always");
    }

    #[test]
    fn test_nullability_default_is_maybe() {
        // CAP-AI-01: Default is Maybe
        let default: Nullability = Default::default();
        assert_eq!(default, Nullability::Maybe);
    }

    // =========================================================================
    // AbstractValue Tests (CAP-AI-02 to CAP-AI-06)
    // =========================================================================

    #[test]
    fn test_abstract_value_has_required_fields() {
        // CAP-AI-02: AbstractValue has type_, range_, nullable, constant
        let value = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(1), Some(10))),
            nullable: Nullability::Never,
            constant: Some(ConstantValue::Int(5)),
        };

        assert_eq!(value.type_, Some("int".to_string()));
        assert_eq!(value.range_, Some((Some(1), Some(10))));
        assert_eq!(value.nullable, Nullability::Never);
        assert!(value.constant.is_some());
    }

    #[test]
    fn test_abstract_value_is_hashable() {
        // CAP-AI-02: AbstractValue must be hashable
        let value1 = AbstractValue::from_constant(ConstantValue::Int(5));
        let value2 = AbstractValue::from_constant(ConstantValue::Int(5));

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();
        value1.hash(&mut hasher1);
        value2.hash(&mut hasher2);

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_abstract_value_top_creates_unknown() {
        // CAP-AI-04: top() creates unknown value
        let top = AbstractValue::top();

        assert_eq!(top.type_, None);
        assert_eq!(top.range_, None);
        assert_eq!(top.nullable, Nullability::Maybe);
        assert!(top.constant.is_none());
    }

    #[test]
    fn test_abstract_value_bottom_creates_contradiction() {
        // CAP-AI-04: bottom() creates contradiction
        let bottom = AbstractValue::bottom();

        assert_eq!(bottom.type_, Some("<bottom>".to_string()));
        assert_eq!(bottom.range_, Some((None, None)));
        assert_eq!(bottom.nullable, Nullability::Never);
        assert!(bottom.constant.is_none());
    }

    #[test]
    fn test_abstract_value_from_constant_int() {
        // CAP-AI-03: from_constant for positive int
        let value = AbstractValue::from_constant(ConstantValue::Int(5));

        assert_eq!(value.type_, Some("int".to_string()));
        assert_eq!(value.range_, Some((Some(5), Some(5))));
        assert_eq!(value.nullable, Nullability::Never);
        assert_eq!(value.constant, Some(ConstantValue::Int(5)));
    }

    #[test]
    fn test_abstract_value_from_constant_negative_int() {
        // CAP-AI-03: from_constant for negative int
        let value = AbstractValue::from_constant(ConstantValue::Int(-42));

        assert_eq!(value.type_, Some("int".to_string()));
        assert_eq!(value.range_, Some((Some(-42), Some(-42))));
        assert_eq!(value.nullable, Nullability::Never);
        assert_eq!(value.constant, Some(ConstantValue::Int(-42)));
    }

    #[test]
    fn test_abstract_value_from_constant_string() {
        // CAP-AI-03: from_constant for string
        let value = AbstractValue::from_constant(ConstantValue::String("hello".to_string()));

        assert_eq!(value.type_, Some("str".to_string()));
        assert_eq!(value.nullable, Nullability::Never);
        assert!(value.constant.is_some());
    }

    #[test]
    fn test_abstract_value_string_tracks_length() {
        // CAP-AI-18: String tracks length in range
        let value = AbstractValue::from_constant(ConstantValue::String("hello".to_string()));

        // "hello" has length 5
        assert_eq!(value.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_abstract_value_from_constant_none() {
        // CAP-AI-03: from_constant for Null
        let value = AbstractValue::from_constant(ConstantValue::Null);

        assert_eq!(value.type_, Some("NoneType".to_string()));
        assert_eq!(value.range_, None);
        assert_eq!(value.nullable, Nullability::Always);
        assert!(value.constant.is_none()); // Null is represented by nullable=Always
    }

    #[test]
    fn test_abstract_value_from_constant_bool() {
        // CAP-AI-03: from_constant for bool
        let value_true = AbstractValue::from_constant(ConstantValue::Bool(true));
        let value_false = AbstractValue::from_constant(ConstantValue::Bool(false));

        assert_eq!(value_true.type_, Some("bool".to_string()));
        assert_eq!(value_true.range_, Some((Some(1), Some(1)))); // true as 1
        assert_eq!(value_false.range_, Some((Some(0), Some(0)))); // false as 0
    }

    #[test]
    fn test_abstract_value_from_constant_float() {
        // CAP-AI-03: from_constant for float
        let value = AbstractValue::from_constant(ConstantValue::Float(PI));

        assert_eq!(value.type_, Some("float".to_string()));
        assert_eq!(value.range_, None); // Float ranges not tracked
        assert_eq!(value.nullable, Nullability::Never);
    }

    // =========================================================================
    // may_be_zero Tests (CAP-AI-05)
    // =========================================================================

    #[test]
    fn test_may_be_zero_returns_true_when_range_includes_zero() {
        // CAP-AI-05: Range includes zero
        let value = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(-5), Some(5))),
            nullable: Nullability::Never,
            constant: None,
        };
        assert!(value.may_be_zero());

        // Exact zero
        let exact_zero = AbstractValue::from_constant(ConstantValue::Int(0));
        assert!(exact_zero.may_be_zero());
    }

    #[test]
    fn test_may_be_zero_returns_false_when_range_excludes_zero() {
        // CAP-AI-05: Range excludes zero
        let positive = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(1), Some(10))),
            nullable: Nullability::Never,
            constant: None,
        };
        assert!(!positive.may_be_zero());

        let negative = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(-10), Some(-1))),
            nullable: Nullability::Never,
            constant: None,
        };
        assert!(!negative.may_be_zero());
    }

    #[test]
    fn test_may_be_zero_returns_true_for_unknown_range() {
        // CAP-AI-05: Unknown range -> conservative true
        let top = AbstractValue::top();
        assert!(top.may_be_zero());
    }

    // =========================================================================
    // may_be_null Tests (CAP-AI-06)
    // =========================================================================

    #[test]
    fn test_may_be_null_for_maybe() {
        // CAP-AI-06: Maybe nullable -> true
        let value = AbstractValue {
            type_: None,
            range_: None,
            nullable: Nullability::Maybe,
            constant: None,
        };
        assert!(value.may_be_null());
    }

    #[test]
    fn test_may_be_null_for_never() {
        // CAP-AI-06: Never nullable -> false
        let value = AbstractValue::from_constant(ConstantValue::Int(5));
        assert!(!value.may_be_null());
    }

    #[test]
    fn test_may_be_null_for_always() {
        // CAP-AI-06: Always nullable -> true
        let value = AbstractValue::from_constant(ConstantValue::Null);
        assert!(value.may_be_null());
    }

    // =========================================================================
    // is_constant Tests
    // =========================================================================

    #[test]
    fn test_is_constant_true_when_constant_set() {
        let value = AbstractValue::from_constant(ConstantValue::Int(42));
        assert!(value.is_constant());
    }

    #[test]
    fn test_is_constant_false_when_constant_none() {
        let value = AbstractValue::top();
        assert!(!value.is_constant());
    }

    // =========================================================================
    // AbstractState Tests (CAP-AI-07)
    // =========================================================================

    #[test]
    fn test_abstract_state_empty_initialization() {
        let state = AbstractState::new();
        assert!(state.values.is_empty());
    }

    #[test]
    fn test_abstract_state_get_returns_value_for_existing_var() {
        let mut state = AbstractState::new();
        let value = AbstractValue::from_constant(ConstantValue::Int(5));
        state.values.insert("x".to_string(), value.clone());

        let retrieved = state.get("x");
        assert_eq!(retrieved.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_abstract_state_get_returns_top_for_missing_var() {
        // CAP-AI-07: Missing vars default to top
        let state = AbstractState::new();
        let value = state.get("nonexistent");

        assert_eq!(value.type_, None);
        assert_eq!(value.range_, None);
        assert_eq!(value.nullable, Nullability::Maybe);
    }

    #[test]
    fn test_abstract_state_set_returns_new_state() {
        // Immutable update pattern
        let state1 = AbstractState::new();
        let state2 = state1.set("x", AbstractValue::from_constant(ConstantValue::Int(5)));

        // Original unchanged
        assert!(state1.values.is_empty());
        // New state has the value
        assert!(state2.values.contains_key("x"));
    }

    #[test]
    fn test_abstract_state_copy_creates_independent_copy() {
        let mut state1 = AbstractState::new();
        state1.values.insert(
            "x".to_string(),
            AbstractValue::from_constant(ConstantValue::Int(5)),
        );

        let state2 = state1.copy();

        // Modify original
        state1.values.insert(
            "y".to_string(),
            AbstractValue::from_constant(ConstantValue::Int(10)),
        );

        // Copy should not have y
        assert!(state2.values.contains_key("x"));
        assert!(!state2.values.contains_key("y"));
    }

    #[test]
    fn test_abstract_state_equality() {
        let state1 =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
        let state2 =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
        let state3 =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(10)));

        assert_eq!(state1, state2);
        assert_ne!(state1, state3);
    }

    // =========================================================================
    // AbstractInterpInfo Tests (CAP-AI-21, CAP-AI-22)
    // =========================================================================

    #[test]
    fn test_abstract_interp_info_has_required_fields() {
        let info = AbstractInterpInfo::new("test_func");

        assert_eq!(info.function_name, "test_func");
        assert!(info.state_in.is_empty());
        assert!(info.state_out.is_empty());
        assert!(info.potential_div_zero.is_empty());
        assert!(info.potential_null_deref.is_empty());
    }

    #[test]
    fn test_value_at_returns_abstract_value_at_block_entry() {
        let mut info = AbstractInterpInfo::new("test");
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(42)));
        info.state_in.insert(0, state);

        let value = info.value_at(0, "x");
        assert_eq!(value.range_, Some((Some(42), Some(42))));
    }

    #[test]
    fn test_value_at_returns_top_for_missing_block() {
        let info = AbstractInterpInfo::new("test");
        let value = info.value_at(999, "x");

        // Should return top() for missing block
        assert_eq!(value.type_, None);
        assert_eq!(value.range_, None);
    }

    #[test]
    fn test_value_at_exit_returns_value_at_block_exit() {
        let mut info = AbstractInterpInfo::new("test");
        let state =
            AbstractState::new().set("y", AbstractValue::from_constant(ConstantValue::Int(100)));
        info.state_out.insert(1, state);

        let value = info.value_at_exit(1, "y");
        assert_eq!(value.range_, Some((Some(100), Some(100))));
    }

    #[test]
    fn test_range_at_returns_range_tuple() {
        let mut info = AbstractInterpInfo::new("test");
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
        info.state_in.insert(0, state);

        let range = info.range_at(0, "x");
        assert_eq!(range, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_type_at_returns_inferred_type() {
        let mut info = AbstractInterpInfo::new("test");
        let state = AbstractState::new().set(
            "x",
            AbstractValue::from_constant(ConstantValue::String("hello".to_string())),
        );
        info.state_in.insert(0, state);

        let type_ = info.type_at(0, "x");
        assert_eq!(type_, Some("str".to_string()));
    }

    #[test]
    fn test_is_definitely_not_null_for_never_nullable() {
        let mut info = AbstractInterpInfo::new("test");
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
        info.state_in.insert(0, state);

        assert!(info.is_definitely_not_null(0, "x"));
    }

    #[test]
    fn test_is_definitely_not_null_for_maybe_nullable() {
        let mut info = AbstractInterpInfo::new("test");
        let state = AbstractState::new().set("x", AbstractValue::top());
        info.state_in.insert(0, state);

        assert!(!info.is_definitely_not_null(0, "x"));
    }

    #[test]
    fn test_get_constants_returns_known_constant_values() {
        let mut info = AbstractInterpInfo::new("test");
        let state = AbstractState::new()
            .set("x", AbstractValue::from_constant(ConstantValue::Int(5)))
            .set(
                "y",
                AbstractValue::from_constant(ConstantValue::String("hello".to_string())),
            )
            .set("z", AbstractValue::top()); // Not a constant
        info.state_out.insert(0, state);

        let constants = info.get_constants();
        assert_eq!(constants.len(), 2);
        assert!(constants.contains_key("x"));
        assert!(constants.contains_key("y"));
        assert!(!constants.contains_key("z"));
    }

    #[test]
    fn test_abstract_interp_to_json_serializable() {
        let mut info = AbstractInterpInfo::new("example");
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(42)));
        info.state_in.insert(0, state.clone());
        info.state_out.insert(0, state);
        info.potential_div_zero.push((10, "y".to_string()));
        info.potential_null_deref.push((15, "obj".to_string()));

        let json = info.to_json();

        // Verify it's valid JSON
        assert!(json.is_object());
        assert_eq!(json["function"], "example");
        assert!(json["state_in"].is_object());
        assert!(json["state_out"].is_object());
        assert!(json["potential_div_zero"].is_array());
        assert!(json["potential_null_deref"].is_array());

        // Verify serialization works
        let serialized = serde_json::to_string(&json);
        assert!(serialized.is_ok());
    }

    // =========================================================================
    // Phase 8: Multi-Language Keyword Tests (CAP-AI-15, CAP-AI-16, CAP-AI-17)
    // =========================================================================

    #[test]
    fn test_python_none_keyword_recognized() {
        // CAP-AI-15: Python None keyword
        let keywords = get_null_keywords("python");
        assert!(keywords.contains(&"None"));
    }

    #[test]
    fn test_typescript_null_keyword_recognized() {
        // CAP-AI-15: TypeScript null keyword
        let keywords = get_null_keywords("typescript");
        assert!(keywords.contains(&"null"));
    }

    #[test]
    fn test_typescript_undefined_keyword_recognized() {
        // CAP-AI-15: TypeScript undefined keyword
        let keywords = get_null_keywords("typescript");
        assert!(keywords.contains(&"undefined"));
    }

    #[test]
    fn test_go_nil_keyword_recognized() {
        // CAP-AI-15: Go nil keyword
        let keywords = get_null_keywords("go");
        assert!(keywords.contains(&"nil"));
    }

    #[test]
    fn test_rust_has_no_null_keyword() {
        // CAP-AI-15: Rust has no null (None is Option::None, not null)
        let keywords = get_null_keywords("rust");
        assert!(keywords.is_empty(), "Rust should have no null keywords");
    }

    #[test]
    fn test_python_boolean_capitalized() {
        // CAP-AI-16: Python uses True/False (capitalized)
        let bools = get_boolean_keywords("python");
        assert_eq!(bools.get("True"), Some(&true));
        assert_eq!(bools.get("False"), Some(&false));
    }

    #[test]
    fn test_typescript_boolean_lowercase() {
        // CAP-AI-16: TypeScript uses true/false (lowercase)
        let bools = get_boolean_keywords("typescript");
        assert_eq!(bools.get("true"), Some(&true));
        assert_eq!(bools.get("false"), Some(&false));
    }

    #[test]
    fn test_python_comment_pattern() {
        // CAP-AI-17: Python uses # for comments
        let pattern = get_comment_pattern("python");
        assert_eq!(pattern, "#");
    }

    #[test]
    fn test_typescript_comment_pattern() {
        // CAP-AI-17: TypeScript uses // for comments
        let pattern = get_comment_pattern("typescript");
        assert_eq!(pattern, "//");
    }

    // =========================================================================
    // Arithmetic Tests (CAP-AI-13) - Phase 7
    // =========================================================================

    #[test]
    fn test_arithmetic_add() {
        // CAP-AI-13: Abstract arithmetic - addition
        // [5, 5] + 3 -> [8, 8]
        let operand = AbstractValue::from_constant(ConstantValue::Int(5));
        let result = apply_arithmetic(&operand, '+', 3);

        assert_eq!(result.range_, Some((Some(8), Some(8))));
        assert_eq!(result.constant, Some(ConstantValue::Int(8)));
    }

    #[test]
    fn test_arithmetic_subtract() {
        // CAP-AI-13: Abstract arithmetic - subtraction
        // [10, 10] - 3 -> [7, 7]
        let operand = AbstractValue::from_constant(ConstantValue::Int(10));
        let result = apply_arithmetic(&operand, '-', 3);

        assert_eq!(result.range_, Some((Some(7), Some(7))));
        assert_eq!(result.constant, Some(ConstantValue::Int(7)));
    }

    #[test]
    fn test_arithmetic_multiply() {
        // CAP-AI-13: Abstract arithmetic - multiplication
        // [4, 4] * 2 -> [8, 8]
        let operand = AbstractValue::from_constant(ConstantValue::Int(4));
        let result = apply_arithmetic(&operand, '*', 2);

        assert_eq!(result.range_, Some((Some(8), Some(8))));
        assert_eq!(result.constant, Some(ConstantValue::Int(8)));
    }

    #[test]
    fn test_arithmetic_on_range() {
        // CAP-AI-13: Arithmetic on a range
        // [1, 5] + 10 -> [11, 15]
        let operand = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(1), Some(5))),
            nullable: Nullability::Never,
            constant: None,
        };

        let result = apply_arithmetic(&operand, '+', 10);

        assert_eq!(result.range_, Some((Some(11), Some(15))));
        // Not a constant because range is not a single value
        assert!(result.constant.is_none());
    }

    #[test]
    fn test_arithmetic_overflow_saturates_add() {
        // TIGER-PASS1-11: Overflow should widen to unbounded (None)
        // i64::MAX + 1 -> widened to unbounded
        let operand = AbstractValue::from_constant(ConstantValue::Int(i64::MAX));
        let result = apply_arithmetic(&operand, '+', 1);

        // Range should contain at least one None (widened due to overflow)
        match result.range_ {
            Some((low, high)) => {
                // Either low or high should be None due to saturation
                assert!(
                    low.is_none() || high.is_none(),
                    "Overflow should widen to unbounded: got ({:?}, {:?})",
                    low,
                    high
                );
            }
            None => {
                // No range at all is also acceptable
            }
        }
    }

    #[test]
    fn test_arithmetic_overflow_saturates_sub() {
        // TIGER-PASS1-11: Underflow should widen to unbounded
        // i64::MIN - 1 -> widened to unbounded
        let operand = AbstractValue::from_constant(ConstantValue::Int(i64::MIN));
        let result = apply_arithmetic(&operand, '-', 1);

        // Range should contain at least one None (widened due to overflow)
        if let Some((low, high)) = result.range_ {
            assert!(
                low.is_none() || high.is_none(),
                "Underflow should widen to unbounded: got ({:?}, {:?})",
                low,
                high
            );
        }
    }

    #[test]
    fn test_arithmetic_multiply_by_negative() {
        // Multiplication by negative swaps bounds
        // [2, 4] * (-3) -> [-12, -6]
        let operand = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(2), Some(4))),
            nullable: Nullability::Never,
            constant: None,
        };

        let result = apply_arithmetic(&operand, '*', -3);

        assert_eq!(result.range_, Some((Some(-12), Some(-6))));
    }

    #[test]
    fn test_arithmetic_multiply_by_zero() {
        // Multiplication by zero gives [0, 0]
        let operand = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(1), Some(100))),
            nullable: Nullability::Never,
            constant: None,
        };

        let result = apply_arithmetic(&operand, '*', 0);

        assert_eq!(result.range_, Some((Some(0), Some(0))));
    }

    #[test]
    fn test_arithmetic_preserves_type() {
        // Arithmetic should preserve the type
        let operand = AbstractValue::from_constant(ConstantValue::Int(5));
        let result = apply_arithmetic(&operand, '+', 3);

        assert_eq!(result.type_, Some("int".to_string()));
    }

    #[test]
    fn test_arithmetic_preserves_nullable() {
        // Arithmetic should preserve nullability
        let operand = AbstractValue::from_constant(ConstantValue::Int(5));
        assert_eq!(operand.nullable, Nullability::Never);

        let result = apply_arithmetic(&operand, '+', 3);
        assert_eq!(result.nullable, Nullability::Never);
    }

    #[test]
    fn test_arithmetic_unknown_op() {
        // Unknown operator should widen to unbounded
        let operand = AbstractValue::from_constant(ConstantValue::Int(5));
        let result = apply_arithmetic(&operand, '^', 3); // ^ not supported

        // Should widen to unbounded
        assert_eq!(result.range_, Some((None, None)));
        assert!(result.constant.is_none());
    }

    #[test]
    fn test_arithmetic_on_no_range() {
        // Arithmetic on value with no range returns no range
        let operand = AbstractValue::top();
        let result = apply_arithmetic(&operand, '+', 5);

        assert!(result.range_.is_none());
    }

    // =========================================================================
    // Phase 6 Tests: Join and Widening (CAP-AI-08, CAP-AI-09)
    // =========================================================================

    #[test]
    fn test_join_values_ranges_union() {
        // CAP-AI-08: Join takes union of ranges [1,1] join [10,10] -> [1,10]
        let val1 = AbstractValue::from_constant(ConstantValue::Int(1));
        let val2 = AbstractValue::from_constant(ConstantValue::Int(10));

        let joined = join_values(&val1, &val2);

        // Range should be union: [1, 10]
        assert_eq!(joined.range_, Some((Some(1), Some(10))));
    }

    #[test]
    fn test_join_values_loses_constant_on_disagreement() {
        // CAP-AI-08: Constant lost when values disagree
        let val1 = AbstractValue::from_constant(ConstantValue::Int(1));
        let val2 = AbstractValue::from_constant(ConstantValue::Int(10));

        let joined = join_values(&val1, &val2);

        assert!(
            joined.constant.is_none(),
            "Constant should be lost on disagreement"
        );
    }

    #[test]
    fn test_join_values_preserves_constant_on_agreement() {
        // CAP-AI-08: Constant kept when values agree
        let val1 = AbstractValue::from_constant(ConstantValue::Int(5));
        let val2 = AbstractValue::from_constant(ConstantValue::Int(5));

        let joined = join_values(&val1, &val2);

        assert_eq!(joined.constant, Some(ConstantValue::Int(5)));
    }

    #[test]
    fn test_join_values_nullable_maybe_if_any_maybe() {
        // CAP-AI-08: Nullable becomes MAYBE if either is MAYBE
        let val1 = AbstractValue {
            type_: None,
            range_: None,
            nullable: Nullability::Never,
            constant: None,
        };
        let val2 = AbstractValue {
            type_: None,
            range_: None,
            nullable: Nullability::Maybe,
            constant: None,
        };

        let joined = join_values(&val1, &val2);

        assert_eq!(joined.nullable, Nullability::Maybe);
    }

    #[test]
    fn test_join_values_nullable_never_if_both_never() {
        // CAP-AI-08: NEVER + NEVER = NEVER
        let val1 = AbstractValue::from_constant(ConstantValue::Int(1));
        let val2 = AbstractValue::from_constant(ConstantValue::Int(2));

        let joined = join_values(&val1, &val2);

        assert_eq!(joined.nullable, Nullability::Never);
    }

    #[test]
    fn test_join_values_type_preserved_when_same() {
        // CAP-AI-08: Type preserved when both values have same type
        let val1 = AbstractValue::from_constant(ConstantValue::Int(1));
        let val2 = AbstractValue::from_constant(ConstantValue::Int(2));

        let joined = join_values(&val1, &val2);

        assert_eq!(joined.type_, Some("int".to_string()));
    }

    #[test]
    fn test_join_values_type_lost_when_different() {
        // CAP-AI-08: Type lost when values have different types
        let val1 = AbstractValue::from_constant(ConstantValue::Int(1));
        let val2 = AbstractValue::from_constant(ConstantValue::String("hello".to_string()));

        let joined = join_values(&val1, &val2);

        assert_eq!(joined.type_, None);
    }

    #[test]
    fn test_join_states_empty() {
        // CAP-AI-08: Join of empty states is empty
        let states: Vec<&AbstractState> = vec![];
        let joined = join_states(&states);

        assert!(joined.values.is_empty());
    }

    #[test]
    fn test_join_states_single() {
        // CAP-AI-08: Join of single state returns that state
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(5)));
        let states: Vec<&AbstractState> = vec![&state];

        let joined = join_states(&states);

        assert_eq!(joined.get("x").range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_join_states_multiple() {
        // CAP-AI-08: Join of multiple states combines variables
        let state1 =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(1)));
        let state2 =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(10)));
        let states: Vec<&AbstractState> = vec![&state1, &state2];

        let joined = join_states(&states);

        // x should have range [1, 10]
        assert_eq!(joined.get("x").range_, Some((Some(1), Some(10))));
    }

    #[test]
    fn test_widen_value_upper_bound_to_infinity() {
        // CAP-AI-09: Growing upper bound -> widen to +inf (None)
        let old = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(0), Some(5))),
            nullable: Nullability::Never,
            constant: None,
        };
        let new = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(0), Some(10))), // Upper bound grew
            nullable: Nullability::Never,
            constant: None,
        };

        let widened = widen_value(&old, &new);

        // Upper bound should be widened to +inf (None)
        assert_eq!(widened.range_, Some((Some(0), None)));
    }

    #[test]
    fn test_widen_value_lower_bound_to_infinity() {
        // CAP-AI-09: Growing lower bound -> widen to -inf (None)
        let old = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(-5), Some(10))),
            nullable: Nullability::Never,
            constant: None,
        };
        let new = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(-10), Some(10))), // Lower bound grew (more negative)
            nullable: Nullability::Never,
            constant: None,
        };

        let widened = widen_value(&old, &new);

        // Lower bound should be widened to -inf (None)
        assert_eq!(widened.range_, Some((None, Some(10))));
    }

    #[test]
    fn test_widen_value_loses_constant() {
        // CAP-AI-09: Widening loses constant information
        let old = AbstractValue::from_constant(ConstantValue::Int(5));
        let new = AbstractValue::from_constant(ConstantValue::Int(6));

        let widened = widen_value(&old, &new);

        assert!(widened.constant.is_none(), "Widening should lose constant");
    }

    #[test]
    fn test_widen_value_stable_bounds_not_widened() {
        // CAP-AI-09: Stable or shrinking bounds are not widened
        let old = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(0), Some(10))),
            nullable: Nullability::Never,
            constant: None,
        };
        let new = AbstractValue {
            type_: Some("int".to_string()),
            range_: Some((Some(0), Some(10))), // Same bounds
            nullable: Nullability::Never,
            constant: None,
        };

        let widened = widen_value(&old, &new);

        // Bounds should remain the same
        assert_eq!(widened.range_, Some((Some(0), Some(10))));
    }

    #[test]
    fn test_widen_state_applies_to_all_vars() {
        // CAP-AI-09: Widening applies to all variables in both states
        let old = AbstractState::new()
            .set("x", AbstractValue::from_constant(ConstantValue::Int(5)))
            .set("y", AbstractValue::from_constant(ConstantValue::Int(0)));
        let new = AbstractState::new()
            .set("x", AbstractValue::from_constant(ConstantValue::Int(10)))
            .set("y", AbstractValue::from_constant(ConstantValue::Int(0)));

        let widened = widen_state(&old, &new);

        // x: old=[5,5], new=[10,10]
        // Lower bound went from 5 to 10 (grew upward, not downward) -> keep new value (10)
        // Upper bound went from 5 to 10 (grew upward) -> widen to +inf (None)
        // Result: [10, None]
        assert_eq!(widened.get("x").range_, Some((Some(10), None)));
        // y should be unchanged (same bounds)
        assert_eq!(widened.get("y").range_, Some((Some(0), Some(0))));
    }

    // =========================================================================
    // Phase 9 Tests: RHS Parsing (CAP-AI-14)
    // =========================================================================

    #[test]
    fn test_extract_rhs_simple_assignment() {
        // CAP-AI-14: Extract RHS from simple assignment
        let rhs = extract_rhs("x = a + b", "x");
        assert_eq!(rhs, Some("a + b".to_string()));

        let rhs = extract_rhs("foo = 42", "foo");
        assert_eq!(rhs, Some("42".to_string()));

        let rhs = extract_rhs("result = None", "result");
        assert_eq!(rhs, Some("None".to_string()));
    }

    #[test]
    fn test_extract_rhs_augmented_assignment() {
        // TIGER-PASS2-2: Augmented assignments converted to regular form
        let rhs = extract_rhs("x += 5", "x");
        assert_eq!(rhs, Some("x + 5".to_string()));

        let rhs = extract_rhs("y -= 3", "y");
        assert_eq!(rhs, Some("y - 3".to_string()));

        let rhs = extract_rhs("count *= 2", "count");
        assert_eq!(rhs, Some("count * 2".to_string()));
    }

    #[test]
    fn test_extract_rhs_with_spaces() {
        // Various spacing patterns
        let rhs = extract_rhs("x=5", "x");
        assert_eq!(rhs, Some("5".to_string()));

        let rhs = extract_rhs("x =5", "x");
        assert_eq!(rhs, Some("5".to_string()));

        let rhs = extract_rhs("x= 5", "x");
        assert_eq!(rhs, Some("5".to_string()));
    }

    #[test]
    fn test_extract_rhs_not_found() {
        // No assignment to this variable
        let rhs = extract_rhs("y = 5", "x");
        assert_eq!(rhs, None);

        // Partial match should not match
        let rhs = extract_rhs("xy = 5", "x");
        assert_eq!(rhs, None);
    }

    #[test]
    fn test_strip_comment_python() {
        // TIGER-PASS1-14: Single-line comments stripped
        let stripped = strip_comment("x = 5  # this is a comment", "python");
        assert_eq!(stripped, "x = 5  ");

        let stripped = strip_comment("x = 5", "python");
        assert_eq!(stripped, "x = 5");
    }

    #[test]
    fn test_strip_comment_typescript() {
        let stripped = strip_comment("x = 5  // this is a comment", "typescript");
        assert_eq!(stripped, "x = 5  ");

        let stripped = strip_comment("x = 5", "typescript");
        assert_eq!(stripped, "x = 5");
    }

    #[test]
    fn test_strip_comment_preserves_string() {
        // Comment marker inside string should not be stripped
        let stripped = strip_comment("x = \"hello # world\"", "python");
        assert_eq!(stripped, "x = \"hello # world\"");

        let stripped = strip_comment("x = 'hello // world'", "typescript");
        assert_eq!(stripped, "x = 'hello // world'");
    }

    #[test]
    fn test_strip_strings_blanks_path_separators() {
        // Path inside string: slashes should be blanked
        let result = strip_strings("Path::new(\"src/main.rs\")", "rust");
        assert_eq!(result, "Path::new(\"           \")");
        assert!(
            !result.contains('/'),
            "slashes inside strings must be blanked"
        );
    }

    #[test]
    fn test_strip_strings_preserves_code() {
        // Division operator outside strings should be preserved
        let result = strip_strings("let ratio = a / b;", "rust");
        assert_eq!(result, "let ratio = a / b;");
    }

    #[test]
    fn test_strip_strings_handles_escapes() {
        // Escaped quote inside string should not end the string
        let result = strip_strings(r#"let s = "path/to/\"file\""; a / b"#, "rust");
        assert!(result.contains("a / b"), "code division must survive");
        // The path/to part inside the string should be blanked
        assert!(
            !result[8..25].contains('/'),
            "slashes in string must be blanked"
        );
    }

    #[test]
    fn test_strip_strings_single_quotes() {
        let result = strip_strings("let c = '/'; x / y", "rust");
        assert!(result.contains("x / y"), "code division must survive");
        // The '/' char literal should be blanked
        assert_eq!(result.matches('/').count(), 1, "only code division remains");
    }

    #[test]
    fn test_strip_strings_rust_raw_string() {
        // r#"..."# raw strings: contents must be blanked
        let result = strip_strings(r##"let xml = r#"</coverage>"#;"##, "rust");
        assert!(
            !result.contains('/'),
            "slashes inside raw strings must be blanked"
        );
        assert!(
            !result.contains("coverage"),
            "identifiers inside raw strings must be blanked"
        );
    }

    #[test]
    fn test_strip_strings_rust_raw_no_hashes() {
        // r"..." raw strings without hashes
        let result = strip_strings(r#"let p = r"/src/main.rs"; a / b"#, "rust");
        assert!(result.contains("a / b"), "code division must survive");
        // Only the code `/` should remain
        assert_eq!(result.matches('/').count(), 1, "only code division remains");
    }

    #[test]
    fn test_strip_strings_rust_raw_double_hash() {
        // r##"..."## raw strings
        let result = strip_strings(r###"let s = r##"a/b"##;"###, "rust");
        assert!(
            !result.contains("a/b"),
            "contents of r##\"...\"## must be blanked"
        );
    }

    #[test]
    fn test_parse_simple_arithmetic_var_plus_const() {
        // CAP-AI-13: Variable + constant
        let result = parse_simple_arithmetic("a + 1");
        assert_eq!(result, Some(("a".to_string(), '+', 1)));

        let result = parse_simple_arithmetic("count - 5");
        assert_eq!(result, Some(("count".to_string(), '-', 5)));

        let result = parse_simple_arithmetic("x * 2");
        assert_eq!(result, Some(("x".to_string(), '*', 2)));
    }

    #[test]
    fn test_parse_simple_arithmetic_const_plus_var() {
        // Commutative: const + var (only for + and *)
        let result = parse_simple_arithmetic("1 + a");
        assert_eq!(result, Some(("a".to_string(), '+', 1)));

        let result = parse_simple_arithmetic("2 * x");
        assert_eq!(result, Some(("x".to_string(), '*', 2)));
    }

    #[test]
    fn test_parse_simple_arithmetic_negative_const() {
        // Negative constants
        let result = parse_simple_arithmetic("a + -5");
        assert_eq!(result, Some(("a".to_string(), '+', -5)));
    }

    #[test]
    fn test_parse_simple_arithmetic_no_match() {
        // Complex expressions don't match
        let result = parse_simple_arithmetic("a + b"); // Two variables
        assert_eq!(result, None);

        let result = parse_simple_arithmetic("5"); // Just a constant
        assert_eq!(result, None);

        let result = parse_simple_arithmetic("foo"); // Just a variable
        assert_eq!(result, None);
    }

    #[test]
    fn test_is_identifier() {
        assert!(is_identifier("x"));
        assert!(is_identifier("foo"));
        assert!(is_identifier("_bar"));
        assert!(is_identifier("var123"));
        assert!(is_identifier("__init__"));

        assert!(!is_identifier(""));
        assert!(!is_identifier("123var"));
        assert!(!is_identifier("foo.bar"));
        assert!(!is_identifier("foo bar"));
        assert!(!is_identifier("foo-bar"));
    }

    #[test]
    fn test_parse_rhs_abstract_integer() {
        // CAP-AI-14: Integer literal
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = 5", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(5), Some(5))));
        assert_eq!(val.constant, Some(ConstantValue::Int(5)));
        assert_eq!(val.type_, Some("int".to_string()));
    }

    #[test]
    fn test_parse_rhs_abstract_negative_integer() {
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = -42", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(-42), Some(-42))));
        assert_eq!(val.constant, Some(ConstantValue::Int(-42)));
    }

    #[test]
    fn test_parse_rhs_abstract_float() {
        // CAP-AI-14: Float literal
        // Use PI to avoid clippy::approx_constant; tests that arbitrary float literals
        // round-trip correctly through parse_rhs_abstract.
        let pi_literal = format!("x = {}", std::f64::consts::PI);
        let state = AbstractState::new();
        let val = parse_rhs_abstract(&pi_literal, "x", &state, "python");

        assert_eq!(val.type_, Some("float".to_string()));
        if let Some(ConstantValue::Float(f)) = val.constant {
            assert_eq!(f, std::f64::consts::PI);
        } else {
            panic!("Expected float constant");
        }
    }

    #[test]
    fn test_parse_rhs_abstract_string_double_quotes() {
        // CAP-AI-14: String literal with double quotes
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = \"hello\"", "x", &state, "python");

        assert_eq!(val.type_, Some("str".to_string()));
        assert_eq!(
            val.constant,
            Some(ConstantValue::String("hello".to_string()))
        );
        // CAP-AI-18: String length tracked
        assert_eq!(val.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_parse_rhs_abstract_string_single_quotes() {
        // CAP-AI-14: String literal with single quotes
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = 'world'", "x", &state, "python");

        assert_eq!(val.type_, Some("str".to_string()));
        assert_eq!(
            val.constant,
            Some(ConstantValue::String("world".to_string()))
        );
    }

    #[test]
    fn test_parse_rhs_abstract_python_none() {
        // CAP-AI-15: Python None
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = None", "x", &state, "python");

        assert_eq!(val.nullable, Nullability::Always);
        assert_eq!(val.type_, Some("NoneType".to_string()));
    }

    #[test]
    fn test_parse_rhs_abstract_typescript_null() {
        // CAP-AI-15: TypeScript null
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = null", "x", &state, "typescript");

        assert_eq!(val.nullable, Nullability::Always);
    }

    #[test]
    fn test_parse_rhs_abstract_typescript_undefined() {
        // TIGER-PASS2-8: TypeScript undefined tracked separately
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = undefined", "x", &state, "typescript");

        assert_eq!(val.nullable, Nullability::Always);
        assert_eq!(val.type_, Some("undefined".to_string()));
    }

    #[test]
    fn test_parse_rhs_abstract_go_nil() {
        // CAP-AI-15: Go nil
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = nil", "x", &state, "go");

        assert_eq!(val.nullable, Nullability::Always);
    }

    #[test]
    fn test_parse_rhs_abstract_python_bool() {
        // CAP-AI-16: Python True/False (capitalized)
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = True", "x", &state, "python");

        assert_eq!(val.type_, Some("bool".to_string()));
        assert_eq!(val.constant, Some(ConstantValue::Bool(true)));

        let val = parse_rhs_abstract("y = False", "y", &state, "python");
        assert_eq!(val.constant, Some(ConstantValue::Bool(false)));
    }

    #[test]
    fn test_parse_rhs_abstract_typescript_bool() {
        // CAP-AI-16: TypeScript true/false (lowercase)
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = true", "x", &state, "typescript");

        assert_eq!(val.type_, Some("bool".to_string()));
        assert_eq!(val.constant, Some(ConstantValue::Bool(true)));
    }

    #[test]
    fn test_parse_rhs_abstract_variable_copy() {
        // CAP-AI-19: Variable copy (y = x copies value)
        let state =
            AbstractState::new().set("a", AbstractValue::from_constant(ConstantValue::Int(42)));

        let val = parse_rhs_abstract("x = a", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(42), Some(42))));
        assert_eq!(val.constant, Some(ConstantValue::Int(42)));
    }

    #[test]
    fn test_parse_rhs_abstract_simple_arithmetic() {
        // CAP-AI-13: Simple arithmetic x = a + 1
        let state =
            AbstractState::new().set("a", AbstractValue::from_constant(ConstantValue::Int(5)));

        let val = parse_rhs_abstract("x = a + 3", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(8), Some(8))));
        assert_eq!(val.constant, Some(ConstantValue::Int(8)));
    }

    #[test]
    fn test_parse_rhs_abstract_augmented_assignment() {
        // TIGER-PASS2-2: x += 1 treated as x = x + 1
        let state =
            AbstractState::new().set("x", AbstractValue::from_constant(ConstantValue::Int(10)));

        let val = parse_rhs_abstract("x += 5", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(15), Some(15))));
        assert_eq!(val.constant, Some(ConstantValue::Int(15)));
    }

    #[test]
    fn test_parse_rhs_abstract_with_comment() {
        // TIGER-PASS1-14: Comments stripped before parsing
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = 5  # this is the value", "x", &state, "python");

        assert_eq!(val.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_parse_rhs_abstract_unknown_returns_top() {
        // Unknown RHS returns top
        let state = AbstractState::new();
        let val = parse_rhs_abstract("x = foo(a, b)", "x", &state, "python");

        // Should be top (unknown)
        assert_eq!(val.type_, None);
        assert_eq!(val.range_, None);
        assert_eq!(val.nullable, Nullability::Maybe);
    }

    #[test]
    fn test_parse_rhs_abstract_no_assignment() {
        // Line doesn't contain assignment to this var
        let state = AbstractState::new();
        let val = parse_rhs_abstract("y = 5", "x", &state, "python");

        // Should be top (unknown)
        assert_eq!(val.type_, None);
        assert_eq!(val.range_, None);
    }

    // =========================================================================
    // Phase 10 Tests: compute_abstract_interp Main Algorithm
    // =========================================================================

    use crate::types::{BlockType, CfgBlock, CfgEdge, CfgInfo, DfgInfo, EdgeType, VarRef};

    /// Helper to create a minimal CFG for testing
    fn make_test_cfg(function: &str, blocks: Vec<CfgBlock>, edges: Vec<CfgEdge>) -> CfgInfo {
        CfgInfo {
            function: function.to_string(),
            blocks,
            edges,
            entry_block: 0,
            exit_blocks: vec![0], // Simple case
            cyclomatic_complexity: 1,
            nested_functions: HashMap::new(),
        }
    }

    /// Helper to create a VarRef
    fn make_var_ref(name: &str, ref_type: RefType, line: u32, column: u32) -> VarRef {
        VarRef {
            name: name.to_string(),
            ref_type,
            line,
            column,
            context: None,
            group_id: None,
        }
    }

    #[test]
    fn test_compute_abstract_interp_returns_info() {
        // Basic: compute_abstract_interp returns AbstractInterpInfo
        let cfg = make_test_cfg(
            "test_func",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "test_func".to_string(),
            refs: vec![],
            edges: vec![],
            variables: vec![],
        };

        let result = compute_abstract_interp(&cfg, &dfg, None, "python").unwrap();
        assert_eq!(result.function_name, "test_func");
    }

    #[test]
    fn test_compute_tracks_constant_assignment() {
        // x = 5 should result in x having range [5, 5]
        let cfg = make_test_cfg(
            "const_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "const_test".to_string(),
            refs: vec![make_var_ref("x", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = ["x = 5"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();
        let val = result.value_at_exit(0, "x");
        assert_eq!(val.range_, Some((Some(5), Some(5))));
    }

    #[test]
    fn test_compute_tracks_variable_copy() {
        // CAP-AI-19: y = x copies abstract value
        // x = 5
        // y = x  -> y should have same abstract value as x
        let cfg = make_test_cfg(
            "copy_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "copy_test".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 4),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 5", "y = x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();
        let val_x = result.value_at_exit(0, "x");
        let val_y = result.value_at_exit(0, "y");
        assert_eq!(val_x.range_, val_y.range_);
    }

    #[test]
    fn test_compute_tracks_none_assignment() {
        // x = None should result in x being ALWAYS nullable
        let cfg = make_test_cfg(
            "none_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "none_test".to_string(),
            refs: vec![make_var_ref("x", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = ["x = None"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();
        let val = result.value_at_exit(0, "x");
        assert_eq!(val.nullable, Nullability::Always);
    }

    #[test]
    fn test_abstract_interp_empty_function_no_crash() {
        // Empty function should not crash
        let cfg = make_test_cfg(
            "empty_func",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "empty_func".to_string(),
            refs: vec![],
            edges: vec![],
            variables: vec![],
        };

        let result = compute_abstract_interp(&cfg, &dfg, None, "python");
        assert!(result.is_ok());
    }

    #[test]
    fn test_unknown_rhs_defaults_to_top() {
        // Unknown RHS (e.g., function call) defaults to top()
        // x = some_unknown_function()
        let cfg = make_test_cfg(
            "unknown_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "unknown_test".to_string(),
            refs: vec![make_var_ref("x", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = ["x = some_function()"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();
        let val = result.value_at_exit(0, "x");
        // Should be top (unknown)
        assert_eq!(val.type_, None);
        assert_eq!(val.range_, None);
        assert_eq!(val.nullable, Nullability::Maybe);
    }

    #[test]
    fn test_parameter_starts_as_top() {
        // Function parameters start as top() (unknown input)
        let cfg = make_test_cfg(
            "param_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "param_test".to_string(),
            refs: vec![make_var_ref("param", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["param".to_string()],
        };
        // No source - just parameter definition with no assignment
        let result = compute_abstract_interp(&cfg, &dfg, None, "python").unwrap();
        let val = result.value_at(0, "param");
        // Parameters start as top (unknown)
        assert_eq!(val.type_, None);
        assert_eq!(val.range_, None);
        assert_eq!(val.nullable, Nullability::Maybe);
    }

    #[test]
    fn test_nested_loops_terminate() {
        // Nested loops should terminate via widening
        // Create a CFG with nested loop structure
        let cfg = CfgInfo {
            function: "nested_loop".to_string(),
            blocks: vec![
                CfgBlock {
                    id: 0,
                    block_type: BlockType::Entry,
                    lines: (1, 1),
                    calls: vec![],
                },
                CfgBlock {
                    id: 1,
                    block_type: BlockType::LoopHeader,
                    lines: (2, 2),
                    calls: vec![],
                },
                CfgBlock {
                    id: 2,
                    block_type: BlockType::LoopHeader,
                    lines: (3, 3),
                    calls: vec![],
                },
                CfgBlock {
                    id: 3,
                    block_type: BlockType::LoopBody,
                    lines: (4, 4),
                    calls: vec![],
                },
                CfgBlock {
                    id: 4,
                    block_type: BlockType::Exit,
                    lines: (5, 5),
                    calls: vec![],
                },
            ],
            edges: vec![
                CfgEdge {
                    from: 0,
                    to: 1,
                    edge_type: EdgeType::Unconditional,
                    condition: None,
                },
                CfgEdge {
                    from: 1,
                    to: 2,
                    edge_type: EdgeType::True,
                    condition: Some("i < n".to_string()),
                },
                CfgEdge {
                    from: 1,
                    to: 4,
                    edge_type: EdgeType::False,
                    condition: None,
                },
                CfgEdge {
                    from: 2,
                    to: 3,
                    edge_type: EdgeType::True,
                    condition: Some("j < m".to_string()),
                },
                CfgEdge {
                    from: 2,
                    to: 1,
                    edge_type: EdgeType::False,
                    condition: None,
                },
                CfgEdge {
                    from: 3,
                    to: 2,
                    edge_type: EdgeType::BackEdge,
                    condition: None,
                },
            ],
            entry_block: 0,
            exit_blocks: vec![4],
            cyclomatic_complexity: 3,
            nested_functions: HashMap::new(),
        };
        let dfg = DfgInfo {
            function: "nested_loop".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 4, 0),
                make_var_ref("x", RefType::Use, 4, 4),
            ],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = [
            "x = 0",
            "for i in range(n):",
            "  for j in range(m):",
            "    x = x + 1",
            "return x",
        ];
        let source_refs: Vec<&str> = source.to_vec();

        // Should not infinite loop - widening ensures termination
        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python");
        assert!(result.is_ok());
    }

    #[test]
    fn test_compute_accepts_language_parameter() {
        // compute_abstract_interp should accept language parameter
        let cfg = make_test_cfg(
            "lang_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "lang_test".to_string(),
            refs: vec![],
            edges: vec![],
            variables: vec![],
        };

        // Both should succeed with different languages
        let result_py = compute_abstract_interp(&cfg, &dfg, None, "python");
        let result_ts = compute_abstract_interp(&cfg, &dfg, None, "typescript");
        assert!(result_py.is_ok());
        assert!(result_ts.is_ok());
    }

    #[test]
    fn test_compute_with_typescript_null() {
        // TypeScript: let x = null;
        // Should recognize 'null' as null value
        let cfg = make_test_cfg(
            "ts_null_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "ts_null_test".to_string(),
            refs: vec![make_var_ref("x", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = ["let x = null"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "typescript").unwrap();
        let val = result.value_at_exit(0, "x");
        assert_eq!(val.nullable, Nullability::Always);
    }

    #[test]
    fn test_compute_with_go_nil() {
        // Go: x := nil
        // Should recognize 'nil' as null value
        let cfg = make_test_cfg(
            "go_nil_test",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 1),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "go_nil_test".to_string(),
            refs: vec![make_var_ref("x", RefType::Definition, 1, 0)],
            edges: vec![],
            variables: vec!["x".to_string()],
        };
        let source = ["x := nil"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "go").unwrap();
        let val = result.value_at_exit(0, "x");
        assert_eq!(val.nullable, Nullability::Always);
    }

    // =========================================================================
    // Phase 11 Tests: Division-by-Zero and Null Dereference Detection
    // =========================================================================

    #[test]
    fn test_div_zero_detected_for_constant_zero() {
        // CAP-AI-10: x=0; y=1/x -> warning at y
        let cfg = make_test_cfg(
            "div_zero_const",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "div_zero_const".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 6),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 0", "y = 1 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // Should detect division by zero at line 2
        assert!(
            !result.potential_div_zero.is_empty(),
            "Should detect division by zero"
        );
        assert!(
            result
                .potential_div_zero
                .iter()
                .any(|(line, var)| *line == 2 && var == "x"),
            "Should flag x at line 2 as potential div-by-zero"
        );
    }

    #[test]
    fn test_div_zero_detected_for_range_including_zero() {
        // CAP-AI-10: Range [-5, 5] includes zero, should warn
        let cfg = make_test_cfg(
            "div_zero_range",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "div_zero_range".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0), // x is unknown (top)
                make_var_ref("x", RefType::Use, 2, 6),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        // x = foo() returns unknown value (could be zero)
        let source = ["x = foo()", "y = 1 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // Unknown value may be zero
        assert!(
            !result.potential_div_zero.is_empty(),
            "Should detect potential division by zero for unknown value"
        );
    }

    #[test]
    fn test_div_safe_no_warning_for_constant_nonzero() {
        // CAP-AI-10: x=5; y=1/x -> NO warning
        let cfg = make_test_cfg(
            "div_safe_const",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "div_safe_const".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 6),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 5", "y = 1 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // x = 5, definitely not zero
        assert!(
            result.potential_div_zero.is_empty()
                || !result
                    .potential_div_zero
                    .iter()
                    .any(|(line, var)| *line == 2 && var == "x"),
            "Should NOT warn for division by constant non-zero"
        );
    }

    #[test]
    fn test_div_safe_no_warning_for_positive_range() {
        // CAP-AI-10: x=5; x=x+1; y=1/x -> NO warning (range [6,6])
        let cfg = make_test_cfg(
            "div_safe_range",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 3),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "div_safe_range".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 4),
                make_var_ref("x", RefType::Definition, 2, 0),
                make_var_ref("x", RefType::Use, 3, 6),
                make_var_ref("y", RefType::Definition, 3, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 5", "x = x + 1", "y = 1 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // x has range [6, 6], definitely not zero
        assert!(
            result.potential_div_zero.is_empty()
                || !result
                    .potential_div_zero
                    .iter()
                    .any(|(line, var)| *line == 3 && var == "x"),
            "Should NOT warn for positive range that excludes zero"
        );
    }

    #[test]
    fn test_div_zero_intra_block_accuracy() {
        // CAP-AI-20 / TIGER-PASS1-13: Intra-block precision
        // x = 0       # line 1
        // x = 5       # line 2 - redefined to non-zero
        // y = 1 / x   # line 3 - should NOT warn (x is 5 at this point)
        let cfg = make_test_cfg(
            "div_intra_block",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 3),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "div_intra_block".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Definition, 2, 0), // Redefined
                make_var_ref("x", RefType::Use, 3, 6),
                make_var_ref("y", RefType::Definition, 3, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 0", "x = 5", "y = 1 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // At line 3, x should be 5 (not 0), so no warning
        // This tests intra-block precision
        assert!(result.potential_div_zero.is_empty() ||
                !result.potential_div_zero.iter().any(|(line, var)| *line == 3 && var == "x"),
            "Should NOT warn when divisor is redefined to non-zero before division (intra-block precision)");
    }

    #[test]
    fn test_div_zero_not_triggered_by_path_strings() {
        // Regression: Path::new("/projects/myapp") was flagged as division
        // because the `/` in string literals was not stripped.
        let cfg = make_test_cfg(
            "path_strings",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "path_strings".to_string(),
            refs: vec![
                make_var_ref("root", RefType::Definition, 1, 0),
                make_var_ref("child", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["root".to_string(), "child".to_string()],
        };
        let source = ["root = \"/projects/myapp\"", "child = \"/src/main.rs\""];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        assert!(
            result.potential_div_zero.is_empty(),
            "Path separators inside string literals must not trigger div-by-zero; got: {:?}",
            result.potential_div_zero
        );
    }

    #[test]
    fn test_div_zero_still_detects_real_division_with_strings() {
        // Real division must still be detected even when string paths are on same line
        let cfg = make_test_cfg(
            "mixed_strings_div",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 3),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "mixed_strings_div".to_string(),
            refs: vec![
                make_var_ref("path", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Definition, 2, 0),
                make_var_ref("y", RefType::Definition, 3, 0),
                make_var_ref("x", RefType::Use, 3, 10),
            ],
            edges: vec![],
            variables: vec!["path".to_string(), "x".to_string(), "y".to_string()],
        };
        let source = ["path = \"/src/main.rs\"", "x = foo()", "y = 100 / x"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        assert!(
            result
                .potential_div_zero
                .iter()
                .any(|(line, var)| *line == 3 && var == "x"),
            "Real division by unknown x should still be flagged; got: {:?}",
            result.potential_div_zero
        );
        // And no FP from the path string
        assert!(
            !result
                .potential_div_zero
                .iter()
                .any(|(_, var)| var == "main" || var == "src"),
            "Path components in strings must not be flagged; got: {:?}",
            result.potential_div_zero
        );
    }

    #[test]
    fn test_null_deref_detected_at_attribute_access() {
        // CAP-AI-11: x=None; y=x.foo -> warning at y
        let cfg = make_test_cfg(
            "null_deref",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "null_deref".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 4),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = None", "y = x.foo"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // Should detect null dereference
        assert!(
            !result.potential_null_deref.is_empty(),
            "Should detect null dereference"
        );
        assert!(
            result
                .potential_null_deref
                .iter()
                .any(|(line, var)| *line == 2 && var == "x"),
            "Should flag x at line 2 as potential null deref"
        );
    }

    #[test]
    fn test_null_deref_safe_for_non_null_constant() {
        // CAP-AI-11: x='hello'; y=x.upper() -> NO warning
        let cfg = make_test_cfg(
            "null_safe",
            vec![CfgBlock {
                id: 0,
                block_type: BlockType::Entry,
                lines: (1, 2),
                calls: vec![],
            }],
            vec![],
        );
        let dfg = DfgInfo {
            function: "null_safe".to_string(),
            refs: vec![
                make_var_ref("x", RefType::Definition, 1, 0),
                make_var_ref("x", RefType::Use, 2, 4),
                make_var_ref("y", RefType::Definition, 2, 0),
            ],
            edges: vec![],
            variables: vec!["x".to_string(), "y".to_string()],
        };
        let source = ["x = 'hello'", "y = x.upper()"];
        let source_refs: Vec<&str> = source.to_vec();

        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_refs), "python").unwrap();

        // String constant is not null
        assert!(
            result.potential_null_deref.is_empty()
                || !result
                    .potential_null_deref
                    .iter()
                    .any(|(line, var)| *line == 2 && var == "x"),
            "Should NOT warn for dereference of non-null constant"
        );
    }
}
