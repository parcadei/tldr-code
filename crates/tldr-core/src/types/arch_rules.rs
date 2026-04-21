//! Architecture rules types for layer constraints and violation checking
//!
//! This module defines types for the `arch --generate-rules` and `arch --check-rules`
//! commands (Phase 3).
//! Addresses blockers: A11 (ArchRule/ViolationReport missing)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::LayerType;

// =============================================================================
// Architecture Rules (A11)
// =============================================================================

/// A single architecture rule
///
/// Rules can be layer constraints (e.g., "LOW may not import HIGH")
/// or cycle-break recommendations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchRule {
    /// Unique rule identifier (e.g., "L1", "C1")
    pub id: String,
    /// Human-readable constraint description
    pub constraint: String,
    /// Type of rule
    pub rule_type: ArchRuleType,
    /// Source layers (for layer rules)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from_layers: Vec<String>,
    /// Target layers (forbidden for layer rules)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to_layers: Vec<String>,
    /// Specific files (for cycle-break rules)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// Severity of violation
    pub severity: RuleSeverity,
    /// Rationale for the rule
    pub rationale: String,
}

impl ArchRule {
    /// Create a layer constraint rule
    pub fn layer(
        id: impl Into<String>,
        constraint: impl Into<String>,
        from_layers: Vec<String>,
        to_layers: Vec<String>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            constraint: constraint.into(),
            rule_type: ArchRuleType::Layer,
            from_layers,
            to_layers,
            files: Vec::new(),
            severity: RuleSeverity::Error,
            rationale: rationale.into(),
        }
    }

    /// Create a cycle-break rule
    pub fn cycle_break(
        id: impl Into<String>,
        constraint: impl Into<String>,
        files: Vec<String>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            constraint: constraint.into(),
            rule_type: ArchRuleType::CycleBreak,
            from_layers: Vec::new(),
            to_layers: Vec::new(),
            files,
            severity: RuleSeverity::Warn,
            rationale: rationale.into(),
        }
    }

    /// Set severity
    pub fn with_severity(mut self, severity: RuleSeverity) -> Self {
        self.severity = severity;
        self
    }
}

/// Type of architecture rule
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArchRuleType {
    /// Layer constraint (e.g., LOW may not import HIGH)
    Layer,
    /// Cycle break recommendation
    CycleBreak,
}

/// Severity level for rule violations
///
/// Note: This is separate from the security::Severity to avoid confusion
/// and allow different semantics for architecture rules.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleSeverity {
    /// Error - violation should block CI
    Error,
    /// Warning - violation should be reviewed
    Warn,
}

impl std::fmt::Display for RuleSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleSeverity::Error => write!(f, "error"),
            RuleSeverity::Warn => write!(f, "warn"),
        }
    }
}

// =============================================================================
// Architecture Rules File
// =============================================================================

/// Complete architecture rules file structure
///
/// Corresponds to the YAML format:
/// ```yaml
/// version: "1.0"
/// generated_at: "2026-02-02T10:00:00Z"
/// layers:
///   high: { directories: [...], description: "..." }
/// rules:
///   - id: L1
///     constraint: "..."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchRulesFile {
    /// Format version
    pub version: String,
    /// Generation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// Layer definitions
    pub layers: LayerDefinitions,
    /// Architecture rules
    pub rules: Vec<ArchRule>,
}

impl ArchRulesFile {
    /// Create a new rules file
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            generated_at: None,
            layers: LayerDefinitions::default(),
            rules: Vec::new(),
        }
    }

    /// Set the generation timestamp
    pub fn with_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.generated_at = Some(timestamp.into());
        self
    }

    /// Add a rule
    pub fn with_rule(mut self, rule: ArchRule) -> Self {
        self.rules.push(rule);
        self
    }
}

impl Default for ArchRulesFile {
    fn default() -> Self {
        Self::new()
    }
}

/// Layer definitions in the rules file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LayerDefinitions {
    /// High layer (Entry/Controller)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<LayerDefinition>,
    /// Middle layer (Service/Business)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle: Option<LayerDefinition>,
    /// Low layer (Utility/Data)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<LayerDefinition>,
}

/// Definition of a single layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDefinition {
    /// Description of the layer
    pub description: String,
    /// Directories belonging to this layer
    pub directories: Vec<String>,
}

impl LayerDefinition {
    /// Create a new layer definition
    pub fn new(description: impl Into<String>, directories: Vec<String>) -> Self {
        Self {
            description: description.into(),
            directories,
        }
    }
}

// =============================================================================
// Violation Report (A11)
// =============================================================================

/// Report of architecture rule violations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationReport {
    /// Whether all rules passed (no error-severity violations)
    pub pass: bool,
    /// List of violations found
    pub violations: Vec<Violation>,
    /// Summary statistics
    pub summary: ViolationSummary,
}

impl ViolationReport {
    /// Create a new empty report (passing)
    pub fn new() -> Self {
        Self {
            pass: true,
            violations: Vec::new(),
            summary: ViolationSummary::default(),
        }
    }

    /// Add a violation
    pub fn add_violation(&mut self, violation: Violation) {
        if violation.severity == RuleSeverity::Error {
            self.pass = false;
            self.summary.error_count += 1;
        } else {
            self.summary.warn_count += 1;
        }
        self.violations.push(violation);
    }

    /// Check if the report has any violations
    pub fn has_violations(&self) -> bool {
        !self.violations.is_empty()
    }
}

impl Default for ViolationReport {
    fn default() -> Self {
        Self::new()
    }
}

/// A single architecture rule violation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    /// ID of the rule that was violated
    pub rule_id: String,
    /// The constraint that was violated
    pub rule_constraint: String,
    /// File containing the violation
    pub from_file: PathBuf,
    /// Line number of the violating import
    pub from_line: u32,
    /// File being imported (that violates the rule)
    pub imports_file: PathBuf,
    /// Layer of the source file
    pub from_layer: String,
    /// Layer of the imported file
    pub to_layer: String,
    /// Severity of this violation
    pub severity: RuleSeverity,
    /// Whether this is a transitive violation (A3: Transitive violation support)
    #[serde(default)]
    pub transitive: bool,
    /// Full path for transitive violations (A3)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<PathBuf>,
}

/// Base parameters for an architecture rule violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationInfo {
    /// ID of the rule that was violated
    pub rule_id: String,
    /// The constraint that was violated
    pub rule_constraint: String,
    /// File containing the violation
    pub from_file: PathBuf,
    /// Line number of the violating import
    pub from_line: u32,
    /// File being imported (that violates the rule)
    pub imports_file: PathBuf,
    /// Layer of the source file
    pub from_layer: String,
    /// Layer of the imported file
    pub to_layer: String,
    /// Severity of this violation
    pub severity: RuleSeverity,
}

impl Violation {
    /// Create a direct (non-transitive) violation
    pub fn direct(info: ViolationInfo) -> Self {
        Self {
            rule_id: info.rule_id,
            rule_constraint: info.rule_constraint,
            from_file: info.from_file,
            from_line: info.from_line,
            imports_file: info.imports_file,
            from_layer: info.from_layer,
            to_layer: info.to_layer,
            severity: info.severity,
            transitive: false,
            path: Vec::new(),
        }
    }

    /// Create a transitive violation
    pub fn transitive(info: ViolationInfo, path: Vec<PathBuf>) -> Self {
        Self {
            rule_id: info.rule_id,
            rule_constraint: info.rule_constraint,
            from_file: info.from_file,
            from_line: info.from_line,
            imports_file: info.imports_file,
            from_layer: info.from_layer,
            to_layer: info.to_layer,
            severity: info.severity,
            transitive: true,
            path,
        }
    }
}

/// Summary of violation checking
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ViolationSummary {
    /// Number of rules checked
    pub rules_checked: usize,
    /// Number of files scanned
    pub files_scanned: usize,
    /// Number of error-severity violations
    pub error_count: usize,
    /// Number of warning-severity violations
    pub warn_count: usize,
}

// =============================================================================
// Cycle Detection Types (for Tarjan SCC)
// =============================================================================

/// A strongly connected component (cycle)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SCC {
    /// Nodes in the cycle (function or file names)
    pub nodes: Vec<String>,
    /// Size of the cycle
    pub size: usize,
    /// Edges within the cycle
    pub edges: Vec<(String, String)>,
}

impl SCC {
    /// Create a new SCC from nodes
    pub fn new(nodes: Vec<String>) -> Self {
        let size = nodes.len();
        Self {
            nodes,
            size,
            edges: Vec::new(),
        }
    }

    /// Add edges to the SCC
    pub fn with_edges(mut self, edges: Vec<(String, String)>) -> Self {
        self.edges = edges;
        self
    }
}

/// Cycle detection report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleReport {
    /// Detected cycles (SCCs with size > 1)
    pub cycles: Vec<SCC>,
    /// Summary statistics
    pub summary: CycleSummary,
    /// Granularity of the analysis
    pub granularity: CycleGranularity,
    /// Human-readable explanation
    pub explanation: String,
}

impl CycleReport {
    /// Create a new cycle report
    pub fn new(granularity: CycleGranularity) -> Self {
        Self {
            cycles: Vec::new(),
            summary: CycleSummary::default(),
            granularity,
            explanation: String::new(),
        }
    }

    /// Add a cycle and update summary
    pub fn add_cycle(&mut self, scc: SCC) {
        self.summary.total_sccs += 1;
        if scc.size > 1 {
            self.summary.cycle_count += 1;
            if scc.size > self.summary.largest_cycle {
                self.summary.largest_cycle = scc.size;
            }
        }
        self.cycles.push(scc);
    }

    /// Generate explanation text
    pub fn with_explanation(mut self) -> Self {
        let total_nodes: usize = self.cycles.iter().map(|c| c.size).sum();
        self.explanation = format!(
            "Found {} cycle(s) involving {} nodes",
            self.summary.cycle_count, total_nodes
        );
        self
    }
}

/// Cycle detection summary
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CycleSummary {
    /// Total SCCs found (including size-1)
    pub total_sccs: usize,
    /// Number of actual cycles (size > 1)
    pub cycle_count: usize,
    /// Size of the largest cycle
    pub largest_cycle: usize,
}

/// Granularity of cycle detection
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CycleGranularity {
    /// Function-level cycles
    Function,
    /// File-level cycles
    File,
}

impl std::str::FromStr for CycleGranularity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "function" | "func" | "fn" => Ok(CycleGranularity::Function),
            "file" | "module" => Ok(CycleGranularity::File),
            _ => Err(format!(
                "Invalid granularity: {}. Expected 'function' or 'file'",
                s
            )),
        }
    }
}

// =============================================================================
// Rules Generation Context (A8)
// =============================================================================

/// Context for generating architecture rules
#[derive(Debug, Clone, Default)]
pub struct RulesGenerationContext {
    /// Detected layer mappings (directory -> layer type)
    pub layer_mappings: HashMap<PathBuf, LayerType>,
    /// Detected circular dependencies
    pub circular_deps: Vec<(PathBuf, PathBuf)>,
    /// Project root
    pub project_root: PathBuf,
}

impl RulesGenerationContext {
    /// Create a new context
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            layer_mappings: HashMap::new(),
            circular_deps: Vec::new(),
            project_root,
        }
    }

    /// Add a layer mapping
    pub fn add_layer(&mut self, dir: PathBuf, layer: LayerType) {
        self.layer_mappings.insert(dir, layer);
    }

    /// Add a circular dependency
    pub fn add_circular_dep(&mut self, a: PathBuf, b: PathBuf) {
        self.circular_deps.push((a, b));
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arch_rule_layer_serialization() {
        let rule = ArchRule::layer(
            "L1",
            "LOW may not import HIGH",
            vec!["LOW".to_string()],
            vec!["HIGH".to_string()],
            "Utility layers should not depend on entry layers",
        );

        let json = serde_json::to_string_pretty(&rule).unwrap();
        assert!(json.contains("\"id\": \"L1\""));
        assert!(json.contains("\"layer\""));
        assert!(json.contains("\"error\""));

        let parsed: ArchRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "L1");
        assert_eq!(parsed.rule_type, ArchRuleType::Layer);
    }

    #[test]
    fn test_arch_rule_cycle_break_serialization() {
        let rule = ArchRule::cycle_break(
            "C1",
            "Break cycle: services/auth.py should not import api/routes.py",
            vec!["services/auth.py".to_string(), "api/routes.py".to_string()],
            "Circular dependency between services and api",
        );

        let json = serde_json::to_string(&rule).unwrap();
        let parsed: ArchRule = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "C1");
        assert_eq!(parsed.rule_type, ArchRuleType::CycleBreak);
        assert_eq!(parsed.severity, RuleSeverity::Warn);
    }

    #[test]
    fn test_arch_rules_file_yaml_compatible() {
        let mut rules_file = ArchRulesFile::new().with_timestamp("2026-02-02T10:00:00Z");

        rules_file.layers.high = Some(LayerDefinition::new(
            "Entry/Controller layer",
            vec!["api/".to_string(), "cli/".to_string()],
        ));
        rules_file.layers.low = Some(LayerDefinition::new(
            "Utility/Data layer",
            vec!["utils/".to_string()],
        ));

        rules_file = rules_file.with_rule(ArchRule::layer(
            "L1",
            "LOW may not import HIGH",
            vec!["LOW".to_string()],
            vec!["HIGH".to_string()],
            "Utility layers should not depend on entry layers",
        ));

        // Test JSON serialization
        let json = serde_json::to_string_pretty(&rules_file).unwrap();
        assert!(json.contains("\"version\": \"1.0\""));
        assert!(json.contains("\"generated_at\":"));

        let parsed: ArchRulesFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.rules.len(), 1);
    }

    #[test]
    fn test_violation_report() {
        let mut report = ViolationReport::new();
        assert!(report.pass);

        report.add_violation(Violation::direct(ViolationInfo {
            rule_id: "L1".to_string(),
            rule_constraint: "LOW may not import HIGH".to_string(),
            from_file: PathBuf::from("utils/helpers.py"),
            from_line: 5,
            imports_file: PathBuf::from("api/routes.py"),
            from_layer: "LOW".to_string(),
            to_layer: "HIGH".to_string(),
            severity: RuleSeverity::Error,
        }));

        assert!(!report.pass);
        assert_eq!(report.summary.error_count, 1);
        assert!(report.has_violations());
    }

    #[test]
    fn test_violation_transitive() {
        let violation = Violation::transitive(
            ViolationInfo {
                rule_id: "L1".to_string(),
                rule_constraint: "LOW may not import HIGH".to_string(),
                from_file: PathBuf::from("utils/a.py"),
                from_line: 10,
                imports_file: PathBuf::from("api/c.py"),
                from_layer: "LOW".to_string(),
                to_layer: "HIGH".to_string(),
                severity: RuleSeverity::Error,
            },
            vec![
                PathBuf::from("utils/a.py"),
                PathBuf::from("services/b.py"),
                PathBuf::from("api/c.py"),
            ],
        );

        assert!(violation.transitive);
        assert_eq!(violation.path.len(), 3);

        let json = serde_json::to_string(&violation).unwrap();
        // Check for both compact and pretty formats
        assert!(
            json.contains("\"transitive\":true") || json.contains("\"transitive\": true"),
            "Expected transitive:true in JSON: {}",
            json
        );
    }

    #[test]
    fn test_scc_serialization() {
        let scc = SCC::new(vec![
            "a.py:foo".to_string(),
            "b.py:bar".to_string(),
            "c.py:baz".to_string(),
        ])
        .with_edges(vec![
            ("a.py:foo".to_string(), "b.py:bar".to_string()),
            ("b.py:bar".to_string(), "c.py:baz".to_string()),
            ("c.py:baz".to_string(), "a.py:foo".to_string()),
        ]);

        let json = serde_json::to_string(&scc).unwrap();
        let parsed: SCC = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.size, 3);
        assert_eq!(parsed.edges.len(), 3);
    }

    #[test]
    fn test_cycle_report() {
        let mut report = CycleReport::new(CycleGranularity::Function);

        report.add_cycle(SCC::new(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ]));

        let report = report.with_explanation();
        assert_eq!(report.summary.cycle_count, 1);
        assert_eq!(report.summary.largest_cycle, 3);
        assert!(report.explanation.contains("1 cycle"));
    }

    #[test]
    fn test_cycle_granularity_from_str() {
        assert_eq!(
            "function".parse::<CycleGranularity>().unwrap(),
            CycleGranularity::Function
        );
        assert_eq!(
            "file".parse::<CycleGranularity>().unwrap(),
            CycleGranularity::File
        );
        assert!("invalid".parse::<CycleGranularity>().is_err());
    }
}
