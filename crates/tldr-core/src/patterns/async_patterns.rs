//! Async/concurrency pattern detection
//!
//! Detects async patterns:
//! - async/await keywords
//! - Go goroutines (go keyword)
//! - Tokio runtime usage
//! - Sync primitives (mutex, channel, semaphore)

use super::signals::PatternSignals;
use crate::types::AsyncPattern;

/// Convert signals to async pattern
pub fn signals_to_pattern(signals: &PatternSignals, evidence_limit: usize) -> Option<AsyncPattern> {
    let async_patterns = &signals.async_patterns;

    if !async_patterns.has_signals() {
        return None;
    }

    let concurrency_confidence = async_patterns.calculate_confidence();

    // Detect patterns
    let mut patterns = Vec::new();

    if !async_patterns.async_await.is_empty() {
        patterns.push("async_await".to_string());
    }

    if !async_patterns.goroutines.is_empty() {
        patterns.push("goroutines".to_string());
    }

    if !async_patterns.tokio_usage.is_empty() {
        patterns.push("tokio".to_string());
    }

    if !async_patterns.thread_spawns.is_empty() {
        patterns.push("thread_spawn".to_string());
    }

    // Collect sync primitives
    let mut sync_primitives: Vec<String> = async_patterns
        .sync_primitives
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    sync_primitives.sort();
    sync_primitives.dedup();

    // Collect evidence (limited)
    let mut evidence = Vec::new();
    evidence.extend(
        async_patterns
            .async_await
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        async_patterns
            .goroutines
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        async_patterns
            .tokio_usage
            .iter()
            .take(evidence_limit)
            .cloned(),
    );
    evidence.extend(
        async_patterns
            .sync_primitives
            .iter()
            .take(evidence_limit)
            .map(|(_, e)| e.clone()),
    );
    evidence.truncate(evidence_limit);

    Some(AsyncPattern {
        concurrency_confidence,
        patterns,
        sync_primitives,
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
    fn test_async_await_detected() {
        let mut signals = PatternSignals::default();
        signals.async_patterns.async_await.push(Evidence::new(
            "service.py",
            10,
            "async def fetch_data():",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"async_await".to_string()));
    }

    #[test]
    fn test_goroutines_detected() {
        let mut signals = PatternSignals::default();
        signals.async_patterns.goroutines.push(Evidence::new(
            "main.go",
            15,
            "go handleRequest(req)",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"goroutines".to_string()));
    }

    #[test]
    fn test_tokio_detected() {
        let mut signals = PatternSignals::default();
        signals.async_patterns.tokio_usage.push(Evidence::new(
            "main.rs",
            5,
            "use tokio::runtime::Runtime;",
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.patterns.contains(&"tokio".to_string()));
    }

    #[test]
    fn test_sync_primitives_detected() {
        let mut signals = PatternSignals::default();
        signals.async_patterns.sync_primitives.push((
            "mutex".to_string(),
            Evidence::new("lib.rs", 10, "let lock = Mutex::new(0);"),
        ));
        signals.async_patterns.sync_primitives.push((
            "channel".to_string(),
            Evidence::new("lib.rs", 15, "let (tx, rx) = mpsc::channel();"),
        ));

        let pattern = signals_to_pattern(&signals, 3).unwrap();
        assert!(pattern.sync_primitives.contains(&"mutex".to_string()));
        assert!(pattern.sync_primitives.contains(&"channel".to_string()));
    }
}
