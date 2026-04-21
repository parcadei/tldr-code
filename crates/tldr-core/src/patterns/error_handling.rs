//! Error handling pattern detection
//!
//! Detects error handling patterns across languages:
//! - Python: try/except, custom Exception classes
//! - Rust: Result<T, E>, ? operator, error enums
//! - Go: if err != nil pattern
//! - TypeScript: try/catch blocks

use super::signals::PatternSignals;
use crate::types::ErrorHandlingPattern;

/// Convert signals to error handling pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<ErrorHandlingPattern> {
    let error_handling = &signals.error_handling;

    if !error_handling.has_signals() {
        return None;
    }

    let confidence = error_handling.calculate_confidence();

    // Detect patterns
    let mut patterns = Vec::new();

    if !error_handling.try_except_blocks.is_empty() || !error_handling.try_catch_blocks.is_empty() {
        patterns.push("try_catch".to_string());
    }

    if !error_handling.result_types.is_empty() {
        patterns.push("result_type".to_string());
    }

    if !error_handling.question_mark_ops.is_empty() {
        patterns.push("question_mark_operator".to_string());
    }

    if !error_handling.err_nil_checks.is_empty() {
        patterns.push("err_nil_check".to_string());
    }

    if !error_handling.custom_exceptions.is_empty() || !error_handling.error_enums.is_empty() {
        patterns.push("custom_errors".to_string());
    }

    // Collect exception types
    let mut exception_types: Vec<String> = error_handling
        .custom_exceptions
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    exception_types.extend(
        error_handling
            .error_enums
            .iter()
            .map(|(name, _)| name.clone()),
    );
    exception_types.sort();
    exception_types.dedup();

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        error_handling
            .try_except_blocks
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        error_handling
            .try_catch_blocks
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        error_handling
            .result_types
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        error_handling
            .err_nil_checks
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        error_handling
            .custom_exceptions
            .iter()
            .take(evidence_limit)
            .map(|(_, e)| e.clone()),
    );
    evidence.truncate(evidence_limit);

    Some(ErrorHandlingPattern {
        confidence,
        patterns,
        exception_types,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Evidence;

    #[test]
    fn test_no_signals_returns_none() {
        let signals = PatternSignals::default();
        assert!(signals_to_pattern(&signals, 3).is_none());
    }

    #[test]
    fn test_try_catch_pattern_detected() {
        let mut signals = PatternSignals::default();
        signals.error_handling.try_except_blocks.push(Evidence::new(
            "service.py",
            10,
            "try:\n    process()\nexcept Exception as e:\n    log(e)",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.confidence >= 0.3);
        assert!(pattern.patterns.contains(&"try_catch".to_string()));
    }

    #[test]
    fn test_result_type_pattern_detected() {
        let mut signals = PatternSignals::default();
        signals.error_handling.result_types.push(Evidence::new(
            "lib.rs",
            10,
            "fn process() -> Result<String, Error>",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.confidence >= 0.4);
        assert!(pattern.patterns.contains(&"result_type".to_string()));
    }

    #[test]
    fn test_custom_exceptions_detected() {
        let mut signals = PatternSignals::default();
        signals.error_handling.custom_exceptions.push((
            "ValidationError".to_string(),
            Evidence::new("errors.py", 5, "class ValidationError(Exception): pass"),
        ));
        signals.error_handling.custom_exceptions.push((
            "NotFoundError".to_string(),
            Evidence::new("errors.py", 10, "class NotFoundError(Exception): pass"),
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"custom_errors".to_string()));
        assert!(pattern
            .exception_types
            .contains(&"ValidationError".to_string()));
        assert!(pattern
            .exception_types
            .contains(&"NotFoundError".to_string()));
    }
}
