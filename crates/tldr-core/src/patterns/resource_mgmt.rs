//! Resource management pattern detection
//!
//! Detects resource cleanup patterns:
//! - Python: context managers (with), __enter__/__exit__
//! - Go: defer statements
//! - Rust: Drop trait implementations (RAII)
//! - TypeScript/JS: try/finally blocks

use super::signals::PatternSignals;
use crate::types::ResourceManagementPattern;

/// Convert signals to resource management pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<ResourceManagementPattern> {
    let resource_mgmt = &signals.resource_management;

    if !resource_mgmt.has_signals() {
        return None;
    }

    let confidence = resource_mgmt.calculate_confidence();

    // Detect patterns
    let mut patterns = Vec::new();

    if !resource_mgmt.context_managers.is_empty() || !resource_mgmt.enter_exit_methods.is_empty() {
        patterns.push("context_manager".to_string());
    }

    if !resource_mgmt.defer_statements.is_empty() {
        patterns.push("defer".to_string());
    }

    if !resource_mgmt.drop_impls.is_empty() {
        patterns.push("raii".to_string());
    }

    if !resource_mgmt.try_finally_blocks.is_empty() {
        patterns.push("finally".to_string());
    }

    if !resource_mgmt.close_calls.is_empty() {
        patterns.push("explicit_close".to_string());
    }

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        resource_mgmt
            .context_managers
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        resource_mgmt
            .enter_exit_methods
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        resource_mgmt
            .defer_statements
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        resource_mgmt
            .drop_impls
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        resource_mgmt
            .try_finally_blocks
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.truncate(evidence_limit);

    Some(ResourceManagementPattern {
        confidence,
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
    fn test_context_manager_detected() {
        let mut signals = PatternSignals::default();
        signals
            .resource_management
            .context_managers
            .push(Evidence::new(
                "file_utils.py",
                10,
                "with open('file.txt') as f:",
            ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.confidence >= 0.4);
        assert!(pattern.patterns.contains(&"context_manager".to_string()));
    }

    #[test]
    fn test_defer_detected() {
        let mut signals = PatternSignals::default();
        signals
            .resource_management
            .defer_statements
            .push(Evidence::new("file.go", 15, "defer f.Close()"));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"defer".to_string()));
    }

    #[test]
    fn test_raii_detected() {
        let mut signals = PatternSignals::default();
        signals.resource_management.drop_impls.push(Evidence::new(
            "lib.rs",
            20,
            "impl Drop for FileHandle { fn drop(&mut self) { ... } }",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"raii".to_string()));
    }
}
