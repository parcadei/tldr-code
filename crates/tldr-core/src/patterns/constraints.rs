//! LLM constraint generation from detected patterns
//!
//! Generates natural language rules/constraints for LLM code generation
//! based on detected codebase patterns.

use crate::types::{
    ApiConventionPattern, AsyncPattern, Constraint, ErrorHandlingPattern, ImportGrouping,
    ImportPattern, ImportStyle, NamingConvention, NamingPattern, ResourceManagementPattern,
    SoftDeletePattern, TestIdiomPattern, TypeCoveragePattern, ValidationPattern,
};

/// Collection of detected code patterns for constraint generation and analysis.
pub struct DetectedPatterns<'a> {
    /// Detected soft-delete conventions.
    pub soft_delete: &'a Option<SoftDeletePattern>,
    /// Detected error handling conventions.
    pub error_handling: &'a Option<ErrorHandlingPattern>,
    /// Detected naming conventions.
    pub naming: &'a Option<NamingPattern>,
    /// Detected resource lifecycle/cleanup conventions.
    pub resource_management: &'a Option<ResourceManagementPattern>,
    /// Detected validation conventions.
    pub validation: &'a Option<ValidationPattern>,
    /// Detected testing idioms.
    pub test_idioms: &'a Option<TestIdiomPattern>,
    /// Detected import organization/style conventions.
    pub import_patterns: &'a Option<ImportPattern>,
    /// Detected type coverage conventions.
    pub type_coverage: &'a Option<TypeCoveragePattern>,
    /// Detected API design conventions.
    pub api_conventions: &'a Option<ApiConventionPattern>,
    /// Detected async/concurrency conventions.
    pub async_patterns: &'a Option<AsyncPattern>,
}

/// Generate LLM constraints from detected patterns
pub fn generate_constraints(patterns: &DetectedPatterns<'_>) -> Vec<Constraint> {
    let mut constraints = Vec::new();

    add_soft_delete_constraints(patterns.soft_delete, &mut constraints);
    add_error_handling_constraints(patterns.error_handling, &mut constraints);
    add_naming_constraints(patterns.naming, &mut constraints);
    add_resource_management_constraints(patterns.resource_management, &mut constraints);
    add_validation_constraints(patterns.validation, &mut constraints);
    add_test_constraints(patterns.test_idioms, &mut constraints);
    add_import_constraints(patterns.import_patterns, &mut constraints);
    add_type_constraints(patterns.type_coverage, &mut constraints);
    add_api_constraints(patterns.api_conventions, &mut constraints);
    add_async_constraints(patterns.async_patterns, &mut constraints);

    // Sort by priority
    constraints.sort_by_key(|c| c.priority);

    constraints
}

fn add_soft_delete_constraints(
    soft_delete: &Option<SoftDeletePattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = soft_delete else {
        return;
    };
    if !pattern.detected || pattern.confidence < 0.4 {
        return;
    }
    constraints.push(Constraint::new(
        "soft_delete",
        format!(
            "Use soft delete pattern with {} fields instead of hard DELETE",
            pattern.column_names.join(", ")
        ),
        pattern.confidence,
        1,
    ));
}

fn add_error_handling_constraints(
    error_handling: &Option<ErrorHandlingPattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = error_handling else {
        return;
    };
    if pattern.confidence < 0.3 {
        return;
    }
    if pattern.patterns.contains(&"result_type".to_string()) {
        constraints.push(Constraint::new(
            "error_handling",
            "Use Result<T, E> return types for fallible operations",
            pattern.confidence,
            1,
        ));
    }
    if pattern.patterns.contains(&"try_catch".to_string()) {
        constraints.push(Constraint::new(
            "error_handling",
            "Wrap error-prone operations in try/catch blocks with specific exception handling",
            pattern.confidence,
            2,
        ));
    }
    if pattern.patterns.contains(&"custom_errors".to_string())
        && !pattern.exception_types.is_empty()
    {
        constraints.push(Constraint::new(
            "error_handling",
            format!(
                "Use existing custom error types: {}",
                pattern.exception_types.join(", ")
            ),
            pattern.confidence,
            2,
        ));
    }
}

fn add_naming_constraints(naming: &Option<NamingPattern>, constraints: &mut Vec<Constraint>) {
    let Some(pattern) = naming else {
        return;
    };
    if pattern.consistency_score < 0.5 {
        return;
    }
    constraints.push(Constraint::new(
        "naming",
        format!(
            "Function names: {}, Class names: {}, Constants: {}",
            convention_to_string(&pattern.functions),
            convention_to_string(&pattern.classes),
            convention_to_string(&pattern.constants),
        ),
        pattern.consistency_score,
        1,
    ));
    if let Some(ref prefix) = pattern.private_prefix {
        constraints.push(Constraint::new(
            "naming",
            format!("Use '{}' prefix for private members", prefix),
            pattern.consistency_score,
            2,
        ));
    }
}

fn add_resource_management_constraints(
    resource_management: &Option<ResourceManagementPattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = resource_management else {
        return;
    };
    if pattern.confidence < 0.4 {
        return;
    }
    for p in &pattern.patterns {
        let rule = match p.as_str() {
            "context_manager" => "Use 'with' statements (context managers) for resource management",
            "defer" => "Use 'defer' to ensure cleanup runs even on error",
            "raii" => "Implement Drop trait for types that manage external resources",
            "finally" => "Use try/finally for explicit resource cleanup",
            _ => continue,
        };
        constraints.push(Constraint::new(
            "resource_management",
            rule,
            pattern.confidence,
            1,
        ));
    }
}

fn add_validation_constraints(
    validation: &Option<ValidationPattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = validation else {
        return;
    };
    if pattern.confidence < 0.3 {
        return;
    }
    if !pattern.frameworks.is_empty() {
        constraints.push(Constraint::new(
            "validation",
            format!(
                "Use {} for input validation",
                pattern.frameworks.join(" or ")
            ),
            pattern.confidence,
            1,
        ));
    }
    if pattern.patterns.contains(&"guard_clauses".to_string()) {
        constraints.push(Constraint::new(
            "validation",
            "Validate inputs at function start with guard clauses",
            pattern.confidence,
            2,
        ));
    }
}

fn add_test_constraints(test_idioms: &Option<TestIdiomPattern>, constraints: &mut Vec<Constraint>) {
    let Some(pattern) = test_idioms else {
        return;
    };
    if pattern.confidence < 0.3 {
        return;
    }
    if let Some(ref framework) = pattern.framework {
        constraints.push(Constraint::new(
            "testing",
            format!("Use {} testing framework", framework),
            pattern.confidence,
            1,
        ));
    }
    if pattern.fixture_usage {
        constraints.push(Constraint::new(
            "testing",
            "Use fixtures for test setup/teardown",
            pattern.confidence,
            2,
        ));
    }
    if pattern.mock_usage {
        constraints.push(Constraint::new(
            "testing",
            "Use mocking for external dependencies in tests",
            pattern.confidence,
            2,
        ));
    }
}

fn add_import_constraints(
    import_patterns: &Option<ImportPattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = import_patterns else {
        return;
    };
    match pattern.absolute_vs_relative {
        ImportStyle::Absolute => constraints.push(Constraint::new(
            "imports",
            "Prefer absolute imports over relative imports",
            1.0,
            2,
        )),
        ImportStyle::Relative => constraints.push(Constraint::new(
            "imports",
            "Prefer relative imports for local modules",
            1.0,
            2,
        )),
        ImportStyle::Mixed => {}
    }
    match pattern.grouping_style {
        ImportGrouping::StdlibFirst => constraints.push(Constraint::new(
            "imports",
            "Group imports: stdlib first, then third-party, then local",
            1.0,
            3,
        )),
        ImportGrouping::ThirdPartyFirst => constraints.push(Constraint::new(
            "imports",
            "Group imports: third-party first, then stdlib, then local",
            1.0,
            3,
        )),
        _ => {}
    }
}

fn add_type_constraints(
    type_coverage: &Option<TypeCoveragePattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = type_coverage else {
        return;
    };
    if pattern.coverage_overall >= 0.5 {
        constraints.push(Constraint::new(
            "types",
            "Add type annotations to function parameters and return values",
            pattern.coverage_overall,
            1,
        ));
    }
    if pattern.typevar_usage {
        constraints.push(Constraint::new(
            "types",
            "Use generics/TypeVar for reusable type-safe functions",
            pattern.coverage_overall,
            2,
        ));
    }
}

fn add_api_constraints(
    api_conventions: &Option<ApiConventionPattern>,
    constraints: &mut Vec<Constraint>,
) {
    let Some(pattern) = api_conventions else {
        return;
    };
    if pattern.confidence < 0.4 {
        return;
    }
    if let Some(ref framework) = pattern.framework {
        constraints.push(Constraint::new(
            "api",
            format!("Follow {} patterns for API endpoints", framework),
            pattern.confidence,
            1,
        ));
    }
    if pattern.patterns.contains(&"rest_crud".to_string()) {
        constraints.push(Constraint::new(
            "api",
            "Follow REST conventions: GET for read, POST for create, PUT for update, DELETE for delete",
            pattern.confidence,
            1,
        ));
    }
    if let Some(ref orm) = pattern.orm_usage {
        constraints.push(Constraint::new(
            "api",
            format!("Use {} for database operations", orm),
            pattern.confidence,
            2,
        ));
    }
}

fn add_async_constraints(async_patterns: &Option<AsyncPattern>, constraints: &mut Vec<Constraint>) {
    let Some(pattern) = async_patterns else {
        return;
    };
    if pattern.concurrency_confidence < 0.3 {
        return;
    }
    if pattern.patterns.contains(&"async_await".to_string()) {
        constraints.push(Constraint::new(
            "async",
            "Use async/await for asynchronous operations",
            pattern.concurrency_confidence,
            1,
        ));
    }
    if pattern.patterns.contains(&"goroutines".to_string()) {
        constraints.push(Constraint::new(
            "async",
            "Use goroutines for concurrent operations",
            pattern.concurrency_confidence,
            1,
        ));
    }
    if !pattern.sync_primitives.is_empty() {
        constraints.push(Constraint::new(
            "async",
            format!(
                "Use {} for thread synchronization",
                pattern.sync_primitives.join(", ")
            ),
            pattern.concurrency_confidence,
            2,
        ));
    }
}

fn convention_to_string(conv: &NamingConvention) -> &'static str {
    match conv {
        NamingConvention::SnakeCase => "snake_case",
        NamingConvention::CamelCase => "camelCase",
        NamingConvention::PascalCase => "PascalCase",
        NamingConvention::UpperSnakeCase => "UPPER_SNAKE_CASE",
        NamingConvention::Mixed => "mixed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_soft_delete_constraint() {
        let soft_delete = Some(SoftDeletePattern {
            detected: true,
            confidence: 0.8,
            column_names: vec!["is_deleted".to_string(), "deleted_at".to_string()],
            evidence: vec![],
        });

        let constraints = generate_constraints(&DetectedPatterns {
            soft_delete: &soft_delete,
            error_handling: &None,
            naming: &None,
            resource_management: &None,
            validation: &None,
            test_idioms: &None,
            import_patterns: &None,
            type_coverage: &None,
            api_conventions: &None,
            async_patterns: &None,
        });

        assert!(!constraints.is_empty());
        assert!(constraints[0].rule.contains("soft delete"));
        assert!(constraints[0].rule.contains("is_deleted"));
    }

    #[test]
    fn test_naming_constraint() {
        let naming = Some(NamingPattern {
            functions: NamingConvention::SnakeCase,
            classes: NamingConvention::PascalCase,
            constants: NamingConvention::UpperSnakeCase,
            private_prefix: Some("_".to_string()),
            consistency_score: 0.9,
            violations: vec![],
        });

        let constraints = generate_constraints(&DetectedPatterns {
            soft_delete: &None,
            error_handling: &None,
            naming: &naming,
            resource_management: &None,
            validation: &None,
            test_idioms: &None,
            import_patterns: &None,
            type_coverage: &None,
            api_conventions: &None,
            async_patterns: &None,
        });

        assert!(constraints.len() >= 2);
        assert!(constraints.iter().any(|c| c.rule.contains("snake_case")));
        assert!(constraints.iter().any(|c| c.rule.contains("_")));
    }

    #[test]
    fn test_no_constraints_below_threshold() {
        let soft_delete = Some(SoftDeletePattern {
            detected: true,
            confidence: 0.2, // Below threshold
            column_names: vec!["is_deleted".to_string()],
            evidence: vec![],
        });

        let constraints = generate_constraints(&DetectedPatterns {
            soft_delete: &soft_delete,
            error_handling: &None,
            naming: &None,
            resource_management: &None,
            validation: &None,
            test_idioms: &None,
            import_patterns: &None,
            type_coverage: &None,
            api_conventions: &None,
            async_patterns: &None,
        });

        assert!(constraints.is_empty());
    }
}
