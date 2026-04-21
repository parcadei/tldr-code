//! Code metrics module - Layer 3
//!
//! This module provides code complexity metrics:
//! - Cyclomatic complexity (McCabe)
//! - Cognitive complexity (SonarSource)
//! - Lines of code
//! - Nesting depth
//!
//! # Session 15 Additions
//!
//! New metrics types and utilities:
//! - `types` - Shared metric data structures (LocInfo, CognitiveInfo, etc.)
//! - `file_utils` - File handling utilities (size checks, binary detection)
//!
//! Session 15 Phase 2 additions:
//! - `loc` - Lines of code analysis with language-aware comment detection
//!
//! Session 15 Phase 3 additions:
//! - `cognitive` - Cognitive complexity with SonarQube algorithm
//!
//! Session 15 Phase 5 additions:
//! - `halstead` - Standalone Halstead software science metrics per function
//!
//! # References
//! - McCabe, T.J. (1976). "A Complexity Measure"
//! - SonarSource Cognitive Complexity whitepaper
//! - Halstead, M.H. (1977). "Elements of Software Science"

pub mod cognitive;
pub mod complexity;
pub mod file_utils;
pub mod halstead;
pub mod loc;
pub mod types;

pub use complexity::{
    calculate_all_complexities, calculate_all_complexities_file,
    calculate_all_complexities_from_tree, calculate_complexity,
};

// Re-export LOC analysis functions
pub use loc::{analyze_loc, count_lines, LocOptions, LocReport};

// Re-export cognitive complexity analysis
pub use cognitive::{
    analyze_cognitive, analyze_cognitive_source, merge_cognitive_reports, CognitiveOptions,
    CognitiveReport, FunctionCognitive, ThresholdStatus as CognitiveThresholdStatus,
};

// Re-export Halstead analysis
pub use halstead::{
    analyze_halstead, classify_tokens, compute_halstead, merge_halstead_reports, FunctionHalstead,
    HalsteadOptions, HalsteadReport, HalsteadSummary, HalsteadThresholds, HalsteadViolation,
    ThresholdStatus as HalsteadThresholdStatus,
};

// Re-export types for convenience
pub use types::{
    CognitiveContributor, CognitiveInfo, CoverageInfo, HalsteadInfo, HotspotInfo, HotspotTrend,
    LocInfo, ThresholdViolation,
};

// Re-export file utilities
pub use file_utils::{
    check_file_size, contains_path_traversal, has_binary_extension, is_binary_file,
    is_path_within_project, is_symlink, resolve_symlink_safely, should_exclude, should_skip_path,
    skip_directories, walk_source_files, WalkOptions, DEFAULT_MAX_FILE_SIZE,
    DEFAULT_MAX_FILE_SIZE_MB,
};
