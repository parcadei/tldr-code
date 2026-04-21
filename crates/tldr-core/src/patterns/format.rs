//! Text formatting for pattern reports
//!
//! Provides human-readable text output for pattern analysis.

use crate::types::{NamingConvention, PatternReport};

/// Format a pattern report as human-readable text
pub fn format_pattern_report_text(report: &PatternReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str("# Pattern Analysis Report\n\n");

    // Metadata
    output.push_str("## Summary\n");
    output.push_str(&format!(
        "- Files analyzed: {}\n",
        report.metadata.files_analyzed
    ));
    output.push_str(&format!(
        "- Files skipped: {}\n",
        report.metadata.files_skipped
    ));
    output.push_str(&format!("- Duration: {}ms\n", report.metadata.duration_ms));
    output.push_str(&format!(
        "- Patterns found: {}\n\n",
        report.metadata.patterns_after_filter
    ));

    // Languages
    if !report
        .metadata
        .language_distribution
        .files_by_language
        .is_empty()
    {
        output.push_str("### Languages\n");
        for (lang, count) in &report.metadata.language_distribution.files_by_language {
            output.push_str(&format!("- {}: {} files\n", lang, count));
        }
        output.push('\n');
    }

    // Pattern sections
    if let Some(ref pattern) = report.soft_delete {
        output.push_str("## Soft Delete Pattern\n");
        output.push_str(&format!("- Detected: {}\n", pattern.detected));
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if !pattern.column_names.is_empty() {
            output.push_str(&format!("- Columns: {}\n", pattern.column_names.join(", ")));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.error_handling {
        output.push_str("## Error Handling\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        if !pattern.exception_types.is_empty() {
            output.push_str(&format!(
                "- Exception types: {}\n",
                pattern.exception_types.join(", ")
            ));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.naming {
        output.push_str("## Naming Conventions\n");
        output.push_str(&format!(
            "- Consistency: {:.0}%\n",
            pattern.consistency_score * 100.0
        ));
        output.push_str(&format!(
            "- Functions: {}\n",
            convention_name(&pattern.functions)
        ));
        output.push_str(&format!(
            "- Classes: {}\n",
            convention_name(&pattern.classes)
        ));
        output.push_str(&format!(
            "- Constants: {}\n",
            convention_name(&pattern.constants)
        ));
        if let Some(ref prefix) = pattern.private_prefix {
            output.push_str(&format!("- Private prefix: '{}'\n", prefix));
        }
        if !pattern.violations.is_empty() {
            output.push_str(&format!("- Violations: {}\n", pattern.violations.len()));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.resource_management {
        output.push_str("## Resource Management\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.validation {
        output.push_str("## Input Validation\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if !pattern.frameworks.is_empty() {
            output.push_str(&format!(
                "- Frameworks: {}\n",
                pattern.frameworks.join(", ")
            ));
        }
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.test_idioms {
        output.push_str("## Test Idioms\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if let Some(ref framework) = pattern.framework {
            output.push_str(&format!("- Framework: {}\n", framework));
        }
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        output.push_str(&format!("- Fixture usage: {}\n", pattern.fixture_usage));
        output.push_str(&format!("- Mock usage: {}\n", pattern.mock_usage));
        output.push('\n');
    }

    if let Some(ref pattern) = report.import_patterns {
        output.push_str("## Import Patterns\n");
        output.push_str(&format!("- Grouping: {:?}\n", pattern.grouping_style));
        output.push_str(&format!("- Style: {:?}\n", pattern.absolute_vs_relative));
        output.push_str(&format!("- Star imports: {:?}\n", pattern.star_imports));
        if !pattern.alias_conventions.is_empty() {
            output.push_str("- Aliases:\n");
            for alias in &pattern.alias_conventions {
                output.push_str(&format!("  - {} -> {}\n", alias.module, alias.alias));
            }
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.type_coverage {
        output.push_str("## Type Coverage\n");
        output.push_str(&format!(
            "- Overall: {:.0}%\n",
            pattern.coverage_overall * 100.0
        ));
        output.push_str(&format!(
            "- Functions: {:.0}%\n",
            pattern.coverage_functions * 100.0
        ));
        output.push_str(&format!(
            "- Variables: {:.0}%\n",
            pattern.coverage_variables * 100.0
        ));
        output.push_str(&format!("- TypeVar usage: {}\n", pattern.typevar_usage));
        if !pattern.generic_patterns.is_empty() {
            output.push_str(&format!(
                "- Generic patterns: {}\n",
                pattern.generic_patterns.join(", ")
            ));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.api_conventions {
        output.push_str("## API Conventions\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.confidence * 100.0
        ));
        if let Some(ref framework) = pattern.framework {
            output.push_str(&format!("- Framework: {}\n", framework));
        }
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        if let Some(ref orm) = pattern.orm_usage {
            output.push_str(&format!("- ORM: {}\n", orm));
        }
        output.push('\n');
    }

    if let Some(ref pattern) = report.async_patterns {
        output.push_str("## Async/Concurrency\n");
        output.push_str(&format!(
            "- Confidence: {:.0}%\n",
            pattern.concurrency_confidence * 100.0
        ));
        if !pattern.patterns.is_empty() {
            output.push_str(&format!("- Patterns: {}\n", pattern.patterns.join(", ")));
        }
        if !pattern.sync_primitives.is_empty() {
            output.push_str(&format!(
                "- Sync primitives: {}\n",
                pattern.sync_primitives.join(", ")
            ));
        }
        output.push('\n');
    }

    // Constraints
    if !report.constraints.is_empty() {
        output.push_str("## LLM Constraints\n");
        for constraint in &report.constraints {
            output.push_str(&format!(
                "- [{}] {}\n",
                constraint.category, constraint.rule
            ));
        }
        output.push('\n');
    }

    // Conflicts
    if !report.conflicts.is_empty() {
        output.push_str("## Conflicts Detected\n");
        for conflict in &report.conflicts {
            output.push_str(&format!("- {}\n", conflict));
        }
        output.push('\n');
    }

    output
}

fn convention_name(conv: &NamingConvention) -> &'static str {
    match conv {
        NamingConvention::SnakeCase => "snake_case",
        NamingConvention::CamelCase => "camelCase",
        NamingConvention::PascalCase => "PascalCase",
        NamingConvention::UpperSnakeCase => "UPPER_SNAKE_CASE",
        NamingConvention::Mixed => "Mixed",
    }
}
