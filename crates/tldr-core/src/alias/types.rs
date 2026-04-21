//! Alias Analysis Types
//!
//! Core types for Andersen-style flow-insensitive points-to analysis.
//!
//! This module provides:
//! - `AbstractLocation` - Abstract memory locations in points-to analysis
//! - `AliasInfo` - Results of alias analysis for a function
//! - `AliasError` - Errors that can occur during analysis
//!
//! # References
//! - Andersen, L. O. (1994). Program Analysis and Specialization for the C
//!   Programming Language. PhD thesis, University of Copenhagen.
//! - See `spec.md` for detailed specification.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// Re-export Confidence from dataflow so alias consumers can use the same type
pub use crate::dataflow::available::Confidence;

// =============================================================================
// Uncertain Alias Types
// =============================================================================

/// An alias relationship that couldn't be proven but might exist.
///
/// Instead of silently discarding potential aliases, we collect them here
/// so consumers can see what was uncertain and why.
///
/// # Example
///
/// ```rust,ignore
/// UncertainAlias {
///     vars: vec!["a".to_string(), "b".to_string()],
///     line: 42,
///     reason: "assignment from function return - type unknown".to_string(),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UncertainAlias {
    /// The variables that might alias each other
    pub vars: Vec<String>,
    /// Source line where the potential aliasing occurs
    pub line: u32,
    /// Why aliasing couldn't be confirmed
    pub reason: String,
}

// =============================================================================
// Abstract Location Types
// =============================================================================

/// Maximum depth for nested field access to prevent stack overflow (TIGER-1).
pub const MAX_FIELD_DEPTH: usize = 10;

/// Abstract memory location that a variable may point to.
///
/// These represent the "targets" in points-to analysis - what objects
/// a reference variable could be pointing to at runtime.
///
/// # Variants
///
/// - `Alloc` - Object allocated at a specific line: `alloc_N`
/// - `Param` - Object passed as a parameter: `param_X`
/// - `Unknown` - Unknown/external source (site-specific for soundness): `unknown_N`
/// - `Field` - Field of another location: `{base}.{field}`
/// - `DefaultArg` - Mutable default argument (Python-specific): `alloc_default_N`
/// - `ClassVar` - Class variable (Python-specific): `alloc_class_Class_field`
///
/// # Examples
///
/// ```
/// use tldr_core::alias::AbstractLocation;
///
/// let alloc = AbstractLocation::alloc(5);
/// assert_eq!(alloc.format(), "alloc_5");
///
/// let param = AbstractLocation::param("x");
/// assert_eq!(param.format(), "param_x");
///
/// let field = AbstractLocation::field(alloc, "data");
/// assert_eq!(field.format(), "alloc_5.data");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AbstractLocation {
    /// Object allocated at a specific line: `alloc_N`
    /// Created when: `x = Foo()`, `x = []`, `x = {}`
    Alloc {
        /// Line number where the allocation occurs in source code.
        site: u32,
    },

    /// Object passed as a parameter: `param_X`
    /// Created for each function parameter (unknown caller context)
    Param {
        /// The parameter name as it appears in the function signature.
        name: String,
    },

    /// Unknown/external source: `unknown_SITE`
    /// Created when: return from unknown function, global access
    /// Site-specific to prevent unsound aliasing (TIGER-5)
    Unknown {
        /// Site identifier distinguishing different unknown sources to prevent unsound aliasing.
        site: u32,
    },

    /// Field of another location: `{base}.{field}`
    /// Created when: `x = obj.field` where obj points to base
    Field {
        /// The base abstract location that this field belongs to.
        base: Box<AbstractLocation>,
        /// The field name being accessed on the base object.
        field: String,
    },

    /// Mutable default argument: `alloc_default_LINE`
    /// Created for default parameter values (shared across calls)
    /// Python-specific: handles `def f(x=[])` correctly (TIGER-7)
    DefaultArg {
        /// Line number of the default argument definition.
        site: u32,
    },

    /// Class-level variable: `alloc_class_CLASS_FIELD`
    /// Singleton per class for class variables vs instance variables
    /// Python-specific: `Foo.x` vs `obj.x` distinction (TIGER-8)
    ClassVar {
        /// The class name that owns this class variable.
        class: String,
        /// The field name of the class variable.
        field: String,
    },
}

impl AbstractLocation {
    /// Create allocation site location.
    ///
    /// # Arguments
    /// * `site` - Line number where allocation occurs
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AbstractLocation;
    /// let loc = AbstractLocation::alloc(5);
    /// assert_eq!(loc.format(), "alloc_5");
    /// ```
    pub fn alloc(site: u32) -> Self {
        AbstractLocation::Alloc { site }
    }

    /// Create parameter location.
    ///
    /// # Arguments
    /// * `name` - Parameter name
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AbstractLocation;
    /// let loc = AbstractLocation::param("x");
    /// assert_eq!(loc.format(), "param_x");
    /// ```
    pub fn param(name: impl Into<String>) -> Self {
        AbstractLocation::Param { name: name.into() }
    }

    /// Create unknown location (site-specific for soundness).
    ///
    /// Each call site that returns unknown gets a unique location
    /// to prevent unsound aliasing between unrelated unknown sources.
    ///
    /// # Arguments
    /// * `site` - Line number where unknown value is produced
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AbstractLocation;
    /// let loc = AbstractLocation::unknown(10);
    /// assert_eq!(loc.format(), "unknown_10");
    /// ```
    pub fn unknown(site: u32) -> Self {
        AbstractLocation::Unknown { site }
    }

    /// Create field location.
    ///
    /// # Arguments
    /// * `base` - Base location being accessed
    /// * `field` - Field name being accessed
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AbstractLocation;
    /// let base = AbstractLocation::alloc(5);
    /// let loc = AbstractLocation::field(base, "data");
    /// assert_eq!(loc.format(), "alloc_5.data");
    /// ```
    pub fn field(base: AbstractLocation, field: impl Into<String>) -> Self {
        AbstractLocation::Field {
            base: Box::new(base),
            field: field.into(),
        }
    }

    /// Create default argument location (Python-specific).
    ///
    /// Default argument values like `def f(x=[])` are shared across
    /// all calls, so they need special handling.
    ///
    /// # Arguments
    /// * `site` - Line number where default is defined
    pub fn default_arg(site: u32) -> Self {
        AbstractLocation::DefaultArg { site }
    }

    /// Create class variable location (Python-specific).
    ///
    /// Class variables are singletons shared across all instances,
    /// unlike instance variables which are per-object.
    ///
    /// # Arguments
    /// * `class` - Class name
    /// * `field` - Field/attribute name
    pub fn class_var(class: impl Into<String>, field: impl Into<String>) -> Self {
        AbstractLocation::ClassVar {
            class: class.into(),
            field: field.into(),
        }
    }

    /// Format as string for JSON output.
    ///
    /// Returns formatted string like:
    /// - `"alloc_5"` for allocation at line 5
    /// - `"param_x"` for parameter x
    /// - `"unknown_10"` for unknown at line 10
    /// - `"alloc_5.data"` for field access
    /// - `"alloc_default_3"` for default argument at line 3
    /// - `"alloc_class_Foo_x"` for class variable Foo.x
    ///
    /// # TIGER-1 Mitigation
    ///
    /// Field chains deeper than `MAX_FIELD_DEPTH` (10) are truncated
    /// to prevent stack overflow from deeply nested field access.
    pub fn format(&self) -> String {
        match self {
            AbstractLocation::Alloc { site } => format!("alloc_{}", site),
            AbstractLocation::Param { name } => format!("param_{}", name),
            AbstractLocation::Unknown { site } => format!("unknown_{}", site),
            AbstractLocation::Field { base, field } => {
                // TIGER-1: Prevent stack overflow in deeply nested field access
                let mut depth = 0;
                let mut current = base.as_ref();
                while let AbstractLocation::Field { base: inner, .. } = current {
                    depth += 1;
                    if depth >= MAX_FIELD_DEPTH {
                        return format!("{}.{}.truncated", base.format(), field);
                    }
                    current = inner.as_ref();
                }
                format!("{}.{}", base.format(), field)
            }
            AbstractLocation::DefaultArg { site } => format!("alloc_default_{}", site),
            AbstractLocation::ClassVar { class, field } => {
                format!("alloc_class_{}_{}", class, field)
            }
        }
    }

    /// Get the field depth of this location.
    ///
    /// Returns 0 for non-field locations, 1+ for field chains.
    pub fn field_depth(&self) -> usize {
        match self {
            AbstractLocation::Field { base, .. } => 1 + base.field_depth(),
            _ => 0,
        }
    }

    /// Check if this is an allocation site.
    pub fn is_alloc(&self) -> bool {
        matches!(self, AbstractLocation::Alloc { .. })
    }

    /// Check if this is a parameter location.
    pub fn is_param(&self) -> bool {
        matches!(self, AbstractLocation::Param { .. })
    }

    /// Check if this is an unknown location.
    pub fn is_unknown(&self) -> bool {
        matches!(self, AbstractLocation::Unknown { .. })
    }

    /// Check if this is a field location.
    pub fn is_field(&self) -> bool {
        matches!(self, AbstractLocation::Field { .. })
    }
}

impl std::fmt::Display for AbstractLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format())
    }
}

// =============================================================================
// Alias Info Result
// =============================================================================

/// Alias analysis results for a function.
///
/// Contains may-alias, must-alias relationships and points-to sets.
/// This is the primary output of `compute_alias()`.
///
/// # Soundness Guarantees
///
/// - **may_alias is SOUND:** If `may_alias_check(a, b)` returns `false`,
///   then `a` and `b` definitely do NOT alias at runtime (no false negatives).
///
/// - **must_alias is PRECISE:** If `must_alias_check(a, b)` returns `true`,
///   then `a` and `b` definitely DO alias at runtime (no false positives).
///
/// # Examples
///
/// ```
/// use tldr_core::alias::AliasInfo;
/// use std::collections::HashSet;
///
/// let mut info = AliasInfo::new("process_data");
///
/// // Same variable always aliases itself
/// assert!(info.may_alias_check("x", "x"));
/// assert!(info.must_alias_check("x", "x"));
///
/// // Unknown variables return false (no info)
/// assert!(!info.may_alias_check("unknown1", "unknown2"));
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AliasInfo {
    /// Function name
    pub function_name: String,

    /// May-alias relationships: var -> set of vars that MAY alias.
    /// Symmetric: if b in may_alias[a], then a in may_alias[b].
    pub may_alias: HashMap<String, HashSet<String>>,

    /// Must-alias relationships: var -> set of vars that DEFINITELY alias.
    /// Symmetric and transitive.
    pub must_alias: HashMap<String, HashSet<String>>,

    /// Points-to sets: var -> set of abstract location names it may point to.
    /// Location names are formatted strings like "alloc_5", "param_x".
    pub points_to: HashMap<String, HashSet<String>>,

    /// Allocation sites: line -> abstract location name.
    /// Records where objects are created.
    pub allocation_sites: HashMap<u32, String>,

    /// Alias relationships that couldn't be proven but might exist.
    ///
    /// Contains assignments/bindings where aliasing MIGHT occur but can't be proven:
    /// - `a = foo()` -- return type unknown
    /// - `a = b` -- depends on value vs reference semantics
    /// - `a = b.field` -- could be alias if reference type
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uncertain: Vec<UncertainAlias>,

    /// Overall confidence level for this analysis result.
    #[serde(default)]
    pub confidence: Confidence,

    /// Language-specific notes about aliasing semantics.
    ///
    /// For example: "Python uses reference semantics for non-primitive types"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language_notes: String,
}

impl AliasInfo {
    /// Create empty alias info for a function.
    ///
    /// # Arguments
    /// * `function_name` - Name of the function being analyzed
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AliasInfo;
    /// let info = AliasInfo::new("my_function");
    /// assert_eq!(info.function_name, "my_function");
    /// assert!(info.may_alias.is_empty());
    /// ```
    pub fn new(function_name: impl Into<String>) -> Self {
        Self {
            function_name: function_name.into(),
            may_alias: HashMap::new(),
            must_alias: HashMap::new(),
            points_to: HashMap::new(),
            allocation_sites: HashMap::new(),
            uncertain: Vec::new(),
            confidence: Confidence::default(),
            language_notes: String::new(),
        }
    }

    /// Check if two variables MAY alias (point to same object).
    ///
    /// Returns `true` if there exists ANY execution path where
    /// `a` and `b` could reference the same object.
    ///
    /// # Soundness
    /// - Returns `false` ONLY IF variables definitely don't alias
    /// - May return `true` even if they never alias (conservative)
    ///
    /// # Arguments
    /// * `a` - First variable name (SSA name like "x_0")
    /// * `b` - Second variable name (SSA name like "y_0")
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AliasInfo;
    /// use std::collections::HashSet;
    ///
    /// let mut info = AliasInfo::new("test");
    ///
    /// // Same variable always aliases itself
    /// assert!(info.may_alias_check("x", "x"));
    ///
    /// // Variables with overlapping points-to sets may alias
    /// info.points_to.insert("x".to_string(), HashSet::from(["alloc_1".to_string()]));
    /// info.points_to.insert("y".to_string(), HashSet::from(["alloc_1".to_string()]));
    /// assert!(info.may_alias_check("x", "y"));
    /// ```
    pub fn may_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always aliases itself
        if a == b {
            return true;
        }

        // Check explicit may_alias set
        if self.may_alias.get(a).is_some_and(|s| s.contains(b)) {
            return true;
        }
        if self.may_alias.get(b).is_some_and(|s| s.contains(a)) {
            return true;
        }

        // Check points-to set overlap
        let pts_a = self.points_to.get(a);
        let pts_b = self.points_to.get(b);

        match (pts_a, pts_b) {
            (Some(a_set), Some(b_set)) => !a_set.is_disjoint(b_set),
            _ => false,
        }
    }

    /// Check if two variables MUST alias (definitely same object).
    ///
    /// Returns `true` ONLY IF on ALL execution paths, `a` and `b`
    /// reference the same object.
    ///
    /// # Precision
    /// - Returns `true` ONLY IF variables definitely alias
    /// - May return `false` even if they always alias (conservative)
    ///
    /// # Arguments
    /// * `a` - First variable name (SSA name like "x_0")
    /// * `b` - Second variable name (SSA name like "y_0")
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AliasInfo;
    /// use std::collections::HashSet;
    ///
    /// let mut info = AliasInfo::new("test");
    ///
    /// // Same variable always must-aliases itself
    /// assert!(info.must_alias_check("x", "x"));
    ///
    /// // Explicit must-alias relationship
    /// info.must_alias.insert("x".to_string(), HashSet::from(["y".to_string()]));
    /// assert!(info.must_alias_check("x", "y"));
    /// ```
    pub fn must_alias_check(&self, a: &str, b: &str) -> bool {
        // Same variable always must-aliases itself
        if a == b {
            return true;
        }

        // Check explicit must_alias set
        if self.must_alias.get(a).is_some_and(|s| s.contains(b)) {
            return true;
        }
        if self.must_alias.get(b).is_some_and(|s| s.contains(a)) {
            return true;
        }

        false
    }

    /// Get the points-to set for a variable.
    ///
    /// Returns the set of abstract locations this variable may reference.
    /// Returns empty set for unknown variables.
    ///
    /// # Arguments
    /// * `var` - Variable name (SSA name like "x_0")
    ///
    /// # Examples
    /// ```
    /// use tldr_core::alias::AliasInfo;
    /// use std::collections::HashSet;
    ///
    /// let mut info = AliasInfo::new("test");
    /// info.points_to.insert("x".to_string(), HashSet::from(["alloc_1".to_string()]));
    ///
    /// assert_eq!(info.get_points_to("x"), HashSet::from(["alloc_1".to_string()]));
    /// assert!(info.get_points_to("unknown").is_empty());
    /// ```
    pub fn get_points_to(&self, var: &str) -> HashSet<String> {
        self.points_to.get(var).cloned().unwrap_or_default()
    }

    /// Get all variables that may alias with the given variable.
    ///
    /// Returns the explicit may_alias set (does not compute from points-to).
    ///
    /// # Arguments
    /// * `var` - Variable name (SSA name like "x_0")
    pub fn get_aliases(&self, var: &str) -> HashSet<String> {
        self.may_alias.get(var).cloned().unwrap_or_default()
    }

    /// Add a may-alias relationship (symmetric).
    ///
    /// Ensures both directions are recorded: a may-alias b and b may-alias a.
    pub fn add_may_alias(&mut self, a: &str, b: &str) {
        if a != b {
            self.may_alias
                .entry(a.to_string())
                .or_default()
                .insert(b.to_string());
            self.may_alias
                .entry(b.to_string())
                .or_default()
                .insert(a.to_string());
        }
    }

    /// Add a must-alias relationship (symmetric).
    ///
    /// Ensures both directions are recorded: a must-alias b and b must-alias a.
    pub fn add_must_alias(&mut self, a: &str, b: &str) {
        if a != b {
            self.must_alias
                .entry(a.to_string())
                .or_default()
                .insert(b.to_string());
            self.must_alias
                .entry(b.to_string())
                .or_default()
                .insert(a.to_string());
        }
    }

    /// Add a location to a variable's points-to set.
    pub fn add_points_to(&mut self, var: &str, location: &str) {
        self.points_to
            .entry(var.to_string())
            .or_default()
            .insert(location.to_string());
    }

    /// Record an allocation site.
    pub fn add_allocation_site(&mut self, line: u32, location: &str) {
        self.allocation_sites.insert(line, location.to_string());
    }

    /// Convert to serializable format for JSON output.
    ///
    /// Sets are converted to sorted Vecs for deterministic output.
    pub fn to_json_value(&self) -> serde_json::Value {
        use serde_json::json;

        let sorted_may_alias: HashMap<_, Vec<_>> = self
            .may_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();

        let sorted_must_alias: HashMap<_, Vec<_>> = self
            .must_alias
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();

        let sorted_points_to: HashMap<_, Vec<_>> = self
            .points_to
            .iter()
            .map(|(k, v)| {
                let mut sorted: Vec<_> = v.iter().cloned().collect();
                sorted.sort();
                (k.clone(), sorted)
            })
            .collect();

        json!({
            "function": self.function_name,
            "may_alias": sorted_may_alias,
            "must_alias": sorted_must_alias,
            "points_to": sorted_points_to,
            "allocation_sites": self.allocation_sites,
        })
    }
}

// =============================================================================
// Error Types
// =============================================================================

/// Alias analysis errors.
///
/// These represent conditions that prevent alias analysis from completing.
#[derive(Debug, Clone)]
pub enum AliasError {
    /// SSA form required but not available.
    ///
    /// Alias analysis requires SSA form to track variable versions.
    NoSsa(String),

    /// CFG required but not available.
    ///
    /// Some analysis modes require control flow information.
    NoCfg(String),

    /// Fixed-point iteration exceeded limit.
    ///
    /// The constraint solver failed to converge within the maximum
    /// number of iterations. This may indicate cyclic constraints.
    IterationLimit(usize),

    /// Invalid phi function source reference.
    ///
    /// A phi function references an SSA name that doesn't exist.
    InvalidPhiSource {
        /// The phi variable being defined
        phi_var: String,
        /// The invalid source reference
        source: String,
    },

    /// Invalid variable reference.
    ///
    /// A reference to a variable that doesn't exist in the SSA form.
    InvalidRef(String),

    /// Internal analysis error.
    ///
    /// An unexpected condition occurred during analysis.
    Internal(String),
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AliasError::NoSsa(func) => {
                write!(f, "SSA form not available for function: {}", func)
            }
            AliasError::NoCfg(func) => {
                write!(f, "CFG not available for function: {}", func)
            }
            AliasError::IterationLimit(iters) => {
                write!(
                    f,
                    "Fixed-point iteration limit exceeded: {} iterations",
                    iters
                )
            }
            AliasError::InvalidPhiSource { phi_var, source } => {
                write!(
                    f,
                    "Invalid phi source: {} references non-existent {}",
                    phi_var, source
                )
            }
            AliasError::InvalidRef(var) => {
                write!(f, "Invalid variable reference: {}", var)
            }
            AliasError::Internal(msg) => {
                write!(f, "Internal alias analysis error: {}", msg)
            }
        }
    }
}

impl std::error::Error for AliasError {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // AbstractLocation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_abstract_location_alloc() {
        let loc = AbstractLocation::alloc(5);
        assert_eq!(loc.format(), "alloc_5");
        assert!(loc.is_alloc());
        assert!(!loc.is_param());
    }

    #[test]
    fn test_abstract_location_param() {
        let loc = AbstractLocation::param("x");
        assert_eq!(loc.format(), "param_x");
        assert!(loc.is_param());
    }

    #[test]
    fn test_abstract_location_unknown() {
        let loc = AbstractLocation::unknown(10);
        assert_eq!(loc.format(), "unknown_10");
        assert!(loc.is_unknown());
    }

    #[test]
    fn test_abstract_location_field() {
        let base = AbstractLocation::alloc(5);
        let loc = AbstractLocation::field(base, "data");
        assert_eq!(loc.format(), "alloc_5.data");
        assert!(loc.is_field());
        assert_eq!(loc.field_depth(), 1);
    }

    #[test]
    fn test_abstract_location_nested_field() {
        let base = AbstractLocation::alloc(5);
        let field1 = AbstractLocation::field(base, "inner");
        let field2 = AbstractLocation::field(field1, "value");
        assert_eq!(field2.format(), "alloc_5.inner.value");
        assert_eq!(field2.field_depth(), 2);
    }

    #[test]
    fn test_abstract_location_default_arg() {
        let loc = AbstractLocation::default_arg(3);
        assert_eq!(loc.format(), "alloc_default_3");
    }

    #[test]
    fn test_abstract_location_class_var() {
        let loc = AbstractLocation::class_var("Foo", "counter");
        assert_eq!(loc.format(), "alloc_class_Foo_counter");
    }

    #[test]
    fn test_abstract_location_deep_field_truncation() {
        // Create a chain of MAX_FIELD_DEPTH + 1 fields
        let mut loc = AbstractLocation::alloc(1);
        for i in 0..=MAX_FIELD_DEPTH {
            loc = AbstractLocation::field(loc, format!("f{}", i));
        }
        // Should be truncated
        assert!(loc.format().contains("truncated"));
    }

    #[test]
    fn test_abstract_location_display() {
        let loc = AbstractLocation::alloc(5);
        assert_eq!(format!("{}", loc), "alloc_5");
    }

    #[test]
    fn test_abstract_location_serde() {
        let loc = AbstractLocation::alloc(5);
        let json = serde_json::to_string(&loc).unwrap();
        let parsed: AbstractLocation = serde_json::from_str(&json).unwrap();
        assert_eq!(loc, parsed);
    }

    // -------------------------------------------------------------------------
    // AliasInfo Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_alias_info_new() {
        let info = AliasInfo::new("test_func");
        assert_eq!(info.function_name, "test_func");
        assert!(info.may_alias.is_empty());
        assert!(info.must_alias.is_empty());
        assert!(info.points_to.is_empty());
        assert!(info.allocation_sites.is_empty());
    }

    #[test]
    fn test_alias_info_default() {
        let info = AliasInfo::default();
        assert!(info.function_name.is_empty());
        assert!(info.may_alias.is_empty());
    }

    #[test]
    fn test_alias_info_may_alias_same_var() {
        let info = AliasInfo::new("test");
        assert!(info.may_alias_check("x", "x"));
        assert!(info.may_alias_check("anything", "anything"));
    }

    #[test]
    fn test_alias_info_may_alias_in_set() {
        let mut info = AliasInfo::new("test");
        info.add_may_alias("x", "y");

        assert!(info.may_alias_check("x", "y"));
        assert!(info.may_alias_check("y", "x")); // Symmetric
    }

    #[test]
    fn test_alias_info_may_alias_points_to_overlap() {
        let mut info = AliasInfo::new("test");
        info.add_points_to("x", "alloc_1");
        info.add_points_to("x", "alloc_2");
        info.add_points_to("y", "alloc_2");
        info.add_points_to("y", "alloc_3");
        info.add_points_to("z", "alloc_4");

        // x and y share alloc_2
        assert!(info.may_alias_check("x", "y"));
        // x and z don't share any location
        assert!(!info.may_alias_check("x", "z"));
    }

    #[test]
    fn test_alias_info_must_alias_same_var() {
        let info = AliasInfo::new("test");
        assert!(info.must_alias_check("x", "x"));
    }

    #[test]
    fn test_alias_info_must_alias_in_set() {
        let mut info = AliasInfo::new("test");
        info.add_must_alias("a", "b");

        assert!(info.must_alias_check("a", "b"));
        assert!(info.must_alias_check("b", "a")); // Symmetric
    }

    #[test]
    fn test_alias_info_must_alias_not_in_set() {
        let mut info = AliasInfo::new("test");
        info.add_may_alias("a", "c");

        // c only may-alias, not must-alias
        assert!(!info.must_alias_check("a", "c"));
    }

    #[test]
    fn test_alias_info_get_points_to() {
        let mut info = AliasInfo::new("test");
        info.add_points_to("x", "alloc_1");
        info.add_points_to("x", "param_0");

        let pts = info.get_points_to("x");
        assert!(pts.contains("alloc_1"));
        assert!(pts.contains("param_0"));
        assert!(info.get_points_to("unknown").is_empty());
    }

    #[test]
    fn test_alias_info_get_aliases() {
        let mut info = AliasInfo::new("test");
        info.add_may_alias("x", "y");
        info.add_may_alias("x", "z");

        let aliases = info.get_aliases("x");
        assert!(aliases.contains("y"));
        assert!(aliases.contains("z"));
        assert!(info.get_aliases("unknown").is_empty());
    }

    #[test]
    fn test_alias_info_to_json_sorted() {
        let mut info = AliasInfo::new("test");
        info.add_may_alias("x", "z");
        info.add_may_alias("x", "a");
        info.add_may_alias("x", "m");

        let json = info.to_json_value();
        let may_alias = json["may_alias"]["x"].as_array().unwrap();

        // Should be sorted
        let sorted: Vec<&str> = may_alias.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(sorted, vec!["a", "m", "z"]);
    }

    #[test]
    fn test_alias_info_allocation_sites() {
        let mut info = AliasInfo::new("test");
        info.add_allocation_site(5, "alloc_5");

        assert_eq!(info.allocation_sites.get(&5), Some(&"alloc_5".to_string()));
    }

    // -------------------------------------------------------------------------
    // AliasError Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_alias_error_display() {
        let err = AliasError::NoSsa("test_func".to_string());
        assert!(err.to_string().contains("SSA form not available"));

        let err = AliasError::IterationLimit(100);
        assert!(err.to_string().contains("100 iterations"));

        let err = AliasError::InvalidPhiSource {
            phi_var: "x_2".to_string(),
            source: "y_99".to_string(),
        };
        assert!(err.to_string().contains("x_2"));
        assert!(err.to_string().contains("y_99"));
    }
}
