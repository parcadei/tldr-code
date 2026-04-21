//! Type annotation coverage pattern detection
//!
//! Detects type annotation patterns:
//! - Function parameter type coverage
//! - Return type coverage
//! - Variable annotation coverage
//! - Generic type usage (TypeVar, Generic[])

use super::signals::PatternSignals;
use crate::types::{Evidence, TypeCoveragePattern};

/// Convert signals to type coverage pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<TypeCoveragePattern> {
    let type_coverage = &signals.type_coverage;

    if !type_coverage.has_signals() {
        return None;
    }

    let coverage_overall = type_coverage.calculate_overall_coverage();
    let coverage_functions = type_coverage.calculate_function_coverage();
    let coverage_variables = type_coverage.calculate_variable_coverage();

    let typevar_usage = !type_coverage.generic_usage.is_empty();
    let generic_patterns: Vec<String> = type_coverage.generic_patterns.iter().cloned().collect();

    // Collect evidence (limited)
    let evidence: Vec<Evidence> = type_coverage
        .generic_usage
        .iter()
        .take(evidence_limit)
        .cloned()
        .collect();

    Some(TypeCoveragePattern {
        coverage_overall,
        coverage_functions,
        coverage_variables,
        typevar_usage,
        generic_patterns,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_signals_returns_none() {
        let signals = PatternSignals::default();
        assert!(signals_to_pattern(&signals, 3).is_none());
    }

    #[test]
    fn test_full_type_coverage() {
        let mut signals = PatternSignals::default();
        signals.type_coverage.typed_params = 10;
        signals.type_coverage.untyped_params = 0;
        signals.type_coverage.typed_returns = 5;
        signals.type_coverage.untyped_returns = 0;

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!((pattern.coverage_functions - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_partial_type_coverage() {
        let mut signals = PatternSignals::default();
        signals.type_coverage.typed_params = 5;
        signals.type_coverage.untyped_params = 5;
        signals.type_coverage.typed_returns = 2;
        signals.type_coverage.untyped_returns = 8;

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.coverage_functions < 1.0);
        assert!(pattern.coverage_functions > 0.0);
    }

    #[test]
    fn test_generic_patterns_detected() {
        let mut signals = PatternSignals::default();
        signals.type_coverage.typed_params = 5;
        signals
            .type_coverage
            .generic_patterns
            .insert("Optional".to_string());
        signals
            .type_coverage
            .generic_patterns
            .insert("List".to_string());

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.generic_patterns.contains(&"Optional".to_string()));
        assert!(pattern.generic_patterns.contains(&"List".to_string()));
    }

    #[test]
    fn test_typevar_usage_detected() {
        let mut signals = PatternSignals::default();
        signals.type_coverage.typed_params = 5;
        signals
            .type_coverage
            .generic_usage
            .push(crate::types::Evidence::new(
                "utils.py",
                10,
                "T = TypeVar('T')",
            ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.typevar_usage);
    }
}
