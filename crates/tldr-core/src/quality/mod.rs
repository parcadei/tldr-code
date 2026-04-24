//! Code quality analysis module - Phase 8
//!
//! This module provides code quality metrics and smell detection:
//! - Code smells (God Class, Long Method, Long Parameter List)
//! - Maintainability Index (Microsoft formula with Halstead metrics)
//! - Code churn analysis (git history metrics)
//! - Technical debt analysis (SQALE method)
//! - Health analysis (comprehensive code health dashboard)
//! - Complexity analysis (cyclomatic complexity with hotspot detection)
//! - Cohesion analysis (LCOM4 class cohesion metrics)
//! - Dead code analysis (unreachable function detection)
//! - Martin metrics (package coupling metrics: Ca, Ce, I, A, D)
//! - Coupling analysis (pairwise module coupling detection)
//! - Similarity analysis (function clone detection with parallelization)
//! - Coverage parsing (Cobertura XML, LCOV, coverage.py JSON) - Session 15
//! - Hotspots analysis (churn x complexity) - Session 15
//!
//! # References
//! - Microsoft Maintainability Index whitepaper
//! - Martin Fowler's "Refactoring"
//! - SQALE Method (Software Quality Assessment based on Lifecycle Expectations)
//! - Robert Martin's Package Coupling Metrics
//! - Chidamber & Kemerer LCOM4 Metrics

pub mod churn;
pub mod cohesion;
pub mod complexity;
pub mod coupling;
pub mod coverage;
pub mod dead_code;
pub mod debt;
pub mod health;
pub mod hotspots;
pub mod maintainability;
pub mod martin;
pub mod similarity;
pub mod smells;

#[cfg(test)]
mod churn_tests;

#[cfg(test)]
mod debt_tests;

#[cfg(test)]
mod health_tests;

pub use churn::{
    build_summary, check_shallow_clone, get_author_stats, get_file_churn, get_recommendation,
    is_bot_author, is_git_repository, matches_exclude_pattern, AuthorStats, ChurnError,
    ChurnReport, ChurnSummary, FileChurn, Hotspot,
};
pub use cohesion::{
    analyze_cohesion, analyze_cohesion_with_options, ClassCohesion, CohesionOptions,
    CohesionReport, CohesionSummary, CohesionVerdict, ComponentInfo,
};
pub use complexity::{
    analyze_complexity, ComplexityHotspot, ComplexityOptions, ComplexityReport, ComplexitySummary,
    FunctionComplexity,
};
pub use coupling::{
    analyze_coupling, analyze_coupling_with_graph, build_cycle_membership, compute_ca_ce,
    compute_instability, compute_martin_metrics_from_deps, CallSite, CouplingOptions,
    CouplingReport, CouplingVerdict, MartinMetricsReport, MartinModuleMetrics, MartinOptions,
    MartinSummary, ModuleCoupling,
};
pub use coverage::{
    detect_format, parse_cobertura, parse_coverage, parse_coverage_py_json, parse_lcov,
    CoverageFormat, CoverageOptions, CoverageReport, CoverageSummary, FileCoverage,
    FunctionCoverage, LineCoverage, UncoveredFunction, UncoveredLineRange, UncoveredSummary,
};
pub use dead_code::{
    analyze_dead_code, analyze_dead_code_with_graph, analyze_dead_code_with_refcount,
    DeadCodeReport, DeadCodeSummary, DeadFunction, DeadReason, Visibility,
};
pub use debt::{
    analyze_debt, analyze_file, compute_lcom4, count_loc, find_complexity_issues,
    find_deep_nesting, find_god_classes, find_high_coupling, find_missing_docs, find_todo_comments,
    DebtCategory, DebtIssue, DebtOptions, DebtReport, DebtRule, DebtSummary, FileDebt,
};
pub use health::{
    run_health, HealthOptions, HealthReport, HealthSummary, Severity, SubAnalysisResult,
};
pub use hotspots::{
    analyze_hotspots, calculate_trend, composite_score_weighted, has_variance,
    knowledge_fragmentation, normalize_value, percentile_ranks, recency_weight, relative_churn,
    HotspotEntry, HotspotsError, HotspotsMetadata, HotspotsOptions, HotspotsReport,
    HotspotsSummary, ScoringWeights, TrendDirection, TrendInfo,
};
pub use maintainability::{
    maintainability_index, FileMI, HalsteadMetrics, MISummary, MaintainabilityReport,
};
pub use martin::{
    compute_martin_metrics, MartinReport, MetricsHealth, MetricsProblems, MetricsSummary,
    PackageMetrics, Zone,
};
pub use similarity::{
    find_similar, find_similar_with_options, FunctionRef as SimilarityFunctionRef, SimilarPair,
    SimilarityOptions, SimilarityReason, SimilarityReport,
};
pub use smells::{
    analyze_smells_aggregated, analyze_smells_aggregated_with_walker_opts, detect_smells,
    detect_smells_with_walker_opts, SmellFinding, SmellType, SmellsReport, SmellsWalkerOpts,
    ThresholdPreset,
};
pub use smells::{
    detect_data_classes, detect_deep_nesting, detect_lazy_elements, detect_message_chains,
    detect_primitive_obsession,
};
