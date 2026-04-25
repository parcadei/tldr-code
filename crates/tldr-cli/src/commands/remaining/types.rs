//! Shared types for remaining commands
//!
//! This module defines all data types used across the remaining analysis
//! commands. Types are designed for JSON serialization with serde.

use std::collections::HashMap;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// =============================================================================
// Output Format
// =============================================================================

/// Output format for all commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// JSON output (default)
    #[default]
    Json,
    /// Human-readable text output
    Text,
    /// SARIF format (only for vuln command)
    Sarif,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json => write!(f, "json"),
            Self::Text => write!(f, "text"),
            Self::Sarif => write!(f, "sarif"),
        }
    }
}

// =============================================================================
// Severity Level
// =============================================================================

/// Severity level for findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    #[default]
    Medium,
    Low,
    Info,
}

impl Severity {
    /// Returns ordering value (lower = more severe)
    pub fn order(&self) -> u8 {
        match self {
            Self::Critical => 0,
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
            Self::Info => 4,
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
            Self::Info => write!(f, "info"),
        }
    }
}

// =============================================================================
// Location
// =============================================================================

/// A location in source code (shared across multiple commands).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u32>,
}

impl Location {
    /// Create a new location
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column: 0,
            end_line: None,
            end_column: None,
        }
    }

    /// Create a location with column information
    pub fn with_column(file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
            end_line: None,
            end_column: None,
        }
    }
}

// =============================================================================
// Todo Types
// =============================================================================

/// A single improvement item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// Category of improvement (e.g., "dead_code", "complexity", "cohesion")
    pub category: String,
    /// Priority (lower = higher priority)
    pub priority: u32,
    /// Human-readable description
    pub description: String,
    /// File where the issue was found
    #[serde(default)]
    pub file: String,
    /// Line number
    #[serde(default)]
    pub line: u32,
    /// Severity level
    #[serde(default)]
    pub severity: String,
    /// Score from sub-analysis (0.0-1.0 typically)
    #[serde(default)]
    pub score: f64,
}

impl TodoItem {
    /// Create a new TodoItem
    pub fn new(category: impl Into<String>, priority: u32, description: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            priority,
            description: description.into(),
            file: String::new(),
            line: 0,
            severity: String::new(),
            score: 0.0,
        }
    }

    /// Set file and line
    pub fn with_location(mut self, file: impl Into<String>, line: u32) -> Self {
        self.file = file.into();
        self.line = line;
        self
    }

    /// Set severity
    pub fn with_severity(mut self, severity: impl Into<String>) -> Self {
        self.severity = severity.into();
        self
    }

    /// Set score
    pub fn with_score(mut self, score: f64) -> Self {
        self.score = score;
        self
    }
}

/// Summary of todo analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TodoSummary {
    /// Number of dead code items found
    pub dead_count: u32,
    /// Number of similar code pairs found
    pub similar_pairs: u32,
    /// Number of classes with low cohesion
    pub low_cohesion_count: u32,
    /// Number of complexity hotspots
    pub hotspot_count: u32,
    /// Number of equivalence groups (redundant expressions)
    pub equivalence_groups: u32,
}

/// Todo analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoReport {
    /// Command identifier
    pub wrapper: String,
    /// Path that was analyzed
    pub path: String,
    /// Improvement items sorted by priority
    pub items: Vec<TodoItem>,
    /// Summary statistics
    pub summary: TodoSummary,
    /// Raw results from sub-analyses (when --detail is used)
    #[serde(default)]
    pub sub_results: HashMap<String, Value>,
    /// Total elapsed time in milliseconds
    pub total_elapsed_ms: f64,
}

impl TodoReport {
    /// Create a new TodoReport
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            wrapper: "todo".to_string(),
            path: path.into(),
            items: Vec::new(),
            summary: TodoSummary::default(),
            sub_results: HashMap::new(),
            total_elapsed_ms: 0.0,
        }
    }
}

// =============================================================================
// Secure Types
// =============================================================================

/// A security finding from the secure command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureFinding {
    /// Category of finding (e.g., "taint", "resource_leak", "bounds")
    pub category: String,
    /// Severity level
    pub severity: String,
    /// Human-readable description
    pub description: String,
    /// File where the issue was found
    #[serde(default)]
    pub file: String,
    /// Line number
    #[serde(default)]
    pub line: u32,
}

impl SecureFinding {
    /// Create a new SecureFinding
    pub fn new(
        category: impl Into<String>,
        severity: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            category: category.into(),
            severity: severity.into(),
            description: description.into(),
            file: String::new(),
            line: 0,
        }
    }

    /// Set file and line
    pub fn with_location(mut self, file: impl Into<String>, line: u32) -> Self {
        self.file = file.into();
        self.line = line;
        self
    }
}

/// Security summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecureSummary {
    /// Number of taint-related findings
    pub taint_count: u32,
    /// Number of critical taint findings
    pub taint_critical: u32,
    /// Number of resource leak findings
    pub leak_count: u32,
    /// Number of bounds/overflow warnings
    pub bounds_warnings: u32,
    /// Number of missing contracts
    pub missing_contracts: u32,
    /// Number of mutable parameter issues
    pub mutable_params: u32,
    /// Number of Rust unsafe blocks
    #[serde(default)]
    pub unsafe_blocks: u32,
    /// Number of Rust raw pointer operations
    #[serde(default)]
    pub raw_pointer_ops: u32,
    /// Number of Rust unwrap calls
    #[serde(default)]
    pub unwrap_calls: u32,
    /// Number of todo!/unimplemented! markers in non-test code
    #[serde(default)]
    pub todo_markers: u32,
}

/// Secure analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureReport {
    /// Command identifier
    pub wrapper: String,
    /// Path that was analyzed
    pub path: String,
    /// Security findings sorted by severity
    pub findings: Vec<SecureFinding>,
    /// Summary statistics
    pub summary: SecureSummary,
    /// Raw results from sub-analyses (when --detail is used)
    #[serde(default)]
    pub sub_results: HashMap<String, Value>,
    /// Total elapsed time in milliseconds
    pub total_elapsed_ms: f64,
}

impl SecureReport {
    /// Create a new SecureReport
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            wrapper: "secure".to_string(),
            path: path.into(),
            findings: Vec::new(),
            summary: SecureSummary::default(),
            sub_results: HashMap::new(),
            total_elapsed_ms: 0.0,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_serialization() {
        let json = serde_json::to_string(&OutputFormat::Json).unwrap();
        assert_eq!(json, r#""json""#);

        let text = serde_json::to_string(&OutputFormat::Text).unwrap();
        assert_eq!(text, r#""text""#);

        let sarif = serde_json::to_string(&OutputFormat::Sarif).unwrap();
        assert_eq!(sarif, r#""sarif""#);
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical.order() < Severity::High.order());
        assert!(Severity::High.order() < Severity::Medium.order());
        assert!(Severity::Medium.order() < Severity::Low.order());
        assert!(Severity::Low.order() < Severity::Info.order());
    }

    #[test]
    fn test_location_serialization() {
        let loc = Location::new("test.py", 42);
        let json = serde_json::to_string(&loc).unwrap();
        assert!(json.contains(r#""file":"test.py""#));
        assert!(json.contains(r#""line":42"#));
    }

    #[test]
    fn test_todo_report_serialization() {
        let mut report = TodoReport::new("/path/to/file.py");
        report
            .items
            .push(TodoItem::new("dead_code", 1, "Unused function"));
        report.summary.dead_count = 1;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""wrapper":"todo""#));
        assert!(json.contains(r#""dead_count":1"#));
    }

    #[test]
    fn test_todo_item_builder() {
        let item = TodoItem::new("complexity", 2, "High cyclomatic complexity")
            .with_location("src/main.py", 100)
            .with_severity("high")
            .with_score(0.85);

        assert_eq!(item.category, "complexity");
        assert_eq!(item.file, "src/main.py");
        assert_eq!(item.line, 100);
        assert_eq!(item.severity, "high");
        assert!((item.score - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_secure_report_serialization() {
        let mut report = SecureReport::new("/path/to/file.py");
        report
            .findings
            .push(SecureFinding::new("taint", "critical", "SQL injection"));
        report.summary.taint_count = 1;
        report.summary.taint_critical = 1;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""wrapper":"secure""#));
        assert!(json.contains(r#""taint_count":1"#));
        assert!(json.contains(r#""taint_critical":1"#));
    }

    #[test]
    fn test_secure_finding_builder() {
        let finding = SecureFinding::new("resource_leak", "high", "File not closed")
            .with_location("src/db.py", 42);

        assert_eq!(finding.category, "resource_leak");
        assert_eq!(finding.severity, "high");
        assert_eq!(finding.file, "src/db.py");
        assert_eq!(finding.line, 42);
    }
}

// =============================================================================
// Explain Types
// =============================================================================

/// Parameter information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamInfo {
    /// Parameter name
    pub name: String,
    /// Type hint if present
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
    /// Default value if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl ParamInfo {
    /// Create a new ParamInfo
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_hint: None,
            default: None,
        }
    }

    /// Set type hint
    pub fn with_type(mut self, type_hint: impl Into<String>) -> Self {
        self.type_hint = Some(type_hint.into());
        self
    }

    /// Set default value
    pub fn with_default(mut self, default: impl Into<String>) -> Self {
        self.default = Some(default.into());
        self
    }
}

/// Function signature information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureInfo {
    /// Parameters
    pub params: Vec<ParamInfo>,
    /// Return type if annotated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Decorators
    #[serde(default)]
    pub decorators: Vec<String>,
    /// Whether the function is async
    #[serde(default)]
    pub is_async: bool,
    /// Docstring content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
}

impl SignatureInfo {
    /// Create a new SignatureInfo
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            return_type: None,
            decorators: Vec::new(),
            is_async: false,
            docstring: None,
        }
    }

    /// Add a parameter
    pub fn with_param(mut self, param: ParamInfo) -> Self {
        self.params.push(param);
        self
    }

    /// Set return type
    pub fn with_return_type(mut self, return_type: impl Into<String>) -> Self {
        self.return_type = Some(return_type.into());
        self
    }

    /// Set docstring
    pub fn with_docstring(mut self, docstring: impl Into<String>) -> Self {
        self.docstring = Some(docstring.into());
        self
    }

    /// Set async flag
    pub fn set_async(mut self, is_async: bool) -> Self {
        self.is_async = is_async;
        self
    }
}

impl Default for SignatureInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Purity analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurityInfo {
    /// Classification: "pure", "impure", or "unknown"
    pub classification: String,
    /// List of detected effects (empty if pure)
    #[serde(default)]
    pub effects: Vec<String>,
    /// Confidence: "high", "medium", or "low"
    pub confidence: String,
}

impl PurityInfo {
    /// Create a pure classification
    pub fn pure() -> Self {
        Self {
            classification: "pure".to_string(),
            effects: Vec::new(),
            confidence: "high".to_string(),
        }
    }

    /// Create an impure classification
    pub fn impure(effects: Vec<String>) -> Self {
        Self {
            classification: "impure".to_string(),
            effects,
            confidence: "high".to_string(),
        }
    }

    /// Create an unknown classification
    pub fn unknown() -> Self {
        Self {
            classification: "unknown".to_string(),
            effects: Vec::new(),
            confidence: "low".to_string(),
        }
    }

    /// Set confidence level
    pub fn with_confidence(mut self, confidence: impl Into<String>) -> Self {
        self.confidence = confidence.into();
        self
    }
}

impl Default for PurityInfo {
    fn default() -> Self {
        Self::unknown()
    }
}

/// Complexity metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityInfo {
    /// Cyclomatic complexity
    pub cyclomatic: u32,
    /// Number of basic blocks
    pub num_blocks: u32,
    /// Number of edges in CFG
    pub num_edges: u32,
    /// Whether the function contains loops
    pub has_loops: bool,
}

impl ComplexityInfo {
    /// Create new complexity info
    pub fn new(cyclomatic: u32, num_blocks: u32, num_edges: u32, has_loops: bool) -> Self {
        Self {
            cyclomatic,
            num_blocks,
            num_edges,
            has_loops,
        }
    }
}

impl Default for ComplexityInfo {
    fn default() -> Self {
        Self {
            cyclomatic: 1,
            num_blocks: 1,
            num_edges: 0,
            has_loops: false,
        }
    }
}

/// Caller/callee information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallInfo {
    /// Function name
    pub name: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: u32,
}

impl CallInfo {
    /// Create new call info
    pub fn new(name: impl Into<String>, file: impl Into<String>, line: u32) -> Self {
        Self {
            name: name.into(),
            file: file.into(),
            line,
        }
    }
}

/// Full explain report for a function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainReport {
    /// Function name
    pub function_name: String,
    /// File path
    pub file: String,
    /// Start line of function
    pub line_start: u32,
    /// End line of function
    pub line_end: u32,
    /// Detected language
    pub language: String,
    /// Signature information
    pub signature: SignatureInfo,
    /// Purity analysis
    pub purity: PurityInfo,
    /// Complexity metrics (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<ComplexityInfo>,
    /// Functions that call this function
    #[serde(default)]
    pub callers: Vec<CallInfo>,
    /// Functions called by this function
    #[serde(default)]
    pub callees: Vec<CallInfo>,
}

impl ExplainReport {
    /// Create a new ExplainReport
    pub fn new(
        function_name: impl Into<String>,
        file: impl Into<String>,
        line_start: u32,
        line_end: u32,
        language: impl Into<String>,
    ) -> Self {
        Self {
            function_name: function_name.into(),
            file: file.into(),
            line_start,
            line_end,
            language: language.into(),
            signature: SignatureInfo::default(),
            purity: PurityInfo::default(),
            complexity: None,
            callers: Vec::new(),
            callees: Vec::new(),
        }
    }

    /// Set signature
    pub fn with_signature(mut self, signature: SignatureInfo) -> Self {
        self.signature = signature;
        self
    }

    /// Set purity
    pub fn with_purity(mut self, purity: PurityInfo) -> Self {
        self.purity = purity;
        self
    }

    /// Set complexity
    pub fn with_complexity(mut self, complexity: ComplexityInfo) -> Self {
        self.complexity = Some(complexity);
        self
    }

    /// Add a caller
    pub fn add_caller(&mut self, caller: CallInfo) {
        self.callers.push(caller);
    }

    /// Add a callee
    pub fn add_callee(&mut self, callee: CallInfo) {
        self.callees.push(callee);
    }
}

// =============================================================================
// Definition Types
// =============================================================================

/// Symbol kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Class,
    Method,
    Variable,
    Parameter,
    Constant,
    Module,
    Type,
    Interface,
    Property,
    #[default]
    Unknown,
}

/// Symbol information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolInfo {
    /// Symbol name
    pub name: String,
    /// Symbol kind
    pub kind: SymbolKind,
    /// Location in source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Type annotation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_annotation: Option<String>,
    /// Docstring
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Whether this is a builtin
    #[serde(default)]
    pub is_builtin: bool,
    /// Module path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl SymbolInfo {
    /// Create a new SymbolInfo
    pub fn new(name: impl Into<String>, kind: SymbolKind) -> Self {
        Self {
            name: name.into(),
            kind,
            location: None,
            type_annotation: None,
            docstring: None,
            is_builtin: false,
            module: None,
        }
    }
}

/// Definition lookup result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionResult {
    /// Symbol information
    pub symbol: SymbolInfo,
    /// Definition location
    #[serde(skip_serializing_if = "Option::is_none")]
    pub definition: Option<Location>,
    /// Type definition location (for types)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_definition: Option<Location>,
}

// =============================================================================
// Diff Types
// =============================================================================

/// Type of AST change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Insert,
    Delete,
    Update,
    Move,
    Rename,
    Extract,
    Inline,
    Format,
}

/// Diff granularity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffGranularity {
    /// Token-level diff (L1)
    Token,
    /// Expression-level diff (L2)
    Expression,
    /// Statement-level diff (L3)
    Statement,
    /// Function-level diff (L4) - default
    #[default]
    Function,
    /// Class-level diff (L5)
    Class,
    /// File-level diff (L6)
    File,
    /// Module-level diff (L7)
    Module,
    /// Architecture-level diff (L8)
    Architecture,
}

/// Base class changes for class-level diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseChanges {
    /// Base classes added
    pub added: Vec<String>,
    /// Base classes removed
    pub removed: Vec<String>,
}

/// Kind of AST node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Function,
    Class,
    Method,
    Field,
    Statement,
    Expression,
    Block,
}

/// A single AST change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ASTChange {
    /// Type of change
    pub change_type: ChangeType,
    /// Kind of node changed
    pub node_kind: NodeKind,
    /// Name of the changed element
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Old location
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_location: Option<Location>,
    /// New location
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_location: Option<Location>,
    /// Old text (for updates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    /// New text (for updates)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    /// Similarity score (for moves/renames)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f64>,
    /// Nested member changes (for class-level diff)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<ASTChange>>,
    /// Base class changes (for class-level diff)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_changes: Option<BaseChanges>,
}

/// Diff summary statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffSummary {
    /// Total changes
    pub total_changes: u32,
    /// Semantic changes (excluding format)
    pub semantic_changes: u32,
    /// Number of inserts
    pub inserts: u32,
    /// Number of deletes
    pub deletes: u32,
    /// Number of updates
    pub updates: u32,
    /// Number of moves
    pub moves: u32,
    /// Number of renames
    pub renames: u32,
    /// Number of format-only changes
    pub formats: u32,
    /// Number of extracts
    pub extracts: u32,
}

// =============================================================================
// L6: File-Level Types
// =============================================================================

/// L6: File-level structural fingerprint change.
///
/// Represents a single file's structural change between two directory snapshots.
/// The fingerprint is a hash of sorted function/class signatures, so two files
/// with the same structure but different formatting will produce the same hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLevelChange {
    /// Relative path of the file within the compared directory
    pub relative_path: String,
    /// Type of change (Insert=added, Delete=removed, Update=modified)
    pub change_type: ChangeType,
    /// Structural fingerprint of the file in dir_a (None if added)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_fingerprint: Option<u64>,
    /// Structural fingerprint of the file in dir_b (None if removed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_fingerprint: Option<u64>,
    /// Which signatures changed (only for Update)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_changes: Option<Vec<String>>,
}

// =============================================================================
// L7: Module-Level Types
// =============================================================================

/// L7: An import edge in the module dependency graph.
///
/// Represents a single `from X import Y` or `import X` statement,
/// capturing the source file, target module, and imported names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEdge {
    /// Source file that contains the import statement
    pub source_file: String,
    /// Target module being imported from
    pub target_module: String,
    /// Specific names imported (empty for `import X`)
    pub imported_names: Vec<String>,
}

/// L7: Module-level change combining import graph and structural diffs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleLevelChange {
    /// Module path (relative file path)
    pub module_path: String,
    /// Type of change at the module level
    pub change_type: ChangeType,
    /// Import edges added in dir_b
    pub imports_added: Vec<ImportEdge>,
    /// Import edges removed from dir_a
    pub imports_removed: Vec<ImportEdge>,
    /// L6 file-level change data (if structure also changed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_change: Option<FileLevelChange>,
}

/// L7: Summary of import graph differences between two directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportGraphSummary {
    /// Total import edges in dir_a
    pub total_edges_a: usize,
    /// Total import edges in dir_b
    pub total_edges_b: usize,
    /// Number of edges added (present in B but not A)
    pub edges_added: usize,
    /// Number of edges removed (present in A but not B)
    pub edges_removed: usize,
    /// Number of modules whose import set changed
    pub modules_with_import_changes: usize,
}

// =============================================================================
// L8: Architecture-Level Types
// =============================================================================

/// L8: Type of architectural change detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchChangeType {
    /// A directory migrated from one architectural layer to another
    LayerMigration,
    /// A new directory/layer was added
    Added,
    /// A directory/layer was removed
    Removed,
    /// The composition of a directory changed significantly
    CompositionChanged,
    /// A new dependency cycle was introduced
    CycleIntroduced,
    /// An existing dependency cycle was resolved
    CycleResolved,
}

/// L8: Architecture-level change for a single directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchLevelChange {
    /// Directory path (relative)
    pub directory: String,
    /// Type of architectural change
    pub change_type: ArchChangeType,
    /// Previous layer classification (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_layer: Option<String>,
    /// New layer classification (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_layer: Option<String>,
    /// Functions that migrated between layers
    #[serde(default)]
    pub migrated_functions: Vec<String>,
}

/// L8: Summary of architecture-level differences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchDiffSummary {
    /// Number of directories that migrated between layers
    pub layer_migrations: usize,
    /// Number of new directories added
    pub directories_added: usize,
    /// Number of directories removed
    pub directories_removed: usize,
    /// Number of new dependency cycles introduced
    pub cycles_introduced: usize,
    /// Number of dependency cycles resolved
    pub cycles_resolved: usize,
    /// Overall stability score (1.0 = no changes, 0.0 = everything changed)
    pub stability_score: f64,
}

// =============================================================================
// Diff Report
// =============================================================================

/// Diff report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffReport {
    /// First file
    pub file_a: String,
    /// Second file
    pub file_b: String,
    /// Whether files are identical
    pub identical: bool,
    /// List of changes
    pub changes: Vec<ASTChange>,
    /// Summary statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<DiffSummary>,
    /// Granularity level of this diff
    #[serde(default)]
    pub granularity: DiffGranularity,
    /// L6: File-level structural changes (directory diff)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_changes: Option<Vec<FileLevelChange>>,
    /// L7: Module-level changes with import graph diff
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_changes: Option<Vec<ModuleLevelChange>>,
    /// L7: Import graph summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_graph_summary: Option<ImportGraphSummary>,
    /// L8: Architecture-level changes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch_changes: Option<Vec<ArchLevelChange>>,
    /// L8: Architecture diff summary
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch_summary: Option<ArchDiffSummary>,
}

impl DiffReport {
    /// Create a new DiffReport
    pub fn new(file_a: impl Into<String>, file_b: impl Into<String>) -> Self {
        Self {
            file_a: file_a.into(),
            file_b: file_b.into(),
            identical: true,
            changes: Vec::new(),
            summary: Some(DiffSummary::default()),
            granularity: DiffGranularity::Function,
            file_changes: None,
            module_changes: None,
            import_graph_summary: None,
            arch_changes: None,
            arch_summary: None,
        }
    }
}

// =============================================================================
// Diff Impact Types
// =============================================================================

/// A function affected by changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFunction {
    /// Function name
    pub name: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: u32,
    /// Functions that call this function
    #[serde(default)]
    pub callers: Vec<CallInfo>,
}

/// Diff impact summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiffImpactSummary {
    /// Number of files changed
    pub files_changed: u32,
    /// Number of functions changed
    pub functions_changed: u32,
    /// Number of tests to run
    pub tests_to_run: u32,
}

/// Diff impact report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffImpactReport {
    /// Changed functions
    pub changed_functions: Vec<ChangedFunction>,
    /// Suggested tests to run
    pub suggested_tests: Vec<String>,
    /// Summary
    pub summary: DiffImpactSummary,
}

impl DiffImpactReport {
    /// Create a new DiffImpactReport
    pub fn new() -> Self {
        Self {
            changed_functions: Vec::new(),
            suggested_tests: Vec::new(),
            summary: DiffImpactSummary::default(),
        }
    }
}

impl Default for DiffImpactReport {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// API Check Types
// =============================================================================

/// Categories of API misuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MisuseCategory {
    CallOrder,
    ErrorHandling,
    Parameters,
    Resources,
    Crypto,
    Concurrency,
    Security,
}

/// Severity of API misuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum MisuseSeverity {
    Info,
    Low,
    Medium,
    High,
}

/// An API rule definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APIRule {
    /// Rule identifier
    pub id: String,
    /// Rule name
    pub name: String,
    /// Category
    pub category: MisuseCategory,
    /// Severity
    pub severity: MisuseSeverity,
    /// Description
    pub description: String,
    /// Example of correct usage
    pub correct_usage: String,
}

/// A detected API misuse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisuseFinding {
    /// File path
    pub file: String,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
    /// Rule that was violated
    pub rule: APIRule,
    /// The API call that violated the rule
    pub api_call: String,
    /// Human-readable message
    pub message: String,
    /// Suggested fix
    pub fix_suggestion: String,
    /// Code context
    #[serde(default)]
    pub code_context: String,
}

/// API check summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct APICheckSummary {
    /// Total findings
    pub total_findings: u32,
    /// Findings by category
    #[serde(default)]
    pub by_category: HashMap<String, u32>,
    /// Findings by severity
    #[serde(default)]
    pub by_severity: HashMap<String, u32>,
    /// APIs that were checked
    #[serde(default)]
    pub apis_checked: Vec<String>,
    /// Number of files scanned
    pub files_scanned: u32,
}

/// API check report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct APICheckReport {
    /// Findings
    pub findings: Vec<MisuseFinding>,
    /// Summary
    pub summary: APICheckSummary,
    /// Number of rules applied
    pub rules_applied: u32,
}

impl APICheckReport {
    /// Create a new APICheckReport
    pub fn new() -> Self {
        Self {
            findings: Vec::new(),
            summary: APICheckSummary::default(),
            rules_applied: 0,
        }
    }
}

impl Default for APICheckReport {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Equivalence (GVN) Types
// =============================================================================

/// Reference to an expression in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionRef {
    /// Expression text
    pub text: String,
    /// Line number
    pub line: u32,
    /// Value number (GVN)
    pub value_number: u32,
}

/// A group of expressions sharing the same value number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNEquivalence {
    /// Value number for this group
    pub value_number: u32,
    /// Expressions in this equivalence class
    pub expressions: Vec<ExpressionRef>,
    /// Reason for equivalence
    #[serde(default)]
    pub reason: String,
}

/// A redundant expression pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redundancy {
    /// Original expression
    pub original: ExpressionRef,
    /// Redundant expression
    pub redundant: ExpressionRef,
    /// Reason why it's redundant
    #[serde(default)]
    pub reason: String,
}

/// GVN summary statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNSummary {
    /// Total expressions analyzed
    pub total_expressions: u32,
    /// Unique values (value numbers)
    pub unique_values: u32,
    /// Compression ratio (unique/total)
    pub compression_ratio: f64,
}

impl Default for GVNSummary {
    fn default() -> Self {
        Self {
            total_expressions: 0,
            unique_values: 0,
            compression_ratio: 1.0,
        }
    }
}

/// GVN report for a function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GVNReport {
    /// Function name
    pub function: String,
    /// Equivalence classes
    #[serde(default)]
    pub equivalences: Vec<GVNEquivalence>,
    /// Redundant expressions
    #[serde(default)]
    pub redundancies: Vec<Redundancy>,
    /// Summary statistics
    pub summary: GVNSummary,
}

impl GVNReport {
    /// Create a new GVNReport
    pub fn new(function: impl Into<String>) -> Self {
        Self {
            function: function.into(),
            equivalences: Vec::new(),
            redundancies: Vec::new(),
            summary: GVNSummary::default(),
        }
    }
}

// =============================================================================
// Vuln Types
// =============================================================================

/// Types of vulnerabilities detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum VulnType {
    SqlInjection,
    Xss,
    CommandInjection,
    Ssrf,
    PathTraversal,
    Deserialization,
    UnsafeCode,
    MemorySafety,
    Panic,
    Xxe,
    OpenRedirect,
    LdapInjection,
    XpathInjection,
}

impl VulnType {
    /// Get the CWE identifier for this vulnerability type.
    pub fn cwe_id(&self) -> &'static str {
        match self {
            Self::SqlInjection => "CWE-89",
            Self::Xss => "CWE-79",
            Self::CommandInjection => "CWE-78",
            Self::Ssrf => "CWE-918",
            Self::PathTraversal => "CWE-22",
            Self::Deserialization => "CWE-502",
            Self::UnsafeCode => "CWE-242",
            Self::MemorySafety => "CWE-119",
            Self::Panic => "CWE-703",
            Self::Xxe => "CWE-611",
            Self::OpenRedirect => "CWE-601",
            Self::LdapInjection => "CWE-90",
            Self::XpathInjection => "CWE-643",
        }
    }

    /// Get the default severity for this vulnerability type.
    pub fn default_severity(&self) -> Severity {
        match self {
            Self::SqlInjection
            | Self::CommandInjection
            | Self::Deserialization
            | Self::MemorySafety => Severity::Critical,
            Self::Xxe
            | Self::Xss
            | Self::Ssrf
            | Self::PathTraversal
            | Self::LdapInjection
            | Self::XpathInjection
            | Self::UnsafeCode => Severity::High,
            Self::OpenRedirect | Self::Panic => Severity::Medium,
        }
    }
}

/// A step in the taint flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintFlow {
    /// File path
    pub file: String,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
    /// Code snippet
    pub code_snippet: String,
    /// Description of this step
    pub description: String,
}

/// A vulnerability finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    /// Vulnerability type
    pub vuln_type: VulnType,
    /// Severity
    pub severity: Severity,
    /// CWE identifier
    pub cwe_id: String,
    /// Title
    pub title: String,
    /// Description
    pub description: String,
    /// File path
    pub file: String,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
    /// Taint flow from source to sink
    pub taint_flow: Vec<TaintFlow>,
    /// Remediation advice
    pub remediation: String,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
}

/// Vulnerability summary statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VulnSummary {
    /// Total findings
    pub total_findings: u32,
    /// Findings by severity
    #[serde(default)]
    pub by_severity: HashMap<String, u32>,
    /// Findings by type
    #[serde(default)]
    pub by_type: HashMap<String, u32>,
    /// Files with vulnerabilities
    pub files_with_vulns: u32,
}

/// Vulnerability report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnReport {
    /// Findings
    pub findings: Vec<VulnFinding>,
    /// Summary (optional for incremental results)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<VulnSummary>,
    /// Scan duration in milliseconds
    pub scan_duration_ms: u64,
    /// Number of files scanned
    pub files_scanned: u32,
}

impl VulnReport {
    /// Create a new VulnReport
    pub fn new() -> Self {
        Self {
            findings: Vec::new(),
            summary: None,
            scan_duration_ms: 0,
            files_scanned: 0,
        }
    }
}

impl Default for VulnReport {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Unit Tests for Types
// =============================================================================

#[cfg(test)]
mod unit_types_tests {
    use super::*;

    // =========================================================================
    // Output Format Tests
    // =========================================================================

    #[test]
    fn test_output_format_serialization() {
        let json = serde_json::to_string(&OutputFormat::Json).unwrap();
        assert_eq!(json, r#""json""#);

        let text = serde_json::to_string(&OutputFormat::Text).unwrap();
        assert_eq!(text, r#""text""#);

        let sarif = serde_json::to_string(&OutputFormat::Sarif).unwrap();
        assert_eq!(sarif, r#""sarif""#);
    }

    #[test]
    fn test_output_format_deserialization() {
        let json: OutputFormat = serde_json::from_str(r#""json""#).unwrap();
        assert_eq!(json, OutputFormat::Json);

        let text: OutputFormat = serde_json::from_str(r#""text""#).unwrap();
        assert_eq!(text, OutputFormat::Text);
    }

    // =========================================================================
    // Severity Tests
    // =========================================================================

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical.order() < Severity::High.order());
        assert!(Severity::High.order() < Severity::Medium.order());
        assert!(Severity::Medium.order() < Severity::Low.order());
        assert!(Severity::Low.order() < Severity::Info.order());
    }

    #[test]
    fn test_severity_serialization() {
        let critical = serde_json::to_string(&Severity::Critical).unwrap();
        assert_eq!(critical, r#""critical""#);

        let info = serde_json::to_string(&Severity::Info).unwrap();
        assert_eq!(info, r#""info""#);
    }

    // =========================================================================
    // Location Tests
    // =========================================================================

    #[test]
    fn test_location_serialization() {
        let loc = Location::new("test.py", 42);
        let json = serde_json::to_string(&loc).unwrap();
        assert!(json.contains(r#""file":"test.py""#));
        assert!(json.contains(r#""line":42"#));
    }

    #[test]
    fn test_location_with_column() {
        let loc = Location::with_column("test.py", 42, 10);
        assert_eq!(loc.column, 10);
    }

    // =========================================================================
    // Todo Types Tests
    // =========================================================================

    #[test]
    fn test_todo_report_serialization() {
        let mut report = TodoReport::new("/path/to/file.py");
        report
            .items
            .push(TodoItem::new("dead_code", 1, "Unused function"));
        report.summary.dead_count = 1;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""wrapper":"todo""#));
        assert!(json.contains(r#""dead_count":1"#));
    }

    #[test]
    fn test_todo_item_builder() {
        let item = TodoItem::new("complexity", 2, "High cyclomatic complexity")
            .with_location("src/main.py", 100)
            .with_severity("high")
            .with_score(0.85);

        assert_eq!(item.category, "complexity");
        assert_eq!(item.file, "src/main.py");
        assert_eq!(item.line, 100);
        assert_eq!(item.severity, "high");
        assert!((item.score - 0.85).abs() < 0.001);
    }

    // =========================================================================
    // Explain Types Tests
    // =========================================================================

    #[test]
    fn test_explain_report_serialization() {
        let mut report = ExplainReport::new("calculate_total", "/path/file.py", 10, 20, "python");
        report.purity = PurityInfo::pure();

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""function_name":"calculate_total""#));
        assert!(json.contains(r#""classification":"pure""#));
    }

    #[test]
    fn test_signature_info_builder() {
        let sig = SignatureInfo::new()
            .with_param(ParamInfo::new("x").with_type("int"))
            .with_return_type("int")
            .with_docstring("Doubles the input");

        assert_eq!(sig.params.len(), 1);
        assert_eq!(sig.params[0].name, "x");
        assert_eq!(sig.return_type.unwrap(), "int");
    }

    // =========================================================================
    // Secure Types Tests
    // =========================================================================

    #[test]
    fn test_secure_report_serialization() {
        let mut report = SecureReport::new("/path/to/file.py");
        report
            .findings
            .push(SecureFinding::new("taint", "critical", "SQL injection"));

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""wrapper":"secure""#));
    }

    // =========================================================================
    // Definition Types Tests
    // =========================================================================

    #[test]
    fn test_definition_result_serialization() {
        let result = DefinitionResult {
            symbol: SymbolInfo::new("my_func", SymbolKind::Function),
            definition: Some(Location::new("file.py", 10)),
            type_definition: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""name":"my_func""#));
        assert!(json.contains(r#""kind":"function""#));
    }

    #[test]
    fn test_symbol_kind_serialization() {
        let kind = SymbolKind::Function;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, r#""function""#);
    }

    // =========================================================================
    // Diff Types Tests
    // =========================================================================

    #[test]
    fn test_diff_report_serialization() {
        let mut report = DiffReport::new("a.py", "b.py");
        report.identical = false;
        if let Some(ref mut summary) = report.summary {
            summary.inserts = 1;
        }

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""file_a":"a.py""#));
        assert!(json.contains(r#""identical":false"#));
    }

    #[test]
    fn test_change_type_serialization() {
        let insert = serde_json::to_string(&ChangeType::Insert).unwrap();
        assert_eq!(insert, r#""insert""#);

        let rename = serde_json::to_string(&ChangeType::Rename).unwrap();
        assert_eq!(rename, r#""rename""#);
    }

    // =========================================================================
    // API Check Types Tests
    // =========================================================================

    #[test]
    fn test_api_check_report_serialization() {
        let mut report = APICheckReport::new();
        report.rules_applied = 5;
        report.summary.total_findings = 2;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""rules_applied":5"#));
        assert!(json.contains(r#""total_findings":2"#));
    }

    // =========================================================================
    // GVN Types Tests
    // =========================================================================

    #[test]
    fn test_gvn_report_serialization() {
        let mut report = GVNReport::new("test_func");
        report.summary.total_expressions = 10;
        report.summary.unique_values = 7;
        report.summary.compression_ratio = 0.7;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""function":"test_func""#));
        assert!(json.contains(r#""compression_ratio":0.7"#));
    }

    // =========================================================================
    // Vuln Types Tests
    // =========================================================================

    #[test]
    fn test_vuln_report_serialization() {
        let mut report = VulnReport::new();
        report.files_scanned = 5;
        report.scan_duration_ms = 100;

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""files_scanned":5"#));
        assert!(json.contains(r#""scan_duration_ms":100"#));
    }

    #[test]
    fn test_vuln_type_cwe_mapping() {
        assert_eq!(VulnType::SqlInjection.cwe_id(), "CWE-89");
        assert_eq!(VulnType::Xss.cwe_id(), "CWE-79");
        assert_eq!(VulnType::CommandInjection.cwe_id(), "CWE-78");
        assert_eq!(VulnType::Ssrf.cwe_id(), "CWE-918");
        assert_eq!(VulnType::PathTraversal.cwe_id(), "CWE-22");
        assert_eq!(VulnType::Deserialization.cwe_id(), "CWE-502");
        assert_eq!(VulnType::UnsafeCode.cwe_id(), "CWE-242");
        assert_eq!(VulnType::MemorySafety.cwe_id(), "CWE-119");
        assert_eq!(VulnType::Panic.cwe_id(), "CWE-703");
        assert_eq!(VulnType::Xxe.cwe_id(), "CWE-611");
        assert_eq!(VulnType::OpenRedirect.cwe_id(), "CWE-601");
        assert_eq!(VulnType::LdapInjection.cwe_id(), "CWE-90");
        assert_eq!(VulnType::XpathInjection.cwe_id(), "CWE-643");
    }

    #[test]
    fn test_vuln_type_default_severity() {
        assert_eq!(
            VulnType::SqlInjection.default_severity(),
            Severity::Critical
        );
        assert_eq!(
            VulnType::CommandInjection.default_severity(),
            Severity::Critical
        );
        assert_eq!(
            VulnType::MemorySafety.default_severity(),
            Severity::Critical
        );
        assert_eq!(VulnType::Xss.default_severity(), Severity::High);
        assert_eq!(VulnType::UnsafeCode.default_severity(), Severity::High);
        assert_eq!(VulnType::OpenRedirect.default_severity(), Severity::Medium);
        assert_eq!(VulnType::Panic.default_severity(), Severity::Medium);
    }

    #[test]
    fn test_vuln_type_serialization() {
        let sql_inj = serde_json::to_string(&VulnType::SqlInjection).unwrap();
        assert_eq!(sql_inj, r#""sql_injection""#);

        let xss = serde_json::to_string(&VulnType::Xss).unwrap();
        assert_eq!(xss, r#""xss""#);
    }
}
