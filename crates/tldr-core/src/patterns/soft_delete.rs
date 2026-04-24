//! Soft delete pattern detection
//!
//! Detects fields and patterns related to soft delete:
//! - is_deleted boolean fields
//! - deleted_at timestamp fields
//! - ORM paranoid mode annotations
//! - Query filters on delete fields

use super::signals::PatternSignals;
use crate::types::SoftDeletePattern;

/// Convert signals to soft delete pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<SoftDeletePattern> {
    let soft_delete = &signals.soft_delete;

    if !soft_delete.has_signals() {
        return None;
    }

    let confidence = soft_delete.calculate_confidence();

    // Collect column names
    let mut column_names = Vec::new();
    if !soft_delete.is_deleted_fields.is_empty() {
        column_names.push("is_deleted".to_string());
    }
    if !soft_delete.deleted_at_fields.is_empty() {
        column_names.push("deleted_at".to_string());
    }

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        soft_delete
            .is_deleted_fields
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        soft_delete
            .deleted_at_fields
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        soft_delete
            .delete_query_filters
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        soft_delete
            .paranoid_annotations
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.truncate(evidence_limit);

    Some(SoftDeletePattern {
        detected: true,
        confidence,
        column_names,
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
    fn test_is_deleted_field_detected() {
        let mut signals = PatternSignals::default();
        signals.soft_delete.is_deleted_fields.push(Evidence::new(
            "models/user.py",
            15,
            "is_deleted = Column(Boolean)",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.detected);
        assert!(pattern.confidence >= 0.4);
        assert!(pattern.column_names.contains(&"is_deleted".to_string()));
    }

    #[test]
    fn test_both_fields_high_confidence() {
        let mut signals = PatternSignals::default();
        signals.soft_delete.is_deleted_fields.push(Evidence::new(
            "models/user.py",
            15,
            "is_deleted = Column(Boolean)",
        ));
        signals.soft_delete.deleted_at_fields.push(Evidence::new(
            "models/user.py",
            16,
            "deleted_at = Column(DateTime)",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.confidence >= 0.8);
        assert!(pattern.column_names.contains(&"is_deleted".to_string()));
        assert!(pattern.column_names.contains(&"deleted_at".to_string()));
    }
}
