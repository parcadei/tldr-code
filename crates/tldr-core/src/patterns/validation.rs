//! Input validation pattern detection
//!
//! Detects validation patterns:
//! - Pydantic BaseModel usage
//! - Zod schema definitions
//! - Guard clauses at function start
//! - Assert statements
//! - Type validation (isinstance, typeof)

use super::signals::PatternSignals;
use crate::types::ValidationPattern;

/// Convert signals to validation pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<ValidationPattern> {
    let validation = &signals.validation;

    if !validation.has_signals() {
        return None;
    }

    let confidence = validation.calculate_confidence();

    // Detect frameworks
    let mut frameworks = Vec::new();
    if !validation.pydantic_models.is_empty() {
        frameworks.push("pydantic".to_string());
    }
    if !validation.zod_schemas.is_empty() {
        frameworks.push("zod".to_string());
    }
    for (name, _) in &validation.other_validators {
        frameworks.push(name.clone());
    }
    frameworks.sort();
    frameworks.dedup();

    // Detect patterns
    let mut patterns = Vec::new();
    if !validation.pydantic_models.is_empty() || !validation.zod_schemas.is_empty() {
        patterns.push("schema_validation".to_string());
    }
    if !validation.guard_clauses.is_empty() {
        patterns.push("guard_clauses".to_string());
    }
    if !validation.assert_statements.is_empty() {
        patterns.push("assertions".to_string());
    }
    if !validation.type_checks.is_empty() {
        patterns.push("type_checking".to_string());
    }

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        validation
            .pydantic_models
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(validation.zod_schemas.iter().take(evidence_limit).cloned());
    evidence.extend(
        validation
            .guard_clauses
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        validation
            .assert_statements
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(validation.type_checks.iter().take(evidence_limit).cloned());
    evidence.truncate(evidence_limit);

    Some(ValidationPattern {
        confidence,
        frameworks,
        patterns,
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
    fn test_pydantic_detected() {
        let mut signals = PatternSignals::default();
        signals.validation.pydantic_models.push(Evidence::new(
            "models.py",
            5,
            "class User(BaseModel):",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.confidence >= 0.5);
        assert!(pattern.frameworks.contains(&"pydantic".to_string()));
        assert!(pattern.patterns.contains(&"schema_validation".to_string()));
    }

    #[test]
    fn test_zod_detected() {
        let mut signals = PatternSignals::default();
        signals.validation.zod_schemas.push(Evidence::new(
            "schema.ts",
            10,
            "const userSchema = z.object({ name: z.string() })",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.frameworks.contains(&"zod".to_string()));
    }

    #[test]
    fn test_assert_pattern_detected() {
        let mut signals = PatternSignals::default();
        signals.validation.assert_statements.push(Evidence::new(
            "utils.py",
            15,
            "assert x > 0, 'x must be positive'",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"assertions".to_string()));
    }
}
