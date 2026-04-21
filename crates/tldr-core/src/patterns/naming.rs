//! Naming convention pattern detection
//!
//! Detects naming conventions for:
//! - Functions: snake_case, camelCase
//! - Classes: PascalCase
//! - Constants: UPPER_SNAKE_CASE
//!
//! Calculates consistency score and flags violations.

use std::collections::HashMap;

use super::signals::{NamingCase, PatternSignals};
use crate::types::{NamingConvention, NamingPattern, NamingViolation};

/// Convert signals to naming pattern
pub fn signals_to_pattern(signals: &PatternSignals) -> Option<NamingPattern> {
    let naming = &signals.naming;

    if !naming.has_signals() {
        return None;
    }

    // Determine majority convention for each category
    let functions = detect_majority_convention(&naming.function_names);
    let classes = detect_majority_convention(&naming.class_names);
    let constants = detect_majority_convention(&naming.constant_names);

    // Calculate consistency score
    let function_consistency = calculate_consistency(&naming.function_names, &functions);
    let class_consistency = calculate_consistency(&naming.class_names, &classes);
    let constant_consistency = calculate_consistency(&naming.constant_names, &constants);

    let total_items =
        naming.function_names.len() + naming.class_names.len() + naming.constant_names.len();
    let consistency_score = if total_items > 0 {
        let fn_weight = naming.function_names.len() as f64 / total_items as f64;
        let cls_weight = naming.class_names.len() as f64 / total_items as f64;
        let const_weight = naming.constant_names.len() as f64 / total_items as f64;

        function_consistency * fn_weight
            + class_consistency * cls_weight
            + constant_consistency * const_weight
    } else {
        0.0
    };

    // Detect violations
    let mut violations = Vec::new();
    violations.extend(find_violations(&naming.function_names, &functions));
    violations.extend(find_violations(&naming.class_names, &classes));
    violations.extend(find_violations(&naming.constant_names, &constants));

    // Detect private prefix
    let private_prefix = naming
        .private_prefixes
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(prefix, _)| prefix.clone());

    Some(NamingPattern {
        functions: naming_case_to_convention(functions),
        classes: naming_case_to_convention(classes),
        constants: naming_case_to_convention(constants),
        private_prefix,
        consistency_score,
        violations,
    })
}

/// Detect the majority naming convention from a list of names
fn detect_majority_convention(names: &[(String, NamingCase, String)]) -> NamingCase {
    if names.is_empty() {
        return NamingCase::Unknown;
    }

    let mut counts: HashMap<NamingCase, usize> = HashMap::new();
    for (_, case, _) in names {
        if *case != NamingCase::Unknown {
            *counts.entry(*case).or_insert(0) += 1;
        }
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(case, _)| case)
        .unwrap_or(NamingCase::Unknown)
}

/// Calculate consistency score for a set of names against expected convention
fn calculate_consistency(names: &[(String, NamingCase, String)], expected: &NamingCase) -> f64 {
    if names.is_empty() || *expected == NamingCase::Unknown {
        return 0.0;
    }

    let matching = names.iter().filter(|(_, case, _)| case == expected).count();
    matching as f64 / names.len() as f64
}

/// Find violations (names not matching the expected convention)
fn find_violations(
    names: &[(String, NamingCase, String)],
    expected: &NamingCase,
) -> Vec<NamingViolation> {
    if *expected == NamingCase::Unknown {
        return Vec::new();
    }

    names
        .iter()
        .filter(|(_, case, _)| *case != NamingCase::Unknown && case != expected)
        .map(|(name, case, file)| NamingViolation {
            name: name.clone(),
            expected: naming_case_to_convention(*expected),
            actual: naming_case_to_convention(*case),
            file: file.clone(),
            line: 0, // Line info not tracked in signals
        })
        .collect()
}

/// Convert internal NamingCase to public NamingConvention
fn naming_case_to_convention(case: NamingCase) -> NamingConvention {
    match case {
        NamingCase::SnakeCase => NamingConvention::SnakeCase,
        NamingCase::CamelCase => NamingConvention::CamelCase,
        NamingCase::PascalCase => NamingConvention::PascalCase,
        NamingCase::UpperSnakeCase => NamingConvention::UpperSnakeCase,
        NamingCase::Unknown => NamingConvention::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_signals_returns_none() {
        let signals = PatternSignals::default();
        assert!(signals_to_pattern(&signals).is_none());
    }

    #[test]
    fn test_snake_case_functions_detected() {
        let mut signals = PatternSignals::default();
        signals.naming.function_names.push((
            "find_user_by_id".to_string(),
            NamingCase::SnakeCase,
            "service.py".to_string(),
        ));
        signals.naming.function_names.push((
            "get_all_users".to_string(),
            NamingCase::SnakeCase,
            "service.py".to_string(),
        ));
        signals.naming.function_names.push((
            "create_user".to_string(),
            NamingCase::SnakeCase,
            "service.py".to_string(),
        ));

        let pattern = signals_to_pattern(&signals).unwrap();
        assert_eq!(pattern.functions, NamingConvention::SnakeCase);
        assert!(pattern.consistency_score >= 0.9);
    }

    #[test]
    fn test_pascal_case_classes_detected() {
        let mut signals = PatternSignals::default();
        signals.naming.class_names.push((
            "UserService".to_string(),
            NamingCase::PascalCase,
            "service.py".to_string(),
        ));
        signals.naming.class_names.push((
            "OrderRepository".to_string(),
            NamingCase::PascalCase,
            "repo.py".to_string(),
        ));

        let pattern = signals_to_pattern(&signals).unwrap();
        assert_eq!(pattern.classes, NamingConvention::PascalCase);
    }

    #[test]
    fn test_upper_snake_case_constants_detected() {
        let mut signals = PatternSignals::default();
        signals.naming.constant_names.push((
            "MAX_RETRY_COUNT".to_string(),
            NamingCase::UpperSnakeCase,
            "config.py".to_string(),
        ));
        signals.naming.constant_names.push((
            "DEFAULT_TIMEOUT".to_string(),
            NamingCase::UpperSnakeCase,
            "config.py".to_string(),
        ));

        let pattern = signals_to_pattern(&signals).unwrap();
        assert_eq!(pattern.constants, NamingConvention::UpperSnakeCase);
    }

    #[test]
    fn test_violation_detected() {
        let mut signals = PatternSignals::default();
        signals.naming.function_names.push((
            "find_user".to_string(),
            NamingCase::SnakeCase,
            "service.py".to_string(),
        ));
        signals.naming.function_names.push((
            "getUser".to_string(), // Violation!
            NamingCase::CamelCase,
            "service.py".to_string(),
        ));
        signals.naming.function_names.push((
            "create_user".to_string(),
            NamingCase::SnakeCase,
            "service.py".to_string(),
        ));

        let pattern = signals_to_pattern(&signals).unwrap();
        assert!(!pattern.violations.is_empty());
        assert_eq!(pattern.violations[0].name, "getUser");
        assert_eq!(pattern.violations[0].expected, NamingConvention::SnakeCase);
        assert_eq!(pattern.violations[0].actual, NamingConvention::CamelCase);
    }
}
