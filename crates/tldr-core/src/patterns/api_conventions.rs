//! API convention pattern detection
//!
//! Detects API patterns:
//! - Framework usage (FastAPI, Flask, Express, etc.)
//! - RESTful patterns
//! - ORM usage (SQLAlchemy, Prisma, GORM, etc.)
//! - GraphQL definitions

use super::signals::PatternSignals;
use crate::types::ApiConventionPattern;

/// Convert signals to API convention pattern
pub fn signals_to_pattern(
    signals: &PatternSignals,
    evidence_limit: usize,
) -> Option<ApiConventionPattern> {
    let api_conventions = &signals.api_conventions;

    if !api_conventions.has_signals() {
        return None;
    }

    let confidence = api_conventions.calculate_confidence();

    // Detect framework
    let framework = api_conventions.detect_framework();

    // Detect patterns
    let mut patterns = Vec::new();

    if !api_conventions.fastapi_decorators.is_empty()
        || !api_conventions.flask_decorators.is_empty()
        || !api_conventions.express_routes.is_empty()
    {
        patterns.push("rest_crud".to_string());
    }

    if !api_conventions.restful_patterns.is_empty() {
        patterns.push("restful_naming".to_string());
    }

    if !api_conventions.graphql_defs.is_empty() {
        patterns.push("graphql".to_string());
    }

    // Detect ORM
    let orm_usage = api_conventions.detect_orm();

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        api_conventions
            .fastapi_decorators
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        api_conventions
            .flask_decorators
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        api_conventions
            .express_routes
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        api_conventions
            .restful_patterns
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        api_conventions
            .orm_models
            .iter()
            .take(evidence_limit)
            .map(|(_, e)| e.clone()),
    );
    evidence.truncate(evidence_limit);

    Some(ApiConventionPattern {
        confidence,
        framework,
        patterns,
        orm_usage,
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
    fn test_fastapi_detected() {
        let mut signals = PatternSignals::default();
        signals
            .api_conventions
            .fastapi_decorators
            .push(Evidence::new("routes.py", 10, "@app.get('/users')"));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.framework, Some("fastapi".to_string()));
        assert!(pattern.patterns.contains(&"rest_crud".to_string()));
    }

    #[test]
    fn test_flask_detected() {
        let mut signals = PatternSignals::default();
        signals.api_conventions.flask_decorators.push(Evidence::new(
            "routes.py",
            10,
            "@app.route('/users', methods=['GET'])",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.framework, Some("flask".to_string()));
    }

    #[test]
    fn test_express_detected() {
        let mut signals = PatternSignals::default();
        signals.api_conventions.express_routes.push(Evidence::new(
            "routes.ts",
            10,
            "app.get('/users', (req, res) => { ... })",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.framework, Some("express".to_string()));
    }

    #[test]
    fn test_orm_detected() {
        let mut signals = PatternSignals::default();
        signals.api_conventions.orm_models.push((
            "sqlalchemy".to_string(),
            Evidence::new("models.py", 5, "class User(Base):"),
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert_eq!(pattern.orm_usage, Some("sqlalchemy".to_string()));
    }
}
