//! Pattern detection types for design pattern mining
//!
//! This module defines types for the `patterns` command (Phase 4-6).
//! Addresses blockers: A10 (PatternReport type not defined)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// =============================================================================
// Pattern Report (A10)
// =============================================================================

/// Complete pattern analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternReport {
    /// Metadata about the analysis
    pub metadata: PatternMetadata,
    /// Soft delete pattern detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_delete: Option<SoftDeletePattern>,
    /// Error handling pattern detection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_handling: Option<ErrorHandlingPattern>,
    /// Naming convention patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub naming: Option<NamingPattern>,
    /// Resource management patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_management: Option<ResourceManagementPattern>,
    /// Input validation patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ValidationPattern>,
    /// Test idiom patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_idioms: Option<TestIdiomPattern>,
    /// Import organization patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import_patterns: Option<ImportPattern>,
    /// Type annotation coverage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_coverage: Option<TypeCoveragePattern>,
    /// API convention patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_conventions: Option<ApiConventionPattern>,
    /// Async/concurrency patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub async_patterns: Option<AsyncPattern>,
    /// Generated LLM constraints
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<Constraint>,
    /// Detected conflicts between patterns
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

impl PatternReport {
    /// Create a new empty pattern report
    pub fn new(metadata: PatternMetadata) -> Self {
        Self {
            metadata,
            soft_delete: None,
            error_handling: None,
            naming: None,
            resource_management: None,
            validation: None,
            test_idioms: None,
            import_patterns: None,
            type_coverage: None,
            api_conventions: None,
            async_patterns: None,
            constraints: Vec::new(),
            conflicts: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Metadata
// =============================================================================

/// Metadata about the pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMetadata {
    /// Number of files successfully analyzed
    pub files_analyzed: usize,
    /// Number of files skipped (binary, too large, etc.)
    pub files_skipped: usize,
    /// Number of files with partial results (A23: Parse error handling)
    #[serde(default)]
    pub files_partial: usize,
    /// Duration of analysis in milliseconds
    pub duration_ms: u64,
    /// Language distribution in the codebase
    pub language_distribution: LanguageDistribution,
    /// Number of patterns before confidence filter (A21: Filter metadata)
    #[serde(default)]
    pub patterns_before_filter: usize,
    /// Number of patterns after confidence filter
    #[serde(default)]
    pub patterns_after_filter: usize,
    /// Confidence threshold used for filtering
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
}

fn default_confidence_threshold() -> f64 {
    0.5
}

impl PatternMetadata {
    /// Create new metadata with basic info
    pub fn new(files_analyzed: usize, duration_ms: u64) -> Self {
        Self {
            files_analyzed,
            files_skipped: 0,
            files_partial: 0,
            duration_ms,
            language_distribution: LanguageDistribution::default(),
            patterns_before_filter: 0,
            patterns_after_filter: 0,
            confidence_threshold: default_confidence_threshold(),
        }
    }
}

/// Distribution of languages in the codebase
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanguageDistribution {
    /// Files by language
    pub files_by_language: HashMap<String, usize>,
    /// Patterns by language
    pub patterns_by_language: HashMap<String, usize>,
}

// =============================================================================
// Evidence Type (shared across patterns)
// =============================================================================

/// Evidence for a detected pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// File containing the evidence
    pub file: String,
    /// Line number
    pub line: u32,
    /// Code snippet (3-5 lines of context)
    pub snippet: String,
}

impl Evidence {
    /// Create new evidence
    pub fn new(file: impl Into<String>, line: u32, snippet: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line,
            snippet: snippet.into(),
        }
    }
}

// =============================================================================
// Pattern Category 1: Soft Delete
// =============================================================================

/// Soft delete pattern detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoftDeletePattern {
    /// Whether soft delete was detected
    pub detected: bool,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Column names used for soft delete
    pub column_names: Vec<String>,
    /// Evidence of soft delete pattern
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for SoftDeletePattern {
    fn default() -> Self {
        Self {
            detected: false,
            confidence: 0.0,
            column_names: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 2: Error Handling
// =============================================================================

/// Error handling pattern detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorHandlingPattern {
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Detected patterns (try_catch, result_type, custom_errors, etc.)
    pub patterns: Vec<String>,
    /// Custom exception/error types found
    pub exception_types: Vec<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for ErrorHandlingPattern {
    fn default() -> Self {
        Self {
            confidence: 0.0,
            patterns: Vec::new(),
            exception_types: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 3: Naming Conventions
// =============================================================================

/// Naming convention patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingPattern {
    /// Function naming convention
    pub functions: NamingConvention,
    /// Class naming convention
    pub classes: NamingConvention,
    /// Constant naming convention
    pub constants: NamingConvention,
    /// Private member prefix (e.g., "_" for Python)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_prefix: Option<String>,
    /// Consistency score (0.0-1.0)
    pub consistency_score: f64,
    /// Naming violations found
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub violations: Vec<NamingViolation>,
}

impl Default for NamingPattern {
    fn default() -> Self {
        Self {
            functions: NamingConvention::Mixed,
            classes: NamingConvention::Mixed,
            constants: NamingConvention::Mixed,
            private_prefix: None,
            consistency_score: 0.0,
            violations: Vec::new(),
        }
    }
}

/// Naming convention type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NamingConvention {
    /// snake_case
    SnakeCase,
    /// camelCase
    CamelCase,
    /// PascalCase
    PascalCase,
    /// UPPER_SNAKE_CASE
    UpperSnakeCase,
    /// Mixed conventions (no clear pattern)
    Mixed,
}

/// A naming convention violation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingViolation {
    /// Name that violates the convention
    pub name: String,
    /// Expected convention
    pub expected: NamingConvention,
    /// Actual convention detected
    pub actual: NamingConvention,
    /// File containing the violation
    pub file: String,
    /// Line number of the violating identifier
    pub line: u32,
}

// =============================================================================
// Pattern Category 4: Resource Management
// =============================================================================

/// Resource management patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceManagementPattern {
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Detected patterns (context_manager, defer, raii, finally)
    pub patterns: Vec<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for ResourceManagementPattern {
    fn default() -> Self {
        Self {
            confidence: 0.0,
            patterns: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 5: Validation
// =============================================================================

/// Input validation patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationPattern {
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Frameworks detected (pydantic, zod, validator, etc.)
    pub frameworks: Vec<String>,
    /// Patterns detected (guard_clauses, schema_validation, etc.)
    pub patterns: Vec<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for ValidationPattern {
    fn default() -> Self {
        Self {
            confidence: 0.0,
            frameworks: Vec::new(),
            patterns: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 6: Test Idioms
// =============================================================================

/// Test idiom patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestIdiomPattern {
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Testing framework detected
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    /// Patterns detected (fixtures, mocking, arrange_act_assert, etc.)
    pub patterns: Vec<String>,
    /// Whether fixtures are used
    #[serde(default)]
    pub fixture_usage: bool,
    /// Whether mocking is used
    #[serde(default)]
    pub mock_usage: bool,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for TestIdiomPattern {
    fn default() -> Self {
        Self {
            confidence: 0.0,
            framework: None,
            patterns: Vec::new(),
            fixture_usage: false,
            mock_usage: false,
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 7: Import Patterns
// =============================================================================

/// Import organization patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPattern {
    /// Import grouping style
    pub grouping_style: ImportGrouping,
    /// Absolute vs relative import preference
    pub absolute_vs_relative: ImportStyle,
    /// Star import usage
    pub star_imports: StarImportUsage,
    /// Common alias conventions (e.g., "np" for numpy)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alias_conventions: Vec<AliasConvention>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for ImportPattern {
    fn default() -> Self {
        Self {
            grouping_style: ImportGrouping::Ungrouped,
            absolute_vs_relative: ImportStyle::Mixed,
            star_imports: StarImportUsage::None,
            alias_conventions: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

/// Import grouping style
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportGrouping {
    /// stdlib, third-party, local
    StdlibFirst,
    /// third-party, stdlib, local
    ThirdPartyFirst,
    /// local, third-party, stdlib
    LocalFirst,
    /// No clear ordering
    Ungrouped,
}

/// Import style preference
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportStyle {
    /// Prefer absolute imports
    Absolute,
    /// Prefer relative imports
    Relative,
    /// Mixed usage
    Mixed,
}

/// Star import usage level
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StarImportUsage {
    /// No star imports
    None,
    /// Rare star imports
    Rare,
    /// Common star imports
    Common,
}

/// Alias convention for imports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasConvention {
    /// Module name
    pub module: String,
    /// Common alias
    pub alias: String,
    /// Usage count
    pub count: usize,
}

// =============================================================================
// Pattern Category 8: Type Coverage
// =============================================================================

/// Type annotation coverage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeCoveragePattern {
    /// Overall coverage (0.0-1.0)
    pub coverage_overall: f64,
    /// Function signature coverage
    pub coverage_functions: f64,
    /// Variable annotation coverage
    pub coverage_variables: f64,
    /// Whether TypeVar/Generic is used
    #[serde(default)]
    pub typevar_usage: bool,
    /// Common generic patterns (Optional, List, Dict, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generic_patterns: Vec<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for TypeCoveragePattern {
    fn default() -> Self {
        Self {
            coverage_overall: 0.0,
            coverage_functions: 0.0,
            coverage_variables: 0.0,
            typevar_usage: false,
            generic_patterns: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 9: API Conventions
// =============================================================================

/// API convention patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConventionPattern {
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Framework detected (fastapi, express, gin, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    /// API patterns (rest_crud, graphql, rpc, etc.)
    pub patterns: Vec<String>,
    /// ORM in use (sqlalchemy, prisma, gorm, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orm_usage: Option<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for ApiConventionPattern {
    fn default() -> Self {
        Self {
            confidence: 0.0,
            framework: None,
            patterns: Vec::new(),
            orm_usage: None,
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// Pattern Category 10: Async Patterns
// =============================================================================

/// Async/concurrency patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsyncPattern {
    /// Confidence that async is used (0.0-1.0)
    pub concurrency_confidence: f64,
    /// Patterns detected (async_await, goroutines, tokio, etc.)
    pub patterns: Vec<String>,
    /// Sync primitives used (mutex, channel, semaphore, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sync_primitives: Vec<String>,
    /// Evidence
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
}

impl Default for AsyncPattern {
    fn default() -> Self {
        Self {
            concurrency_confidence: 0.0,
            patterns: Vec::new(),
            sync_primitives: Vec::new(),
            evidence: Vec::new(),
        }
    }
}

// =============================================================================
// LLM Constraints
// =============================================================================

/// A constraint generated for LLM code generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    /// Pattern category this constraint came from
    pub category: String,
    /// Natural language rule
    pub rule: String,
    /// Confidence from the detected pattern
    pub confidence: f64,
    /// Priority (1 = highest)
    pub priority: u8,
}

impl Constraint {
    /// Create a new constraint
    pub fn new(
        category: impl Into<String>,
        rule: impl Into<String>,
        confidence: f64,
        priority: u8,
    ) -> Self {
        Self {
            category: category.into(),
            rule: rule.into(),
            confidence,
            priority,
        }
    }
}

// =============================================================================
// Pattern Categories Enum (for filtering)
// =============================================================================

/// Pattern category for filtering
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PatternCategory {
    /// Soft delete pattern (is_deleted, deleted_at columns)
    SoftDelete,
    /// Error handling patterns (try/catch, Result types, custom errors)
    ErrorHandling,
    /// Naming convention patterns (snake_case, PascalCase, etc.)
    Naming,
    /// Resource management patterns (context managers, defer, RAII)
    ResourceManagement,
    /// Input validation patterns (Pydantic, Zod, guard clauses)
    Validation,
    /// Test idiom patterns (fixtures, mocking, AAA structure)
    TestIdioms,
    /// Import organization patterns (grouping, absolute vs relative)
    ImportPatterns,
    /// Type annotation coverage patterns
    TypeCoverage,
    /// API convention patterns (REST, GraphQL, framework detection)
    ApiConventions,
    /// Async/concurrency patterns (async/await, goroutines, tokio)
    AsyncPatterns,
}

impl std::fmt::Display for PatternCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            PatternCategory::SoftDelete => "soft_delete",
            PatternCategory::ErrorHandling => "error_handling",
            PatternCategory::Naming => "naming",
            PatternCategory::ResourceManagement => "resource_management",
            PatternCategory::Validation => "validation",
            PatternCategory::TestIdioms => "test_idioms",
            PatternCategory::ImportPatterns => "import_patterns",
            PatternCategory::TypeCoverage => "type_coverage",
            PatternCategory::ApiConventions => "api_conventions",
            PatternCategory::AsyncPatterns => "async_patterns",
        };
        write!(f, "{}", name)
    }
}

impl std::str::FromStr for PatternCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "soft_delete" | "softdelete" => Ok(PatternCategory::SoftDelete),
            "error_handling" | "errorhandling" | "errors" => Ok(PatternCategory::ErrorHandling),
            "naming" => Ok(PatternCategory::Naming),
            "resource_management" | "resourcemanagement" | "resources" => {
                Ok(PatternCategory::ResourceManagement)
            }
            "validation" => Ok(PatternCategory::Validation),
            "test_idioms" | "testidioms" | "tests" => Ok(PatternCategory::TestIdioms),
            "import_patterns" | "importpatterns" | "imports" => Ok(PatternCategory::ImportPatterns),
            "type_coverage" | "typecoverage" | "types" => Ok(PatternCategory::TypeCoverage),
            "api_conventions" | "apiconventions" | "api" => Ok(PatternCategory::ApiConventions),
            "async_patterns" | "asyncpatterns" | "async" => Ok(PatternCategory::AsyncPatterns),
            _ => Err(format!("Unknown pattern category: {}", s)),
        }
    }
}

// =============================================================================
// Generic Pattern Match (for HashMap<PatternCategory, Vec<PatternMatch>>)
// =============================================================================

/// A generic pattern match with location and confidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    /// Location (file:line)
    pub location: String,
    /// Pattern name
    pub pattern_name: String,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Evidence/reason for the match
    pub evidence: String,
}

impl PatternMatch {
    /// Create a new pattern match
    pub fn new(
        location: impl Into<String>,
        pattern_name: impl Into<String>,
        confidence: f64,
        evidence: impl Into<String>,
    ) -> Self {
        Self {
            location: location.into(),
            pattern_name: pattern_name.into(),
            confidence,
            evidence: evidence.into(),
        }
    }
}

/// Summary of patterns found
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatternSummary {
    /// Total patterns found
    pub total_patterns: usize,
    /// Patterns by category
    pub by_category: HashMap<String, usize>,
    /// Average confidence
    pub average_confidence: f64,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_report_serialization() {
        let metadata = PatternMetadata::new(42, 1234);
        let report = PatternReport::new(metadata);

        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"files_analyzed\": 42"));
        assert!(json.contains("\"duration_ms\": 1234"));

        let parsed: PatternReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.metadata.files_analyzed, 42);
    }

    #[test]
    fn test_soft_delete_pattern_serialization() {
        let pattern = SoftDeletePattern {
            detected: true,
            confidence: 0.85,
            column_names: vec!["is_deleted".to_string(), "deleted_at".to_string()],
            evidence: vec![Evidence::new(
                "models/user.py",
                15,
                "is_deleted = Column(Boolean)",
            )],
        };

        let json = serde_json::to_string(&pattern).unwrap();
        let parsed: SoftDeletePattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.confidence, 0.85);
        assert_eq!(parsed.column_names.len(), 2);
    }

    #[test]
    fn test_naming_convention_serialization() {
        let pattern = NamingPattern {
            functions: NamingConvention::SnakeCase,
            classes: NamingConvention::PascalCase,
            constants: NamingConvention::UpperSnakeCase,
            private_prefix: Some("_".to_string()),
            consistency_score: 0.95,
            violations: Vec::new(),
        };

        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("\"snake_case\""));
        assert!(json.contains("\"pascal_case\""));

        let parsed: NamingPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.functions, NamingConvention::SnakeCase);
    }

    #[test]
    fn test_constraint_serialization() {
        let constraint = Constraint::new("naming", "Use snake_case for function names", 0.95, 1);

        let json = serde_json::to_string(&constraint).unwrap();
        let parsed: Constraint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.category, "naming");
        assert_eq!(parsed.priority, 1);
    }

    #[test]
    fn test_pattern_category_from_str() {
        assert_eq!(
            "soft_delete".parse::<PatternCategory>().unwrap(),
            PatternCategory::SoftDelete
        );
        assert_eq!(
            "error-handling".parse::<PatternCategory>().unwrap(),
            PatternCategory::ErrorHandling
        );
        assert_eq!(
            "async".parse::<PatternCategory>().unwrap(),
            PatternCategory::AsyncPatterns
        );
        assert!("unknown".parse::<PatternCategory>().is_err());
    }

    #[test]
    fn test_pattern_match_serialization() {
        let match_ = PatternMatch::new(
            "src/models.py:42",
            "soft_delete_field",
            0.8,
            "Found is_deleted column",
        );

        let json = serde_json::to_string(&match_).unwrap();
        let parsed: PatternMatch = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.confidence, 0.8);
    }

    #[test]
    fn test_import_grouping_serialization() {
        let pattern = ImportPattern {
            grouping_style: ImportGrouping::StdlibFirst,
            absolute_vs_relative: ImportStyle::Absolute,
            star_imports: StarImportUsage::Rare,
            alias_conventions: vec![AliasConvention {
                module: "numpy".to_string(),
                alias: "np".to_string(),
                count: 15,
            }],
            evidence: Vec::new(),
        };

        let json = serde_json::to_string(&pattern).unwrap();
        assert!(json.contains("\"stdlib_first\""));
        assert!(json.contains("\"absolute\""));

        let parsed: ImportPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.grouping_style, ImportGrouping::StdlibFirst);
    }
}
