//! L2 deep-analysis engine framework for bugbot.
//!
//! This module provides the trait, types, and registry for L2 engines that
//! perform deeper static analysis beyond diff-level heuristics. Each engine
//! targets specific finding types (e.g., null-deref, use-after-move).
//!
//! # Architecture
//!
//! ```text
//! l2_engine_registry() -> Vec<Box<dyn L2Engine>>
//!       |
//!       v
//!   for engine in engines {
//!       engine.analyze(&ctx) -> L2AnalyzerOutput
//!   }
//! ```

pub mod composition;
pub mod context;
pub mod daemon_client;
pub mod dedup;
pub mod engines;
pub mod findings;
pub mod ir;
pub mod types;

pub use context::L2Context;
pub use types::*;


/// Trait for L2 deep-analysis engines.
///
/// Implementations must be object-safe so they can be stored as
/// `Box<dyn L2Engine>` in the engine registry. Each engine declares:
///
/// - A unique name for logging and identification
/// - The finding types it can produce (e.g., `["null-deref", "uninitialized-read"]`)
/// - Which languages it supports (empty means language-agnostic)
/// - The analysis entry point
pub trait L2Engine: Send + Sync {
    /// Unique human-readable name for this engine (used in logging and reports).
    fn name(&self) -> &'static str;

    /// The set of finding type identifiers this engine can produce.
    fn finding_types(&self) -> &[&'static str];

    /// Languages this engine supports. An empty slice means language-agnostic
    /// (the engine handles all languages or performs language-independent analysis).
    fn languages(&self) -> &[tldr_core::Language] {
        &[]
    }

    /// Run analysis on the provided context and return findings.
    fn analyze(&self, ctx: &context::L2Context) -> types::L2AnalyzerOutput;
}

/// Returns the set of all registered L2 engines.
///
/// Contains the TldrDifferentialEngine that invokes `tldr` CLI commands
/// for differential analysis. The pipeline orchestrator invokes each
/// engine's `analyze` method.
pub fn l2_engine_registry() -> Vec<Box<dyn L2Engine>> {
    vec![
        Box::new(engines::tldr_differential::TldrDifferentialEngine::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registry must contain only TldrDifferentialEngine.
    #[test]
    fn test_l2_engine_registry_contains_engines() {
        let engines = l2_engine_registry();
        assert_eq!(engines.len(), 1, "Registry should contain exactly 1 engine (TldrDifferentialEngine)");
        assert!(
            engines.iter().any(|e| e.name() == "TldrDifferentialEngine"),
            "Registry must contain TldrDifferentialEngine"
        );
    }

    /// TldrDifferentialEngine should be registered and accessible via the registry.
    #[test]
    fn test_tldr_engine_registered() {
        let engines = l2_engine_registry();
        let engine = engines.iter().find(|e| e.name() == "TldrDifferentialEngine");
        assert!(engine.is_some(), "TldrDifferentialEngine must be in registry");
        let engine = engine.unwrap();
        assert_eq!(engine.finding_types().len(), 11);
    }

    /// Verify L2Engine is object-safe by constructing a mock implementation,
    /// storing it as `Box<dyn L2Engine>`, and calling every trait method.
    #[test]
    fn test_l2_engine_trait_object_safe() {
        struct MockEngine;

        impl L2Engine for MockEngine {
            fn name(&self) -> &'static str {
                "MockEngine"
            }

            fn finding_types(&self) -> &[&'static str] {
                &["test-finding"]
            }

            fn analyze(&self, _ctx: &context::L2Context) -> types::L2AnalyzerOutput {
                types::L2AnalyzerOutput {
                    findings: vec![],
                    status: types::AnalyzerStatus::Complete,
                    duration_ms: 0,
                    functions_analyzed: 0,
                    functions_skipped: 0,
                }
            }
        }

        let engine: Box<dyn L2Engine> = Box::new(MockEngine);

        assert_eq!(engine.name(), "MockEngine");
        assert_eq!(engine.finding_types(), &["test-finding"]);
        assert!(engine.languages().is_empty());

        // Construct a minimal L2Context for the analyze call
        let ctx = context::L2Context::new(
            std::path::PathBuf::from("/tmp/test"),
            tldr_core::Language::Rust,
            vec![],
            context::FunctionDiff {
                changed: vec![],
                inserted: vec![],
                deleted: vec![],
            },
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );
        let output = engine.analyze(&ctx);
        assert!(output.findings.is_empty());
        assert_eq!(output.status, types::AnalyzerStatus::Complete);
        assert_eq!(output.duration_ms, 0);
        assert_eq!(output.functions_analyzed, 0);
        assert_eq!(output.functions_skipped, 0);
    }
}
