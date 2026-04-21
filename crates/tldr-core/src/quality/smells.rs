//! Code smell detection
//!
//! Implements detection of common code smells as per spec Section 2.8.1:
//! - God Class: >20 methods or >500 LOC
//! - Long Method: >50 LOC or cyclomatic complexity >10
//! - Long Parameter List: >5 parameters
//!
//! # Example
//! ```ignore
//! use tldr_core::quality::smells::{detect_smells, ThresholdPreset};
//!
//! let report = detect_smells(Path::new("src/"), ThresholdPreset::Default, None, false)?;
//! for smell in &report.smells {
//!     println!("{}: {} in {}", smell.smell_type, smell.name, smell.file.display());
//! }
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::collections::HashMap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::ast::extract::extract_file;
use crate::ast::parser::ParserPool;
use crate::callgraph::cross_file_types::{CallGraphIR, CallSite, CallType, FileIR, FuncDef};
use crate::metrics::calculate_all_complexities_file;
use crate::types::Language;
use crate::types::inheritance::InheritanceReport;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Code smell types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmellType {
    /// Class with too many methods or LOC (>20 methods or >500 LOC)
    GodClass,
    /// Method with too many lines or too high complexity (>50 LOC or cyclomatic >10)
    LongMethod,
    /// Function with too many parameters (>5)
    LongParameterList,
    /// Feature Envy - method uses another class's data more than its own
    FeatureEnvy,
    /// Data Clumps - same group of data items appearing together
    DataClumps,
    /// Class with low cohesion (LCOM4 >= 2) - pulled from cohesion analyzer
    LowCohesion,
    /// Modules with tight coupling (score >= 0.6) - pulled from coupling analyzer
    TightCoupling,
    /// Unreachable functions - pulled from dead code analyzer
    DeadCode,
    /// Duplicate code blocks - pulled from similarity analyzer
    CodeClone,
    /// Functions with high cognitive complexity (>= 15) - pulled from complexity analyzer
    HighCognitiveComplexity,
    /// Function with nesting depth > 4 (nested control flow)
    DeepNesting,
    /// Class with many fields but few/no methods (just a data bag)
    DataClass,
    /// Class with only 1 method and 0-1 fields (too trivial for its own class)
    LazyElement,
    /// Long chains of method calls (a.b().c().d().e()) - high coupling to structure
    MessageChain,
    /// Function with many primitive-typed parameters instead of domain objects
    PrimitiveObsession,
    /// Class where >50% methods just delegate to another class
    MiddleMan,
    /// Subclass using <20% of inherited methods
    RefusedBequest,
    /// Two classes with bidirectional internal access
    InappropriateIntimacy,
}

impl std::fmt::Display for SmellType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmellType::GodClass => write!(f, "God Class"),
            SmellType::LongMethod => write!(f, "Long Method"),
            SmellType::LongParameterList => write!(f, "Long Parameter List"),
            SmellType::FeatureEnvy => write!(f, "Feature Envy"),
            SmellType::DataClumps => write!(f, "Data Clumps"),
            SmellType::LowCohesion => write!(f, "Low Cohesion"),
            SmellType::TightCoupling => write!(f, "Tight Coupling"),
            SmellType::DeadCode => write!(f, "Dead Code"),
            SmellType::CodeClone => write!(f, "Code Clone"),
            SmellType::HighCognitiveComplexity => write!(f, "High Cognitive Complexity"),
            SmellType::DeepNesting => write!(f, "Deep Nesting"),
            SmellType::DataClass => write!(f, "Data Class"),
            SmellType::LazyElement => write!(f, "Lazy Element"),
            SmellType::MessageChain => write!(f, "Message Chain"),
            SmellType::PrimitiveObsession => write!(f, "Primitive Obsession"),
            SmellType::MiddleMan => write!(f, "Middle Man"),
            SmellType::RefusedBequest => write!(f, "Refused Bequest"),
            SmellType::InappropriateIntimacy => write!(f, "Inappropriate Intimacy"),
        }
    }
}

/// Threshold presets for smell detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdPreset {
    /// Strict thresholds for high-quality codebases
    Strict,
    /// Default thresholds (recommended)
    #[default]
    Default,
    /// Relaxed thresholds for legacy code
    Relaxed,
}

// =============================================================================
// Tier-2 Threshold Constants
// =============================================================================

/// Middle Man delegation ratio — Fowler's Refactoring (60% delegation = smell)
const MM_DELEGATION_RATIO_STRICT: f64 = 0.50;
const MM_DELEGATION_RATIO_DEFAULT: f64 = 0.60;
const MM_DELEGATION_RATIO_RELAXED: f64 = 0.75;
const MM_MIN_METHODS: usize = 3;

/// Refused Bequest usage ratio — Marinescu's BUR metric (<33% = smell)
const RB_USAGE_RATIO_STRICT: f64 = 0.33;
const RB_USAGE_RATIO_DEFAULT: f64 = 0.33;
const RB_USAGE_RATIO_RELAXED: f64 = 0.15;
const RB_MIN_INHERITED_STRICT: usize = 3;
const RB_MIN_INHERITED_DEFAULT: usize = 3;
const RB_MIN_INHERITED_RELAXED: usize = 5;

/// Feature Envy — adapted from Lanza-Marinescu ATFD metric
const FE_MIN_FOREIGN_STRICT: usize = 3;
const FE_MIN_FOREIGN_DEFAULT: usize = 4;
const FE_MIN_FOREIGN_RELAXED: usize = 5;
const FE_RATIO_STRICT: f64 = 1.5;
const FE_RATIO_DEFAULT: f64 = 2.0;
const FE_RATIO_RELAXED: f64 = 3.0;

/// Inappropriate Intimacy — adapted from CodeQL bidirectional coupling
const II_MIN_TOTAL_STRICT: usize = 6;
const II_MIN_TOTAL_DEFAULT: usize = 10;
const II_MIN_TOTAL_RELAXED: usize = 15;
const II_MIN_PER_DIR_STRICT: usize = 2;
const II_MIN_PER_DIR_DEFAULT: usize = 3;
const II_MIN_PER_DIR_RELAXED: usize = 4;

/// Thresholds for code smell detection
#[derive(Debug, Clone)]
pub struct Thresholds {
    /// Max methods in a class before God Class
    pub god_class_methods: usize,
    /// Max LOC in a class before God Class
    pub god_class_loc: usize,
    /// Max LOC in a method before Long Method
    pub long_method_loc: usize,
    /// Max cyclomatic complexity before Long Method
    pub long_method_complexity: u32,
    /// Max parameters before Long Parameter List
    pub long_param_count: usize,
    // Tier-2: Middle Man
    /// Minimum delegation ratio (non-constructor methods that are pure delegators)
    pub middle_man_delegation_ratio: f64,
    /// Minimum number of non-constructor methods before Middle Man is checked
    pub middle_man_min_methods: usize,
    // Tier-2: Refused Bequest
    /// Maximum usage ratio below which Refused Bequest triggers
    pub refused_bequest_usage_ratio: f64,
    /// Minimum inherited methods before Refused Bequest is checked
    pub refused_bequest_min_inherited: usize,
    // Tier-2: Feature Envy
    /// Minimum foreign accesses before Feature Envy is checked
    pub feature_envy_min_foreign: usize,
    /// Minimum ratio of foreign-to-own accesses
    pub feature_envy_ratio: f64,
    // Tier-2: Inappropriate Intimacy
    /// Minimum total bidirectional accesses
    pub intimacy_min_total: usize,
    /// Minimum accesses per direction
    pub intimacy_min_per_direction: usize,
}

impl Thresholds {
    /// Get thresholds for a preset
    pub fn from_preset(preset: ThresholdPreset) -> Self {
        match preset {
            ThresholdPreset::Strict => Self {
                god_class_methods: 10,
                god_class_loc: 250,
                long_method_loc: 25,
                long_method_complexity: 5,
                long_param_count: 3,
                // Tier-2: Strict
                middle_man_delegation_ratio: MM_DELEGATION_RATIO_STRICT,
                middle_man_min_methods: MM_MIN_METHODS,
                refused_bequest_usage_ratio: RB_USAGE_RATIO_STRICT,
                refused_bequest_min_inherited: RB_MIN_INHERITED_STRICT,
                feature_envy_min_foreign: FE_MIN_FOREIGN_STRICT,
                feature_envy_ratio: FE_RATIO_STRICT,
                intimacy_min_total: II_MIN_TOTAL_STRICT,
                intimacy_min_per_direction: II_MIN_PER_DIR_STRICT,
            },
            ThresholdPreset::Default => Self {
                god_class_methods: 20,
                god_class_loc: 500,
                long_method_loc: 50,
                long_method_complexity: 10,
                long_param_count: 5,
                // Tier-2: Default
                middle_man_delegation_ratio: MM_DELEGATION_RATIO_DEFAULT,
                middle_man_min_methods: MM_MIN_METHODS,
                refused_bequest_usage_ratio: RB_USAGE_RATIO_DEFAULT,
                refused_bequest_min_inherited: RB_MIN_INHERITED_DEFAULT,
                feature_envy_min_foreign: FE_MIN_FOREIGN_DEFAULT,
                feature_envy_ratio: FE_RATIO_DEFAULT,
                intimacy_min_total: II_MIN_TOTAL_DEFAULT,
                intimacy_min_per_direction: II_MIN_PER_DIR_DEFAULT,
            },
            ThresholdPreset::Relaxed => Self {
                god_class_methods: 30,
                god_class_loc: 1000,
                long_method_loc: 100,
                long_method_complexity: 15,
                long_param_count: 7,
                // Tier-2: Relaxed
                middle_man_delegation_ratio: MM_DELEGATION_RATIO_RELAXED,
                middle_man_min_methods: MM_MIN_METHODS,
                refused_bequest_usage_ratio: RB_USAGE_RATIO_RELAXED,
                refused_bequest_min_inherited: RB_MIN_INHERITED_RELAXED,
                feature_envy_min_foreign: FE_MIN_FOREIGN_RELAXED,
                feature_envy_ratio: FE_RATIO_RELAXED,
                intimacy_min_total: II_MIN_TOTAL_RELAXED,
                intimacy_min_per_direction: II_MIN_PER_DIR_RELAXED,
            },
        }
    }
}

/// A single code smell finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellFinding {
    /// Type of smell detected
    pub smell_type: SmellType,
    /// File containing the smell
    pub file: PathBuf,
    /// Name of the affected element (class or function)
    pub name: String,
    /// Line number where the smell starts
    pub line: u32,
    /// Human-readable reason for the smell
    pub reason: String,
    /// Severity level (1-3, higher is worse)
    pub severity: u8,
    /// Suggestion for fixing (only if requested)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Report from smell detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellsReport {
    /// All detected smells
    pub smells: Vec<SmellFinding>,
    /// Number of files scanned
    pub files_scanned: usize,
    /// Smells grouped by file
    pub by_file: HashMap<PathBuf, Vec<SmellFinding>>,
    /// Summary statistics
    pub summary: SmellsSummary,
}

/// Summary statistics for smell detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellsSummary {
    /// Total number of smells found
    pub total_smells: usize,
    /// Count by smell type
    pub by_type: HashMap<String, usize>,
    /// Average smells per file
    pub avg_smells_per_file: f64,
}

// =============================================================================
// Main API
// =============================================================================

/// Detect code smells in a file or directory
///
/// # Arguments
/// * `path` - File or directory to scan
/// * `threshold` - Threshold preset (Strict, Default, Relaxed)
/// * `smell_type` - Optional filter for specific smell type
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// * `Ok(SmellsReport)` - Report with all detected smells
/// * `Err(TldrError)` - On file system or parse errors
///
/// # Example
/// ```ignore
/// use tldr_core::quality::smells::{detect_smells, ThresholdPreset, SmellType};
///
/// // Scan with default thresholds
/// let report = detect_smells(Path::new("src/"), ThresholdPreset::Default, None, false)?;
///
/// // Scan for specific smell type with suggestions
/// let report = detect_smells(
///     Path::new("src/"),
///     ThresholdPreset::Strict,
///     Some(SmellType::GodClass),
///     true,
/// )?;
/// ```
pub fn detect_smells(
    path: &Path,
    threshold: ThresholdPreset,
    smell_type: Option<SmellType>,
    suggest: bool,
) -> TldrResult<SmellsReport> {
    let thresholds = Thresholds::from_preset(threshold);
    // Max file size to analyze (500KB) - skip minified/generated files
    const MAX_FILE_SIZE: u64 = 500 * 1024;

    // Collect files to scan
    let files: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| Language::from_path(e.path()).is_some())
            .filter(|e| e.metadata().map(|m| m.len() <= MAX_FILE_SIZE).unwrap_or(true))
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Analyze files in parallel using rayon
    let file_results: Vec<Vec<SmellFinding>> = files
        .par_iter()
        .filter_map(|file_path| {
            analyze_file(file_path, &thresholds, smell_type, suggest).ok()
        })
        .collect();

    let files_scanned = file_results.len();
    let smells: Vec<SmellFinding> = file_results.into_iter().flatten().collect();

    // Group by file
    let mut by_file: HashMap<PathBuf, Vec<SmellFinding>> = HashMap::new();
    for smell in &smells {
        by_file
            .entry(smell.file.clone())
            .or_default()
            .push(smell.clone());
    }

    // Calculate summary
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for smell in &smells {
        *by_type.entry(smell.smell_type.to_string()).or_insert(0) += 1;
    }

    let summary = SmellsSummary {
        total_smells: smells.len(),
        by_type,
        avg_smells_per_file: if files_scanned > 0 {
            smells.len() as f64 / files_scanned as f64
        } else {
            0.0
        },
    };

    Ok(SmellsReport {
        smells,
        files_scanned,
        by_file,
        summary,
    })
}

// =============================================================================
// Internal Implementation
// =============================================================================

/// Analyze a single file for smells
fn analyze_file(
    path: &Path,
    thresholds: &Thresholds,
    smell_filter: Option<SmellType>,
    suggest: bool,
) -> TldrResult<Vec<SmellFinding>> {
    let mut smells = Vec::new();

    let module_info = extract_file(path, None)?;

    if should_analyze_smell(smell_filter, SmellType::GodClass) {
        collect_god_class_smells(path, &module_info.classes, thresholds, suggest, &mut smells);
    }

    let complexity_map = calculate_all_complexities_file(path).unwrap_or_default();
    let all_functions = module_info.functions.iter()
        .chain(module_info.classes.iter().flat_map(|c| c.methods.iter()));
    for func in all_functions {
        if should_analyze_smell(smell_filter, SmellType::LongParameterList) {
            maybe_add_long_parameter_smell(path, func, thresholds, suggest, &mut smells);
        }
        if should_analyze_smell(smell_filter, SmellType::LongMethod) {
            maybe_add_long_method_smell(
                path,
                func,
                thresholds,
                suggest,
                &complexity_map,
                &mut smells,
            );
        }
    }

    let source = std::fs::read_to_string(path).unwrap_or_default();
    let lang_str = Language::from_path(path)
        .map(|l| format!("{:?}", l).to_lowercase())
        .unwrap_or_default();
    collect_tier1_ast_smells(
        path,
        &source,
        &lang_str,
        smell_filter,
        suggest,
        &mut smells,
    );

    Ok(smells)
}

fn collect_god_class_smells(
    path: &Path,
    classes: &[crate::types::ClassInfo],
    thresholds: &Thresholds,
    suggest: bool,
    smells: &mut Vec<SmellFinding>,
) {
    for class in classes {
        let method_count = class.methods.len();
        let class_loc = estimate_class_loc(class);
        if method_count > thresholds.god_class_methods {
            smells.push(SmellFinding {
                smell_type: SmellType::GodClass,
                file: path.to_path_buf(),
                name: class.name.clone(),
                line: class.line_number,
                reason: format!(
                    "Class has {} methods (threshold: {})",
                    method_count, thresholds.god_class_methods
                ),
                severity: calculate_severity(method_count, thresholds.god_class_methods),
                suggestion: if suggest {
                    Some("Consider splitting this class into smaller, focused classes using the Single Responsibility Principle".to_string())
                } else {
                    None
                },
            });
            continue;
        }
        if class_loc > thresholds.god_class_loc {
            smells.push(SmellFinding {
                smell_type: SmellType::GodClass,
                file: path.to_path_buf(),
                name: class.name.clone(),
                line: class.line_number,
                reason: format!(
                    "Class has {} lines of code (threshold: {})",
                    class_loc, thresholds.god_class_loc
                ),
                severity: calculate_severity(class_loc, thresholds.god_class_loc),
                suggestion: if suggest {
                    Some("Consider extracting methods and responsibilities into separate classes".to_string())
                } else {
                    None
                },
            });
        }
    }
}

fn maybe_add_long_parameter_smell(
    path: &Path,
    func: &crate::types::FunctionInfo,
    thresholds: &Thresholds,
    suggest: bool,
    smells: &mut Vec<SmellFinding>,
) {
    let param_count = func.params.len();
    if param_count <= thresholds.long_param_count {
        return;
    }
    smells.push(SmellFinding {
        smell_type: SmellType::LongParameterList,
        file: path.to_path_buf(),
        name: func.name.clone(),
        line: func.line_number,
        reason: format!(
            "Function has {} parameters (threshold: {})",
            param_count, thresholds.long_param_count
        ),
        severity: calculate_severity(param_count, thresholds.long_param_count),
        suggestion: if suggest {
            Some("Consider using a parameter object or builder pattern to reduce parameters".to_string())
        } else {
            None
        },
    });
}

fn maybe_add_long_method_smell(
    path: &Path,
    func: &crate::types::FunctionInfo,
    thresholds: &Thresholds,
    suggest: bool,
    complexity_map: &std::collections::HashMap<String, crate::types::ComplexityMetrics>,
    smells: &mut Vec<SmellFinding>,
) {
    let Some(metrics) = complexity_map.get(&func.name) else {
        return;
    };
    if metrics.lines_of_code as usize > thresholds.long_method_loc {
        smells.push(SmellFinding {
            smell_type: SmellType::LongMethod,
            file: path.to_path_buf(),
            name: func.name.clone(),
            line: func.line_number,
            reason: format!(
                "Method has {} lines of code (threshold: {})",
                metrics.lines_of_code, thresholds.long_method_loc
            ),
            severity: calculate_severity(
                metrics.lines_of_code as usize,
                thresholds.long_method_loc,
            ),
            suggestion: if suggest {
                Some("Consider extracting parts of this method into smaller helper methods".to_string())
            } else {
                None
            },
        });
        return;
    }
    if metrics.cyclomatic > thresholds.long_method_complexity {
        smells.push(SmellFinding {
            smell_type: SmellType::LongMethod,
            file: path.to_path_buf(),
            name: func.name.clone(),
            line: func.line_number,
            reason: format!(
                "Method has cyclomatic complexity {} (threshold: {})",
                metrics.cyclomatic, thresholds.long_method_complexity
            ),
            severity: calculate_severity(
                metrics.cyclomatic as usize,
                thresholds.long_method_complexity as usize,
            ),
            suggestion: if suggest {
                Some("Consider simplifying control flow or extracting complex conditions into methods".to_string())
            } else {
                None
            },
        });
    }
}

fn collect_tier1_ast_smells(
    path: &Path,
    source: &str,
    lang_str: &str,
    smell_filter: Option<SmellType>,
    suggest: bool,
    smells: &mut Vec<SmellFinding>,
) {
    if should_analyze_smell(smell_filter, SmellType::DeepNesting) {
        append_ast_findings(
            smells,
            detect_deep_nesting(source, lang_str),
            path,
            suggest,
            "Reduce nesting by extracting inner blocks into helper functions or using early returns",
        );
    }
    if should_analyze_smell(smell_filter, SmellType::DataClass) {
        append_ast_findings(
            smells,
            detect_data_classes(source, lang_str),
            path,
            suggest,
            "Consider adding behavior methods or converting to a plain data structure (dataclass, struct, record)",
        );
    }
    if should_analyze_smell(smell_filter, SmellType::LazyElement) {
        append_ast_findings(
            smells,
            detect_lazy_elements(source, lang_str),
            path,
            suggest,
            "Consider inlining this class into its caller or merging with a related class",
        );
    }
    if should_analyze_smell(smell_filter, SmellType::MessageChain) {
        append_ast_findings(
            smells,
            detect_message_chains(source, lang_str),
            path,
            suggest,
            "Apply the Law of Demeter: hide the chain behind a single method call",
        );
    }
    if should_analyze_smell(smell_filter, SmellType::PrimitiveObsession) {
        append_ast_findings(
            smells,
            detect_primitive_obsession(source, lang_str),
            path,
            suggest,
            "Introduce domain types (value objects) instead of passing raw primitives",
        );
    }
}

fn append_ast_findings(
    smells: &mut Vec<SmellFinding>,
    mut findings: Vec<SmellFinding>,
    path: &Path,
    suggest: bool,
    suggestion: &str,
) {
    for finding in &mut findings {
        finding.file = path.to_path_buf();
        if suggest {
            finding.suggestion = Some(suggestion.to_string());
        }
    }
    smells.extend(findings);
}

/// Estimate LOC for a class based on method line numbers
fn estimate_class_loc(class: &crate::types::ClassInfo) -> usize {
    if class.methods.is_empty() {
        return 0;
    }

    let min_line = class.line_number;
    let max_line = class.methods.iter()
        .map(|m| m.line_number)
        .max()
        .unwrap_or(min_line);

    // Rough estimate: last method line + some buffer
    (max_line - min_line + 20) as usize
}

/// Calculate severity (1-3) based on how much the threshold is exceeded
fn calculate_severity(value: usize, threshold: usize) -> u8 {
    let ratio = value as f64 / threshold as f64;
    if ratio > 2.0 {
        3 // Very severe
    } else if ratio > 1.5 {
        2 // Moderate
    } else {
        1 // Mild
    }
}

// =============================================================================
// Aggregated Severity Helpers
// =============================================================================

/// Calculate severity for low cohesion findings based on LCOM4 value.
///
/// - LCOM4 >= 6: severity 3 (very fragmented class)
/// - LCOM4 >= 4: severity 2 (moderately fragmented)
/// - LCOM4 >= 2: severity 1 (slightly fragmented)
pub(crate) fn cohesion_severity(lcom4: usize) -> u8 {
    if lcom4 >= 6 {
        3
    } else if lcom4 >= 4 {
        2
    } else {
        1
    }
}

/// Calculate severity for tight coupling findings based on coupling score.
///
/// - score >= 0.8: severity 2 (very tight)
/// - score >= 0.6: severity 1 (tight)
pub(crate) fn coupling_severity(score: f64) -> u8 {
    if score >= 0.8 {
        2
    } else {
        1
    }
}

/// Calculate severity for high cognitive complexity findings.
///
/// - cognitive >= 30: severity 3 (extremely complex)
/// - cognitive >= 20: severity 2 (very complex)
/// - cognitive >= 15: severity 1 (complex)
pub(crate) fn cognitive_severity(cognitive: usize) -> u8 {
    if cognitive >= 30 {
        3
    } else if cognitive >= 20 {
        2
    } else {
        1
    }
}

/// Calculate severity for code clone findings based on similarity score.
///
/// - score > 0.8: severity 2 (near-duplicate)
/// - score > 0.6: severity 1 (similar)
pub(crate) fn clone_severity(score: f64) -> u8 {
    if score > 0.8 {
        2
    } else {
        1
    }
}

// =============================================================================
// New Tier-1 Smell Severity Helpers
// =============================================================================

/// Calculate severity for deep nesting findings.
///
/// - depth >= 8: severity 3 (extremely nested)
/// - depth >= 6: severity 2 (very nested)
/// - depth >= 5: severity 1 (nested)
pub(crate) fn nesting_severity(depth: usize) -> u8 {
    if depth >= 8 {
        3
    } else if depth >= 6 {
        2
    } else {
        1
    }
}

/// Calculate severity for data class findings.
///
/// - fields >= 8 AND methods == 0: severity 2 (pure data bag)
/// - fields >= 4 AND methods <= 2: severity 1 (likely data class)
pub(crate) fn data_class_severity(field_count: usize, method_count: usize) -> u8 {
    if field_count >= 8 && method_count == 0 {
        2
    } else {
        1
    }
}

/// Calculate severity for message chain findings.
///
/// - chain >= 6: severity 2 (very long chain)
/// - chain >= 4: severity 1 (long chain)
pub(crate) fn chain_severity(chain_length: usize) -> u8 {
    if chain_length >= 6 {
        2
    } else {
        1
    }
}

/// Calculate severity for primitive obsession findings.
///
/// - primitives >= 6: severity 2 (many primitives)
/// - primitives >= 4: severity 1 (some primitives)
pub(crate) fn primitive_obsession_severity(primitive_count: usize) -> u8 {
    if primitive_count >= 6 {
        2
    } else {
        1
    }
}

// =============================================================================
// Tier-2 Fowler Smell Severity Helpers
// =============================================================================

/// Calculate severity for Middle Man findings based on delegation ratio and count.
///
/// - ratio >= 0.90 AND delegation_count >= 5: severity 3 (near-total delegation)
/// - ratio >= 0.75 OR delegation_count >= 4: severity 2 (heavy delegation)
/// - otherwise: severity 1 (moderate delegation)
pub(crate) fn middle_man_severity(delegation_ratio: f64, delegation_count: usize) -> u8 {
    if delegation_ratio >= 0.90 && delegation_count >= 5 {
        3
    } else if delegation_ratio >= 0.75 || delegation_count >= 4 {
        2
    } else {
        1
    }
}

/// Calculate severity for Refused Bequest findings based on usage ratio and total inherited.
///
/// - usage_ratio == 0.0 AND total_inherited >= 5: severity 3 (uses nothing)
/// - usage_ratio < 0.10 OR (usage_ratio == 0.0 AND total_inherited >= 3): severity 2
/// - otherwise: severity 1
pub(crate) fn refused_bequest_severity(usage_ratio: f64, total_inherited: usize) -> u8 {
    if usage_ratio == 0.0 && total_inherited >= 5 {
        3
    } else if usage_ratio < 0.10 || (usage_ratio == 0.0 && total_inherited >= 3) {
        2
    } else {
        1
    }
}

/// Calculate severity for Feature Envy findings based on foreign vs own access counts.
///
/// - foreign >= 8 AND ratio > 4.0: severity 3 (extreme envy)
/// - foreign >= 5 AND ratio > 2.5: severity 2 (strong envy)
/// - otherwise: severity 1 (mild envy)
pub(crate) fn feature_envy_severity(foreign: usize, own: usize) -> u8 {
    let ratio = foreign as f64 / (own.max(1)) as f64;
    if foreign >= 8 && ratio > 4.0 {
        3
    } else if foreign >= 5 && ratio > 2.5 {
        2
    } else {
        1
    }
}

/// Calculate severity for Inappropriate Intimacy findings based on total accesses
/// and minimum per-direction count.
///
/// - total >= 20 AND min_direction >= 5: severity 3 (extreme intimacy)
/// - total >= 12 AND min_direction >= 3: severity 2 (strong intimacy)
/// - otherwise: severity 1 (mild intimacy)
pub(crate) fn intimacy_severity(total_accesses: usize, min_direction_count: usize) -> u8 {
    if total_accesses >= 20 && min_direction_count >= 5 {
        3
    } else if total_accesses >= 12 && min_direction_count >= 3 {
        2
    } else {
        1
    }
}

// =============================================================================
// Tier-2 Helper Functions
// =============================================================================

/// Get methods for a class, handling Go/Rust where ClassDef.methods is empty.
///
/// Strategy:
/// 1. Try ClassDef.methods first (works for Python, TypeScript, Java)
/// 2. Fall back to filtering FuncDef entries where class_name matches (Go, Rust)
/// 3. Deduplicate by function name
fn get_class_methods_robust<'a>(file_ir: &'a FileIR, class_name: &str) -> Vec<&'a FuncDef> {
    let class_def = file_ir.get_class(class_name);
    let has_methods_list = class_def
        .map(|c| !c.methods.is_empty())
        .unwrap_or(false);

    if has_methods_list {
        // Use ClassDef.methods to find matching FuncDefs
        let method_names: HashSet<&str> = class_def
            .unwrap()
            .methods
            .iter()
            .map(|m| m.as_str())
            .collect();
        let mut seen = HashSet::new();
        file_ir
            .funcs
            .iter()
            .filter(|f| {
                f.class_name.as_deref() == Some(class_name)
                    || method_names.contains(f.name.as_str())
            })
            .filter(|f| seen.insert(&f.name))
            .collect()
    } else {
        // Fallback for Go/Rust: join FuncDef by class_name
        file_ir
            .funcs
            .iter()
            .filter(|f| f.class_name.as_deref() == Some(class_name))
            .collect()
    }
}

/// Returns true if the method name is a constructor for the given language.
///
/// Recognized constructors:
/// - Python: `__init__`
/// - JavaScript/TypeScript/TSX/JSX: `constructor`
/// - Rust: `new`
/// - Go: names starting with `New`
/// - Ruby: `initialize`
/// - PHP: `__construct`
/// - Swift: `init`
/// - Scala: `<init>` or `this`
/// - Java/C#/Kotlin: cannot determine without class name (returns false)
/// - C/C++: cannot determine without class name (returns false)
/// - Elixir/Lua: no traditional constructors (returns false)
fn is_constructor(name: &str, language: &str) -> bool {
    match language {
        "python" | "py" => name == "__init__",
        "javascript" | "typescript" | "tsx" | "jsx" | "js" | "ts" => name == "constructor",
        "rust" | "rs" => name == "new",
        "go" => name.starts_with("New"),
        "ruby" | "rb" => name == "initialize",
        "php" => name == "__construct",
        "swift" => name == "init",
        "scala" => name == "<init>" || name == "this",
        "java" | "csharp" | "cs" | "kotlin" | "kt" => false,
        "c" | "cpp" | "c++" => false,
        "elixir" | "ex" | "lua" => false,
        _ => {
            name == "__init__"
                || name == "constructor"
                || name == "new"
                || name == "initialize"
                || name == "__construct"
                || name == "init"
        }
    }
}

/// Returns true if the receiver name is a self-reference for the given language.
///
/// - Python/Rust/Ruby/Swift: `self`
/// - TypeScript/JavaScript/Java/C#/Kotlin/Scala/C++/PHP: `this`
/// - Go: receiver is a named variable (neither `self` nor `this`)
/// - C/Elixir/Lua: no self-reference concept
/// - Unknown: either `self` or `this`
fn is_self_reference(receiver: &str, language: &str) -> bool {
    match language {
        "python" | "py" | "rust" | "rs" | "ruby" | "rb" | "swift" => receiver == "self",
        "typescript" | "javascript" | "tsx" | "jsx" | "ts" | "js" | "java" | "csharp" | "cs"
        | "kotlin" | "kt" | "scala" | "cpp" | "c++" | "php" => receiver == "this",
        "go" | "c" | "elixir" | "ex" | "lua" => false,
        _ => receiver == "self" || receiver == "this",
    }
}

// =============================================================================
// New Tier-1 AST-based Smell Detectors
// =============================================================================

/// Resolve a language string to a `Language` enum value.
fn resolve_language(lang_str: &str) -> Option<Language> {
    match lang_str.to_lowercase().as_str() {
        "python" | "py" => Some(Language::Python),
        "rust" | "rs" => Some(Language::Rust),
        "typescript" | "ts" => Some(Language::TypeScript),
        "javascript" | "js" => Some(Language::JavaScript),
        "go" => Some(Language::Go),
        "java" => Some(Language::Java),
        "c" => Some(Language::C),
        "cpp" | "c++" => Some(Language::Cpp),
        "ruby" | "rb" => Some(Language::Ruby),
        "csharp" | "c#" | "cs" => Some(Language::CSharp),
        "scala" => Some(Language::Scala),
        "php" => Some(Language::Php),
        "lua" => Some(Language::Lua),
        "kotlin" | "kt" => Some(Language::Kotlin),
        "elixir" | "ex" => Some(Language::Elixir),
        _ => None,
    }
}

/// Parse source code into a tree-sitter tree, returning None on failure.
fn parse_source(source: &str, lang_str: &str) -> Option<(tree_sitter::Tree, Language)> {
    let lang = resolve_language(lang_str)?;
    let pool = ParserPool::new();
    pool.parse(source, lang).ok().map(|tree| (tree, lang))
}

/// Check if a tree-sitter node kind represents a control flow construct that increases nesting.
fn is_nesting_node(kind: &str) -> bool {
    matches!(kind,
        // Common across languages
        "if_statement" | "if_expression" |
        "for_statement" | "for_expression" |
        "while_statement" | "while_expression" |
        "try_statement" | "try_expression" |
        "with_statement" |
        "match_statement" | "match_expression" |
        // Rust-specific
        "if_let_expression" |
        "loop_expression" |
        // Go
        "for_clause" |
        // Java / C# / TypeScript
        "for_in_statement" |
        "switch_statement" | "switch_expression" |
        "do_statement" |
        "catch_clause" |
        // Generic
        "try_catch_statement" |
        "except_clause"
    )
}

/// Detect deep nesting in source code using AST analysis.
///
/// Walks the tree-sitter AST, tracking nesting depth of control flow nodes.
/// Reports any function where max nesting depth >= 5.
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Language name (e.g., "python", "rust")
///
/// # Returns
/// A vector of `SmellFinding` for each function with deep nesting.
pub fn detect_deep_nesting(source: &str, language: &str) -> Vec<SmellFinding> {
    let (tree, _lang) = match parse_source(source, language) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    let mut findings = Vec::new();

    // Find all function nodes, then measure max nesting depth within each
    find_functions_and_measure_nesting(root, source, &mut findings);

    findings
}

/// Recursively find function-like nodes and measure their nesting depth.
fn find_functions_and_measure_nesting(
    node: tree_sitter::Node,
    source: &str,
    findings: &mut Vec<SmellFinding>,
) {
    let kind = node.kind();
    let is_function = matches!(kind,
        "function_definition" | "function_declaration" | "function_item" |
        "method_definition" | "method_declaration" |
        "arrow_function" | "function" | "closure_expression" |
        "function_expression" | "generator_function" |
        "async_function" | "function_def"
    );

    if is_function {
        // Get function name
        let func_name = extract_function_name(node, source).unwrap_or_else(|| "<anonymous>".to_string());
        let line = node.start_position().row as u32 + 1;

        // Measure max nesting depth within this function's body
        let max_depth = measure_max_nesting_depth(node, 0);

        if max_depth >= 5 {
            findings.push(SmellFinding {
                smell_type: SmellType::DeepNesting,
                file: PathBuf::from("<source>"),
                name: func_name,
                line,
                reason: format!(
                    "Function has nesting depth {} (threshold: 5)",
                    max_depth
                ),
                severity: nesting_severity(max_depth),
                suggestion: None,
            });
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Don't recurse into nested functions - they get their own analysis
        if !is_function || !matches!(child.kind(),
            "function_definition" | "function_declaration" | "function_item" |
            "method_definition" | "method_declaration"
        ) {
            find_functions_and_measure_nesting(child, source, findings);
        }
    }
}

/// Measure the maximum nesting depth of control flow nodes within a subtree.
fn measure_max_nesting_depth(node: tree_sitter::Node, current_depth: usize) -> usize {
    let kind = node.kind();
    let new_depth = if is_nesting_node(kind) {
        current_depth + 1
    } else {
        current_depth
    };

    let mut max_depth = new_depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_max = measure_max_nesting_depth(child, new_depth);
        if child_max > max_depth {
            max_depth = child_max;
        }
    }

    max_depth
}

/// Extract function name from a tree-sitter function node.
fn extract_function_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    // Try "name" field first (most languages)
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = &source[name_node.byte_range()];
        return Some(name.to_string());
    }
    None
}

/// Detect data classes: classes with many fields but few/no methods.
///
/// A class is considered a "data class" if it has >= 4 fields and <= 2 methods
/// (or the methods/fields ratio is < 0.5).
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Language name
///
/// # Returns
/// A vector of `SmellFinding` for each data class detected.
pub fn detect_data_classes(source: &str, language: &str) -> Vec<SmellFinding> {
    let (tree, _lang) = match parse_source(source, language) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    let mut findings = Vec::new();

    find_classes_and_check_data_class(root, source, &mut findings);

    findings
}

/// Recursively find class nodes and check if they are data classes.
fn find_classes_and_check_data_class(
    node: tree_sitter::Node,
    source: &str,
    findings: &mut Vec<SmellFinding>,
) {
    let kind = node.kind();
    let is_class = matches!(kind,
        "class_definition" | "class_declaration" |
        "struct_item" | "struct_declaration" |
        "interface_declaration"
    );

    if is_class {
        let class_name = extract_class_name(node, source)
            .unwrap_or_else(|| "<unknown>".to_string());
        let line = node.start_position().row as u32 + 1;

        let (field_count, method_count) = count_class_members(node, source);

        // Data class: many fields, few methods
        if field_count >= 4 && method_count <= 2 {
            let ratio = if field_count > 0 {
                method_count as f64 / field_count as f64
            } else {
                0.0
            };

            if ratio < 0.5 {
                findings.push(SmellFinding {
                    smell_type: SmellType::DataClass,
                    file: PathBuf::from("<source>"),
                    name: class_name,
                    line,
                    reason: format!(
                        "Class has {} fields and {} methods (data bag, ratio {:.2})",
                        field_count, method_count, ratio
                    ),
                    severity: data_class_severity(field_count, method_count),
                    suggestion: None,
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_classes_and_check_data_class(child, source, findings);
    }
}

/// Extract class name from a tree-sitter node.
fn extract_class_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = &source[name_node.byte_range()];
        return Some(name.to_string());
    }
    None
}

/// Count fields and methods in a class node.
///
/// Fields are identified by assignment patterns in __init__ (Python),
/// field_declaration nodes (Java/TS/Rust), etc.
/// Methods are function/method definitions inside the class.
fn count_class_members(node: tree_sitter::Node, source: &str) -> (usize, usize) {
    let mut field_count = 0;
    let mut method_count = 0;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            // Direct method definitions
            "function_definition" | "function_declaration" |
            "method_definition" | "method_declaration" |
            "function_item" => {
                method_count += 1;
                // For Python __init__, count self.x = ... assignments as fields
                let func_name = extract_function_name(child, source);
                if func_name.as_deref() == Some("__init__") {
                    field_count += count_self_assignments(child, source);
                }
            }
            // Field declarations (Java, TypeScript, Rust struct fields)
            "field_declaration" | "field_definition" |
            "property_declaration" | "public_field_definition" |
            "class_variable" => {
                field_count += 1;
            }
            // Rust struct body members
            "field_declaration_list" => {
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "field_declaration" {
                        field_count += 1;
                    }
                }
            }
            // Class body (Python, etc.) - recurse
            "class_body" | "block" | "declaration_list" | "class_heritage" => {
                let (f, m) = count_class_members(child, source);
                field_count += f;
                method_count += m;
            }
            _ => {}
        }
    }

    (field_count, method_count)
}

/// Count `self.x = ...` assignments in a Python __init__ method as field indicators.
fn count_self_assignments(node: tree_sitter::Node, source: &str) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "expression_statement" || child.kind() == "assignment" {
            let text = &source[child.byte_range()];
            if text.starts_with("self.") || text.contains("self.") {
                // Count distinct self.attr = patterns
                count += text.matches("self.").count().min(1);
            }
        }
        // Recurse into blocks / function body
        count += count_self_assignments(child, source);
    }
    count
}

/// Detect lazy elements: classes with only 1 method and 0-1 fields.
///
/// A class this small likely doesn't justify its own abstraction.
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Language name
///
/// # Returns
/// A vector of `SmellFinding` for each lazy element detected.
pub fn detect_lazy_elements(source: &str, language: &str) -> Vec<SmellFinding> {
    let (tree, _lang) = match parse_source(source, language) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    let mut findings = Vec::new();

    find_classes_and_check_lazy(root, source, &mut findings);

    findings
}

/// Recursively find class nodes and check if they are lazy elements.
fn find_classes_and_check_lazy(
    node: tree_sitter::Node,
    source: &str,
    findings: &mut Vec<SmellFinding>,
) {
    let kind = node.kind();
    let is_class = matches!(kind,
        "class_definition" | "class_declaration" |
        "struct_item" | "struct_declaration"
    );

    if is_class {
        let class_name = extract_class_name(node, source)
            .unwrap_or_else(|| "<unknown>".to_string());
        let line = node.start_position().row as u32 + 1;

        let (field_count, method_count) = count_class_members(node, source);

        if method_count <= 1 && field_count <= 1 {
            findings.push(SmellFinding {
                smell_type: SmellType::LazyElement,
                file: PathBuf::from("<source>"),
                name: class_name,
                line,
                reason: format!(
                    "Class has only {} method(s) and {} field(s) - may not justify its own class",
                    method_count, field_count
                ),
                severity: 1, // Always low severity
                suggestion: None,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_classes_and_check_lazy(child, source, findings);
    }
}

/// Detect message chains: long chains of method/attribute access.
///
/// Looks for chains of `.member` access deeper than 3 levels,
/// indicating tight coupling to an object's internal structure.
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Language name
///
/// # Returns
/// A vector of `SmellFinding` for each message chain detected.
pub fn detect_message_chains(source: &str, language: &str) -> Vec<SmellFinding> {
    let (tree, _lang) = match parse_source(source, language) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    let mut findings = Vec::new();
    let mut visited_lines: std::collections::HashSet<u32> = std::collections::HashSet::new();

    find_message_chains(root, source, &mut findings, &mut visited_lines);

    findings
}

/// Recursively find message chains in the AST.
fn find_message_chains(
    node: tree_sitter::Node,
    source: &str,
    findings: &mut Vec<SmellFinding>,
    visited_lines: &mut std::collections::HashSet<u32>,
) {
    let kind = node.kind();

    // Look for attribute/member access patterns
    let is_chain_node = matches!(kind,
        "attribute" | "member_expression" | "field_expression" |
        "call_expression" | "method_invocation" | "call"
    );

    if is_chain_node {
        let chain_length = measure_chain_length(node);
        let line = node.start_position().row as u32 + 1;

        if chain_length > 3 && !visited_lines.contains(&line) {
            visited_lines.insert(line);
            let chain_text = &source[node.byte_range()];
            let truncated = if chain_text.len() > 60 {
                format!("{}...", &chain_text[..57])
            } else {
                chain_text.to_string()
            };

            findings.push(SmellFinding {
                smell_type: SmellType::MessageChain,
                file: PathBuf::from("<source>"),
                name: truncated,
                line,
                reason: format!(
                    "Method chain of length {} (threshold: 3)",
                    chain_length
                ),
                severity: chain_severity(chain_length),
                suggestion: None,
            });
            // Don't recurse into children of this chain - we already counted it
            return;
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_message_chains(child, source, findings, visited_lines);
    }
}

/// Measure the length of a method/attribute chain by walking down the AST.
fn measure_chain_length(node: tree_sitter::Node) -> usize {
    let kind = node.kind();
    let is_access = matches!(kind,
        "attribute" | "member_expression" | "field_expression" |
        "call_expression" | "method_invocation" | "call"
    );

    if !is_access {
        return 0;
    }

    // The "object" or "value" or "function" child is the part before the dot
    let child_chain = node.child_by_field_name("object")
        .or_else(|| node.child_by_field_name("value"))
        .or_else(|| node.child_by_field_name("function"))
        .map(|c| measure_chain_length(c))
        .unwrap_or(0);

    // For call_expression, look at arguments' parent
    if kind == "call_expression" || kind == "call" {
        // The function being called is the chain part
        if let Some(func) = node.child_by_field_name("function") {
            return measure_chain_length(func);
        }
        // Fallback: first child
        if let Some(first) = node.child(0) {
            return measure_chain_length(first);
        }
    }

    1 + child_chain
}

/// Set of primitive type names across languages.
const PRIMITIVE_TYPES: &[&str] = &[
    // Python
    "int", "float", "str", "bool", "bytes",
    // Rust
    "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize",
    "f32", "f64", "String", "&str", "char",
    // TypeScript/JavaScript
    "number", "string", "boolean",
    // Java / C#
    "byte", "short", "long", "double",
    "Integer", "Long", "Double", "Float", "Boolean",
    // Go
    "int8", "int16", "int32", "int64",
    "uint8", "uint16", "uint32", "uint64",
    "float32", "float64",
];

/// Check if a type string is a primitive type.
fn is_primitive_type(type_str: &str) -> bool {
    let trimmed = type_str.trim();
    // Handle reference types like &str, &mut str
    let base = trimmed.trim_start_matches('&').trim_start_matches("mut ");
    PRIMITIVE_TYPES.contains(&base)
}

/// Detect primitive obsession: functions with many primitive-typed parameters.
///
/// Counts parameters with primitive type annotations. If more than 3
/// primitives are found, it's flagged as a smell.
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Language name
///
/// # Returns
/// A vector of `SmellFinding` for each function with primitive obsession.
pub fn detect_primitive_obsession(source: &str, language: &str) -> Vec<SmellFinding> {
    let (tree, _lang) = match parse_source(source, language) {
        Some(v) => v,
        None => return Vec::new(),
    };

    let root = tree.root_node();
    let mut findings = Vec::new();

    find_functions_and_check_primitives(root, source, &mut findings);

    findings
}

/// Recursively find functions and check for primitive obsession.
fn find_functions_and_check_primitives(
    node: tree_sitter::Node,
    source: &str,
    findings: &mut Vec<SmellFinding>,
) {
    let kind = node.kind();
    let is_function = matches!(kind,
        "function_definition" | "function_declaration" | "function_item" |
        "method_definition" | "method_declaration" |
        "arrow_function" | "function"
    );

    if is_function {
        let func_name = extract_function_name(node, source)
            .unwrap_or_else(|| "<anonymous>".to_string());
        let line = node.start_position().row as u32 + 1;

        let primitive_count = count_primitive_params(node, source);

        if primitive_count > 3 {
            findings.push(SmellFinding {
                smell_type: SmellType::PrimitiveObsession,
                file: PathBuf::from("<source>"),
                name: func_name,
                line,
                reason: format!(
                    "Function has {} primitive parameters (threshold: 3)",
                    primitive_count
                ),
                severity: primitive_obsession_severity(primitive_count),
                suggestion: None,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_functions_and_check_primitives(child, source, findings);
    }
}

/// Count the number of primitive-typed parameters in a function node.
fn count_primitive_params(node: tree_sitter::Node, source: &str) -> usize {
    // Look for the parameters field
    let params_node = match node.child_by_field_name("parameters") {
        Some(p) => p,
        None => return 0,
    };

    let mut count = 0;
    let mut cursor = params_node.walk();
    for param in params_node.children(&mut cursor) {
        let param_kind = param.kind();
        // Skip self/this parameters and delimiters
        if param_kind == "self" || param_kind == "," || param_kind == "(" || param_kind == ")" {
            continue;
        }

        // Look for type annotation in the parameter
        if let Some(type_node) = param.child_by_field_name("type") {
            let type_text = &source[type_node.byte_range()];
            if is_primitive_type(type_text) {
                count += 1;
            }
        }
        // Python typed_parameter: look for "type" child
        else if param_kind == "typed_parameter" || param_kind == "typed_default_parameter" {
            // Try to find type annotation child
            let mut inner_cursor = param.walk();
            for child in param.children(&mut inner_cursor) {
                if child.kind() == "type" {
                    let type_text = &source[child.byte_range()];
                    if is_primitive_type(type_text) {
                        count += 1;
                    }
                }
            }
        }
    }

    count
}

// =============================================================================
// Tier-2 Fowler Smell Detectors
// =============================================================================

// --- Source-based stubs (backward compatibility for existing contract tests) ---
// Tier-2 smells require --deep mode with CallGraphIR/InheritanceReport.
// These stubs exist so that detect_middle_man(source, lang) still compiles,
// but they return empty since source-only detection is not supported.

/// Detect Middle Man smell from source string.
///
/// This is a backward-compatible stub. Tier-2 Middle Man detection requires
/// `--deep` mode. Use `detect_middle_man_from_callgraph()` for proper detection.
#[deprecated(since = "0.2.0", note = "Use detect_middle_man_from_callgraph() with --deep mode instead")]
pub fn detect_middle_man(_source: &str, _language: &str) -> Vec<SmellFinding> {
    // Tier-2 smells require --deep mode with CallGraphIR
    Vec::new()
}

/// Detect Refused Bequest smell from source string.
///
/// This is a backward-compatible stub. Tier-2 Refused Bequest detection requires
/// `--deep` mode. Use `detect_refused_bequest_from_callgraph()` for proper detection.
#[deprecated(since = "0.2.0", note = "Use detect_refused_bequest_from_callgraph() with --deep mode instead")]
pub fn detect_refused_bequest(_source: &str, _language: &str) -> Vec<SmellFinding> {
    // Tier-2 smells require --deep mode with CallGraphIR + InheritanceReport
    Vec::new()
}

/// Detect Feature Envy smell from source string.
///
/// This is a backward-compatible stub. Tier-2 Feature Envy detection requires
/// `--deep` mode. Use `detect_feature_envy_from_callgraph()` for proper detection.
#[deprecated(since = "0.2.0", note = "Use detect_feature_envy_from_callgraph() with --deep mode instead")]
pub fn detect_feature_envy(_source: &str, _language: &str) -> Vec<SmellFinding> {
    // Tier-2 smells require --deep mode with CallGraphIR
    Vec::new()
}

/// Detect Inappropriate Intimacy smell from source string.
///
/// This is a backward-compatible stub. Tier-2 Inappropriate Intimacy detection requires
/// `--deep` mode. Use `detect_inappropriate_intimacy_from_callgraph()` for proper detection.
#[deprecated(since = "0.2.0", note = "Use detect_inappropriate_intimacy_from_callgraph() with --deep mode instead")]
pub fn detect_inappropriate_intimacy(_source: &str, _language: &str) -> Vec<SmellFinding> {
    // Tier-2 smells require --deep mode with CallGraphIR + InheritanceReport
    Vec::new()
}

// --- CallGraph-based detection stubs (proper signatures for Phases 2-5) ---

/// Detect Middle Man smell from call graph data.
///
/// Identifies classes where more than `thresholds.middle_man_delegation_ratio` of
/// non-constructor methods are pure delegators to another object.
///
/// # Arguments
/// * `file_ir` - File IR containing classes, functions, and call sites
/// * `thresholds` - Threshold configuration
/// * `language` - Language name for constructor/self detection
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// A vector of `SmellFinding` for each Middle Man class detected.
pub fn detect_middle_man_from_callgraph(
    file_ir: &FileIR,
    thresholds: &Thresholds,
    language: &str,
    suggest: bool,
) -> Vec<SmellFinding> {
    /// Design-pattern class names to exclude (case-insensitive).
    /// These classes legitimately delegate as part of their pattern.
    const EXCLUDED_PATTERNS: &[&str] = &[
        "facade", "adapter", "wrapper", "proxy", "bridge", "decorator", "gateway",
    ];

    let mut findings = Vec::new();

    for class in &file_ir.classes {
        let methods = get_class_methods_robust(file_ir, &class.name);

        // Filter out constructors
        let non_constructor_methods: Vec<&FuncDef> = methods
            .iter()
            .filter(|m| !is_constructor(&m.name, language))
            .copied()
            .collect();

        let total = non_constructor_methods.len();
        if total < thresholds.middle_man_min_methods {
            continue;
        }

        // Facade/Adapter exclusion heuristic -- check before costly analysis
        let name_lower = class.name.to_lowercase();
        if EXCLUDED_PATTERNS.iter().any(|p| name_lower.contains(p)) {
            continue;
        }

        // Check each method for pure delegation
        let mut delegation_count: usize = 0;
        let mut delegate_targets: HashMap<String, usize> = HashMap::new();

        for method in &non_constructor_methods {
            let qualified = format!("{}.{}", class.name, method.name);
            let calls = file_ir
                .calls
                .get(&qualified)
                .or_else(|| file_ir.calls.get(&method.name));

            if let Some(calls) = calls {
                // Collect method/attr calls (the ones that represent delegation)
                let method_calls: Vec<&CallSite> = calls
                    .iter()
                    .filter(|c| matches!(c.call_type, CallType::Method | CallType::Attr))
                    .collect();

                // Pure delegation: exactly 1 method call to a non-self receiver
                if method_calls.len() == 1 {
                    let call = method_calls[0];
                    let receiver_is_self = call
                        .receiver
                        .as_ref()
                        .map(|r| {
                            is_self_reference(r, language)
                                || r.starts_with("self.")
                                || r.starts_with("this.")
                        })
                        .unwrap_or(false);

                    // Count non-method/attr calls (e.g., Direct, Intra).
                    // A pure delegator should have no additional logic calls.
                    let non_method_calls = calls
                        .iter()
                        .filter(|c| !matches!(c.call_type, CallType::Method | CallType::Attr))
                        .count();

                    if !receiver_is_self && non_method_calls == 0 {
                        delegation_count += 1;
                        if let Some(ref rt) = call.receiver_type {
                            *delegate_targets.entry(rt.clone()).or_insert(0) += 1;
                        }
                    }
                }
                // method_calls.len() != 1 means either 0 method calls (no delegation)
                // or multiple method calls (not a pure delegator)
            }
            // No calls entry means the method has real logic or is empty -- not a delegator
        }

        let ratio = delegation_count as f64 / total as f64;
        if ratio >= thresholds.middle_man_delegation_ratio {
            let primary_delegate = delegate_targets
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "unknown".to_string());

            findings.push(SmellFinding {
                smell_type: SmellType::MiddleMan,
                file: file_ir.path.clone(),
                name: class.name.clone(),
                line: class.line,
                reason: format!(
                    "Class delegates {}/{} methods ({:.0}%) to {}",
                    delegation_count,
                    total,
                    ratio * 100.0,
                    primary_delegate
                ),
                severity: middle_man_severity(ratio, delegation_count),
                suggestion: if suggest {
                    Some(format!(
                        "Consider removing {} and using {} directly",
                        class.name, primary_delegate
                    ))
                } else {
                    None
                },
            });
        }
    }

    findings
}

/// Detect Refused Bequest smell from call graph and inheritance data.
///
/// Identifies subclasses that use fewer than `thresholds.refused_bequest_usage_ratio`
/// of their parent's concrete, non-abstract methods.
///
/// # Arguments
/// * `call_graph` - Full call graph with cross-file data
/// * `inheritance_report` - Inheritance analysis report
/// * `thresholds` - Threshold configuration
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// A vector of `SmellFinding` for each Refused Bequest detected.
pub fn detect_refused_bequest_from_callgraph(
    call_graph: &CallGraphIR,
    inheritance_report: &InheritanceReport,
    thresholds: &Thresholds,
    suggest: bool,
) -> Vec<SmellFinding> {
    use crate::types::inheritance::{InheritanceKind, BaseResolution};

    let mut findings = Vec::new();

    for edge in &inheritance_report.edges {
        // Skip external/stdlib/unresolved parents
        if edge.external || edge.resolution == BaseResolution::Unresolved {
            continue;
        }

        // Skip Go embedding (XL2) -- not classical inheritance
        if edge.kind == InheritanceKind::Embeds {
            continue;
        }

        // Skip interface implementations -- compliance, not bequest
        if edge.kind == InheritanceKind::Implements {
            continue;
        }

        // Check parent node flags (C5): skip abstract, protocol, mixin parents
        let parent_node = inheritance_report
            .nodes
            .iter()
            .find(|n| n.name == edge.parent);
        if let Some(parent) = parent_node {
            if parent.is_abstract == Some(true)
                || parent.protocol == Some(true)
                || parent.mixin == Some(true)
            {
                continue;
            }
        }

        // Get parent class methods from any file in the call graph
        let parent_concrete_methods = get_parent_concrete_methods(call_graph, &edge.parent);

        // Apply minimum inherited methods threshold
        if parent_concrete_methods.len() < thresholds.refused_bequest_min_inherited {
            continue;
        }

        // Get child class methods (compute once, use for both override check and call analysis)
        let child_file_ir = call_graph.files.get(&edge.child_file);
        let child_methods = child_file_ir
            .map(|fir| get_class_methods_robust(fir, &edge.child))
            .unwrap_or_default();

        let child_method_names: HashSet<&str> = child_methods
            .iter()
            .map(|f| f.name.as_str())
            .collect();

        // Get all call targets made from any child class method
        let child_call_targets: HashSet<String> = if let Some(fir) = child_file_ir {
            child_methods
                .iter()
                .flat_map(|method| {
                    let qualified = format!("{}.{}", edge.child, method.name);
                    fir.calls
                        .get(&qualified)
                        .into_iter()
                        .chain(fir.calls.get(&method.name).into_iter())
                        .flatten()
                        .map(|c| c.target.clone())
                })
                .collect()
        } else {
            HashSet::new()
        };

        // Count usage: override OR call counts as "used" (C2)
        let mut used_count = 0usize;
        let mut unused_methods = Vec::new();

        for inherited_method in &parent_concrete_methods {
            let is_overridden = child_method_names.contains(inherited_method.as_str());
            let is_called = child_call_targets.contains(inherited_method);

            if is_overridden || is_called {
                used_count += 1;
            } else {
                unused_methods.push(inherited_method.clone());
            }
        }

        let total = parent_concrete_methods.len();
        let usage_ratio = used_count as f64 / total as f64;

        if usage_ratio < thresholds.refused_bequest_usage_ratio {
            let child_line = edge.child_line;

            findings.push(SmellFinding {
                smell_type: SmellType::RefusedBequest,
                file: edge.child_file.clone(),
                name: edge.child.clone(),
                line: child_line,
                reason: format!(
                    "Uses {}/{} ({:.0}%) inherited methods from {}. Unused: {}",
                    used_count,
                    total,
                    usage_ratio * 100.0,
                    edge.parent,
                    if unused_methods.len() <= 5 {
                        unused_methods.join(", ")
                    } else {
                        format!(
                            "{}, ... and {} more",
                            unused_methods[..5].join(", "),
                            unused_methods.len() - 5
                        )
                    }
                ),
                severity: refused_bequest_severity(usage_ratio, total),
                suggestion: if suggest {
                    Some(format!(
                        "Consider composition over inheritance, or remove {} as a base of {}",
                        edge.parent, edge.child
                    ))
                } else {
                    None
                },
            });
        }
    }

    findings
}

/// Get concrete (non-constructor) method names from a parent class across all files in call graph.
///
/// Searches all files in the call graph for the parent class and returns method names
/// excluding constructors (detected via language-agnostic heuristic).
fn get_parent_concrete_methods(call_graph: &CallGraphIR, parent_name: &str) -> Vec<String> {
    let language = &call_graph.language;
    for file_ir in call_graph.files.values() {
        let methods = get_class_methods_robust(file_ir, parent_name);
        if !methods.is_empty() {
            return methods
                .iter()
                .filter(|m| !is_constructor(&m.name, language))
                .map(|m| m.name.clone())
                .collect();
        }
    }
    Vec::new()
}

/// Detect Feature Envy smell from call graph data.
///
/// Identifies methods that access more features (method calls, attribute accesses)
/// of another class than their own class. Uses a dual-threshold approach:
/// - `feature_envy_min_foreign`: minimum number of foreign accesses to consider
/// - `feature_envy_ratio`: minimum ratio of foreign-to-own accesses
///
/// # Role-Based Exclusion (C4)
///
/// Classes whose names (case-insensitive) contain any of the following role
/// keywords are excluded: format, formatter, serialize, serializer, deserialize,
/// handler, visitor, render, renderer, builder, validator, converter, mapper,
/// adapter, factory, transformer, presenter.
///
/// # Edge Cases
///
/// - Static methods (`is_method == false`): skipped
/// - Constructors: skipped
/// - Methods with no calls: skipped
/// - Division by zero when `own_count == 0`: uses `own.max(1)` for ratio
///
/// # Arguments
/// * `file_ir` - File IR containing classes, functions, and call sites
/// * `thresholds` - Threshold configuration
/// * `language` - Language name for self-reference detection
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// A vector of `SmellFinding` for each Feature Envy method detected.
pub fn detect_feature_envy_from_callgraph(
    file_ir: &FileIR,
    thresholds: &Thresholds,
    language: &str,
    suggest: bool,
) -> Vec<SmellFinding> {
    /// Role-based class name patterns to exclude (case-insensitive).
    /// These classes legitimately access foreign data as part of their design role.
    const EXCLUDED_ROLES: &[&str] = &[
        "format", "formatter", "serialize", "serializer", "deserialize",
        "handler", "visitor", "render", "renderer", "builder",
        "validator", "converter", "mapper", "adapter",
        "factory", "transformer", "presenter",
    ];

    let mut findings = Vec::new();

    for class in &file_ir.classes {
        // Role-based exclusion (C4)
        let name_lower = class.name.to_lowercase();
        if EXCLUDED_ROLES.iter().any(|r| name_lower.contains(r)) {
            continue;
        }

        let methods = get_class_methods_robust(file_ir, &class.name);

        for method in &methods {
            // Skip constructors
            if is_constructor(&method.name, language) {
                continue;
            }

            // Skip static methods (no self/this parameter)
            if !method.is_method {
                continue;
            }

            let qualified = format!("{}.{}", class.name, method.name);
            let calls = file_ir
                .calls
                .get(&qualified)
                .or_else(|| file_ir.calls.get(&method.name));

            let calls = match calls {
                Some(c) if !c.is_empty() => c,
                _ => continue, // No calls = no envy possible
            };

            // Count accesses by target class
            let mut own_count: usize = 0;
            let mut foreign_counts: HashMap<String, usize> = HashMap::new();

            for call in calls {
                if !matches!(call.call_type, CallType::Method | CallType::Attr) {
                    continue;
                }

                // Determine if this is an own-class or foreign-class call
                let is_own = call
                    .receiver
                    .as_ref()
                    .map(|r| {
                        is_self_reference(r, language)
                            || r.starts_with("self.")
                            || r.starts_with("this.")
                    })
                    .unwrap_or(false);

                if is_own {
                    own_count += 1;
                } else if let Some(ref rt) = call.receiver_type {
                    if rt == &class.name {
                        own_count += 1; // Type resolved to own class
                    } else {
                        *foreign_counts.entry(rt.clone()).or_insert(0) += 1;
                    }
                }
                // Calls with no receiver_type and not self/this are unclassified -- skip
            }

            // Check each foreign class against thresholds
            for (foreign_class, foreign_count) in &foreign_counts {
                if *foreign_count < thresholds.feature_envy_min_foreign {
                    continue;
                }

                let ratio = *foreign_count as f64 / own_count.max(1) as f64;
                if ratio < thresholds.feature_envy_ratio {
                    continue;
                }

                findings.push(SmellFinding {
                    smell_type: SmellType::FeatureEnvy,
                    file: file_ir.path.clone(),
                    name: format!("{}::{}", class.name, method.name),
                    line: method.line,
                    reason: format!(
                        "Accesses {} features of {} but only {} of own class {} (ratio {:.1}:1)",
                        foreign_count, foreign_class, own_count, class.name, ratio
                    ),
                    severity: feature_envy_severity(*foreign_count, own_count),
                    suggestion: if suggest {
                        Some(format!(
                            "Consider moving {} to {} or extracting shared logic",
                            method.name, foreign_class
                        ))
                    } else {
                        None
                    },
                });
            }
        }
    }

    findings
}

/// Metrics tracking bidirectional access between a normalized class pair.
///
/// A pair (A, B) is normalized so that A <= B lexicographically.
/// `a_to_b` counts accesses from class A's methods to class B,
/// `b_to_a` counts accesses from class B's methods to class A.
struct IntimacyPairMetrics {
    /// Number of accesses from class A (lexicographically first) to class B.
    a_to_b: usize,
    /// Number of accesses from class B to class A.
    b_to_a: usize,
    /// Number of private (underscore-prefixed) accesses from A to B.
    a_to_b_private: usize,
    /// Number of private (underscore-prefixed) accesses from B to A.
    b_to_a_private: usize,
}

impl IntimacyPairMetrics {
    /// Create a new metrics tracker with all counts at zero.
    fn new() -> Self {
        Self {
            a_to_b: 0,
            b_to_a: 0,
            a_to_b_private: 0,
            b_to_a_private: 0,
        }
    }

    /// Total bidirectional access count.
    fn total(&self) -> usize {
        self.a_to_b + self.b_to_a
    }

    /// Minimum access count across both directions.
    fn min_direction(&self) -> usize {
        self.a_to_b.min(self.b_to_a)
    }

    /// Check if both directions meet the minimum per-direction threshold.
    fn is_bidirectional_enough(&self, min_per_dir: usize) -> bool {
        self.a_to_b >= min_per_dir && self.b_to_a >= min_per_dir
    }
}

/// Normalize a class pair to (min, max) lexicographic order for consistent deduplication.
fn normalize_class_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Detect Inappropriate Intimacy smell from call graph and inheritance data.
///
/// Identifies pairs of classes with bidirectional internal access exceeding thresholds.
/// This detector operates on the full `CallGraphIR`, enabling cross-file analysis.
///
/// Detection logic:
/// 1. Build a cross-class access graph from all files in the call graph
/// 2. For each class pair, count accesses in both directions
/// 3. Apply thresholds: total accesses >= `intimacy_min_total` AND
///    min(a_to_b, b_to_a) >= `intimacy_min_per_direction`
/// 4. Exclude inheritance-related pairs (parent-child access is expected)
///
/// # Arguments
/// * `call_graph` - Full call graph with cross-file data
/// * `inheritance_report` - Inheritance analysis report for exclusion checking
/// * `thresholds` - Threshold configuration
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// A vector of `SmellFinding` for each intimate class pair detected.
pub fn detect_inappropriate_intimacy_from_callgraph(
    call_graph: &CallGraphIR,
    inheritance_report: &InheritanceReport,
    thresholds: &Thresholds,
    suggest: bool,
) -> Vec<SmellFinding> {
    let mut findings = Vec::new();
    let mut pair_metrics: HashMap<(String, String), IntimacyPairMetrics> = HashMap::new();

    // Build a set of inheritance-related pairs to exclude (parent-child access is expected)
    let inheritance_pairs: HashSet<(String, String)> = inheritance_report
        .edges
        .iter()
        .map(|e| normalize_class_pair(&e.child, &e.parent))
        .collect();

    // Walk all files in the call graph to build the cross-class access graph
    for file_ir in call_graph.files.values() {
        for class in &file_ir.classes {
            let methods = get_class_methods_robust(file_ir, &class.name);

            for method in &methods {
                let qualified = format!("{}.{}", class.name, method.name);
                let calls = file_ir
                    .calls
                    .get(&qualified)
                    .or_else(|| file_ir.calls.get(&method.name));

                if let Some(calls) = calls {
                    for call in calls {
                        // Only consider method calls and attribute accesses
                        if !matches!(call.call_type, CallType::Method | CallType::Attr) {
                            continue;
                        }

                        if let Some(ref rt) = call.receiver_type {
                            // Skip self-class accesses
                            if rt == &class.name {
                                continue;
                            }

                            let pair = normalize_class_pair(&class.name, rt);

                            // Skip inheritance-related pairs
                            if inheritance_pairs.contains(&pair) {
                                continue;
                            }

                            let metrics = pair_metrics
                                .entry(pair.clone())
                                .or_insert_with(IntimacyPairMetrics::new);

                            // Determine direction based on normalized pair ordering
                            if class.name <= *rt {
                                // class is "A" in the normalized pair (A, B)
                                metrics.a_to_b += 1;
                                if call.target.starts_with('_') {
                                    metrics.a_to_b_private += 1;
                                }
                            } else {
                                // class is "B" in the normalized pair (A, B)
                                metrics.b_to_a += 1;
                                if call.target.starts_with('_') {
                                    metrics.b_to_a_private += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Evaluate each class pair against thresholds
    for ((class_a, class_b), metrics) in &pair_metrics {
        if metrics.total() < thresholds.intimacy_min_total {
            continue;
        }
        if !metrics.is_bidirectional_enough(thresholds.intimacy_min_per_direction) {
            continue;
        }

        findings.push(SmellFinding {
            smell_type: SmellType::InappropriateIntimacy,
            file: PathBuf::from("(cross-class)"),
            name: format!("{} <-> {}", class_a, class_b),
            line: 0,
            reason: format!(
                "Bidirectional coupling: {} -> {} ({} calls, {} private), {} -> {} ({} calls, {} private)",
                class_a, class_b, metrics.a_to_b, metrics.a_to_b_private,
                class_b, class_a, metrics.b_to_a, metrics.b_to_a_private
            ),
            severity: intimacy_severity(metrics.total(), metrics.min_direction()),
            suggestion: if suggest {
                Some(format!(
                    "Consider merging {} and {} or extracting shared behavior into a third class",
                    class_a, class_b
                ))
            } else {
                None
            },
        });
    }

    findings
}

// =============================================================================
// Aggregated Smell Detection (--deep mode)
// =============================================================================

/// Detect code smells using aggregated analysis from multiple existing analyzers.
///
/// This runs the original 3 detectors (God Class, Long Method, Long Parameter List)
/// plus pulls findings from:
/// - Cohesion analyzer (LCOM4 >= 2 => LowCohesion smell)
/// - Coupling analyzer (score >= 0.6 => TightCoupling smell)
/// - Dead code analyzer (unreachable => DeadCode smell)
/// - Similarity analyzer (score > 0.6 => CodeClone smell)
/// - Complexity analyzer (cognitive >= 15 => HighCognitiveComplexity smell)
///
/// Each sub-analyzer is called independently; if one fails, it is skipped
/// and the rest continue.
///
/// # Arguments
/// * `path` - File or directory to scan
/// * `threshold` - Threshold preset (Strict, Default, Relaxed)
/// * `smell_type` - Optional filter for specific smell type
/// * `suggest` - Whether to include fix suggestions
///
/// # Returns
/// * `Ok(SmellsReport)` - Combined report with findings from all analyzers
pub fn analyze_smells_aggregated(
    path: &Path,
    threshold: ThresholdPreset,
    smell_type: Option<SmellType>,
    suggest: bool,
) -> TldrResult<SmellsReport> {
    let mut all_smells: Vec<SmellFinding> = Vec::new();
    let mut files_scanned: usize = 0;

    if should_run_original_detectors(smell_type) {
        if let Ok(base_report) = detect_smells(path, threshold, smell_type, suggest) {
            files_scanned = base_report.files_scanned;
            all_smells.extend(base_report.smells);
        }
    }

    if should_analyze_smell(smell_type, SmellType::LowCohesion) {
        collect_low_cohesion_smells(path, suggest, &mut all_smells);
    }

    let needs_coupling = should_analyze_smell(smell_type, SmellType::TightCoupling);
    let needs_tier2 = needs_tier2_analysis(smell_type);
    let needs_call_graph = needs_coupling || needs_tier2;

    let (root_dir, cg_language) = call_graph_context(path, needs_call_graph);
    let shared_call_graph_ir = build_shared_call_graph_ir(root_dir, &cg_language, needs_call_graph);

    let project_call_graph = if needs_coupling {
        shared_call_graph_ir
            .as_ref()
            .map(crate::callgraph::builder::project_graph_from_ir_ref)
    } else {
        None
    };

    if needs_coupling {
        collect_tight_coupling_smells(
            path,
            &cg_language,
            project_call_graph.as_ref(),
            suggest,
            &mut all_smells,
        );
    }

    if should_analyze_smell(smell_type, SmellType::DeadCode) {
        collect_dead_code_smells(path, suggest, &mut all_smells);
    }

    if should_analyze_smell(smell_type, SmellType::CodeClone) {
        collect_code_clone_smells(path, suggest, &mut all_smells);
    }

    if should_analyze_smell(smell_type, SmellType::HighCognitiveComplexity) {
        collect_high_cognitive_smells(path, suggest, &mut all_smells);
    }

    let inheritance_report = build_inheritance_report(path, needs_tier2);

    let thresholds = Thresholds::from_preset(threshold);

    if should_analyze_smell(smell_type, SmellType::MiddleMan) {
        collect_middle_man_smells(
            shared_call_graph_ir.as_ref(),
            &thresholds,
            suggest,
            &mut all_smells,
        );
    }

    if should_analyze_smell(smell_type, SmellType::RefusedBequest) {
        collect_refused_bequest_smells(
            shared_call_graph_ir.as_ref(),
            inheritance_report.as_ref(),
            &thresholds,
            suggest,
            &mut all_smells,
        );
    }

    if should_analyze_smell(smell_type, SmellType::FeatureEnvy) {
        collect_feature_envy_smells(
            shared_call_graph_ir.as_ref(),
            &thresholds,
            suggest,
            &mut all_smells,
        );
    }

    if should_analyze_smell(smell_type, SmellType::InappropriateIntimacy) {
        collect_inappropriate_intimacy_smells(
            shared_call_graph_ir.as_ref(),
            inheritance_report.as_ref(),
            &thresholds,
            suggest,
            &mut all_smells,
        );
    }

    sort_smells(&mut all_smells);
    let by_file = build_smells_by_file(&all_smells);

    if files_scanned == 0 && !by_file.is_empty() {
        files_scanned = by_file.len();
    }

    let summary = build_smells_summary(&all_smells, files_scanned);

    Ok(SmellsReport {
        smells: all_smells,
        files_scanned,
        by_file,
        summary,
    })
}

fn should_run_original_detectors(smell_type: Option<SmellType>) -> bool {
    matches!(
        smell_type,
        None
            | Some(SmellType::GodClass)
            | Some(SmellType::LongMethod)
            | Some(SmellType::LongParameterList)
            | Some(SmellType::FeatureEnvy)
            | Some(SmellType::DataClumps)
            | Some(SmellType::DeepNesting)
            | Some(SmellType::DataClass)
            | Some(SmellType::LazyElement)
            | Some(SmellType::MessageChain)
            | Some(SmellType::PrimitiveObsession)
    )
}

fn should_analyze_smell(smell_type: Option<SmellType>, target: SmellType) -> bool {
    smell_type.is_none() || smell_type == Some(target)
}

fn needs_tier2_analysis(smell_type: Option<SmellType>) -> bool {
    smell_type.is_none()
        || matches!(
            smell_type,
            Some(SmellType::MiddleMan)
                | Some(SmellType::RefusedBequest)
                | Some(SmellType::FeatureEnvy)
                | Some(SmellType::InappropriateIntimacy)
        )
}

fn call_graph_context(path: &Path, needs_call_graph: bool) -> (&Path, String) {
    if !needs_call_graph {
        return (path, String::new());
    }
    if path.is_file() {
        let lang = Language::from_path(path)
            .map(|l| l.to_string().to_lowercase())
            .unwrap_or_else(|| "python".to_string());
        return (path.parent().unwrap_or(path), lang);
    }
    let lang = WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .find_map(|e| Language::from_path(e.path()))
        .map(|l| l.to_string().to_lowercase())
        .unwrap_or_else(|| "python".to_string());
    (path, lang)
}

fn build_shared_call_graph_ir(
    root_dir: &Path,
    cg_language: &str,
    needs_call_graph: bool,
) -> Option<CallGraphIR> {
    if !needs_call_graph {
        return None;
    }
    use crate::callgraph::builder_v2::{build_project_call_graph_v2, BuildConfig};
    let config = BuildConfig {
        language: cg_language.to_string(),
        ..Default::default()
    };
    build_project_call_graph_v2(root_dir, config).ok()
}

fn collect_low_cohesion_smells(path: &Path, suggest: bool, all_smells: &mut Vec<SmellFinding>) {
    if let Ok(cohesion_report) = crate::quality::cohesion::analyze_cohesion(path, None, 2) {
        for class in &cohesion_report.classes {
            if class.lcom4 < 2 {
                continue;
            }
            all_smells.push(SmellFinding {
                smell_type: SmellType::LowCohesion,
                file: class.file.clone(),
                name: class.name.clone(),
                line: class.line as u32,
                reason: format!(
                    "Class has LCOM4={} (>1 indicates multiple responsibilities)",
                    class.lcom4
                ),
                severity: cohesion_severity(class.lcom4),
                suggestion: if suggest {
                    class
                        .split_suggestion
                        .clone()
                        .or_else(|| Some("Consider splitting this class by responsibility".to_string()))
                } else {
                    None
                },
            });
        }
    }
}

fn collect_tight_coupling_smells(
    path: &Path,
    cg_language: &str,
    project_call_graph: Option<&crate::types::ProjectCallGraph>,
    suggest: bool,
    all_smells: &mut Vec<SmellFinding>,
) {
    let Some(project_call_graph) = project_call_graph else {
        return;
    };
    let lang = cg_language.parse::<Language>().unwrap_or(Language::Python);
    let options = crate::quality::coupling::CouplingOptions {
        max_pairs: 50,
        ..Default::default()
    };
    if let Ok(coupling_report) =
        crate::quality::coupling::analyze_coupling_with_graph(path, lang, project_call_graph, &options)
    {
        for pair in &coupling_report.top_pairs {
            if pair.score < 0.6 {
                continue;
            }
            let source_name = pair
                .source
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| pair.source.display().to_string());
            let target_name = pair
                .target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| pair.target.display().to_string());
            all_smells.push(SmellFinding {
                smell_type: SmellType::TightCoupling,
                file: pair.source.clone(),
                name: format!("{} <-> {}", source_name, target_name),
                line: 0,
                reason: format!(
                    "Coupling score {:.2} ({} calls, {} shared imports)",
                    pair.score,
                    pair.call_count,
                    pair.shared_imports.len()
                ),
                severity: coupling_severity(pair.score),
                suggestion: if suggest {
                    Some(
                        "Consider introducing an interface or mediator to reduce direct coupling"
                            .to_string(),
                    )
                } else {
                    None
                },
            });
        }
    }
}

fn collect_dead_code_smells(path: &Path, suggest: bool, all_smells: &mut Vec<SmellFinding>) {
    if let Ok(dead_report) = crate::quality::dead_code::analyze_dead_code(path, None, &[]) {
        for func in &dead_report.dead_functions {
            all_smells.push(SmellFinding {
                smell_type: SmellType::DeadCode,
                file: func.file.clone(),
                name: func.name.clone(),
                line: func.line as u32,
                reason: format!("Unreachable function ({:?})", func.reason),
                severity: 1,
                suggestion: if suggest {
                    Some("Remove this function or add a call path to it".to_string())
                } else {
                    None
                },
            });
        }
    }
}

fn collect_code_clone_smells(path: &Path, suggest: bool, all_smells: &mut Vec<SmellFinding>) {
    let sim_options = crate::quality::similarity::SimilarityOptions {
        threshold: 0.6,
        max_functions: 500,
        max_pairs: 50,
    };
    if let Ok(sim_report) =
        crate::quality::similarity::find_similar_with_options(path, None, &sim_options)
    {
        for pair in &sim_report.similar_pairs {
            all_smells.push(SmellFinding {
                smell_type: SmellType::CodeClone,
                file: pair.func_a.file.clone(),
                name: format!("{} ~ {}", pair.func_a.name, pair.func_b.name),
                line: pair.func_a.line as u32,
                reason: format!(
                    "Similarity score {:.0}% with {}:{}",
                    pair.score * 100.0,
                    pair.func_b.file.display(),
                    pair.func_b.line
                ),
                severity: clone_severity(pair.score),
                suggestion: if suggest {
                    Some("Consider extracting shared logic into a common function".to_string())
                } else {
                    None
                },
            });
        }
    }
}

fn collect_high_cognitive_smells(path: &Path, suggest: bool, all_smells: &mut Vec<SmellFinding>) {
    let complexity_options = crate::quality::complexity::ComplexityOptions {
        hotspot_threshold: 10,
        max_hotspots: 100,
        include_cognitive: true,
    };
    if let Ok(complexity_report) =
        crate::quality::complexity::analyze_complexity(path, None, Some(complexity_options))
    {
        for func in &complexity_report.functions {
            if func.cognitive < 15 {
                continue;
            }
            all_smells.push(SmellFinding {
                smell_type: SmellType::HighCognitiveComplexity,
                file: func.file.clone(),
                name: func.name.clone(),
                line: func.line as u32,
                reason: format!("Cognitive complexity {} (threshold: 15)", func.cognitive),
                severity: cognitive_severity(func.cognitive),
                suggestion: if suggest {
                    Some(
                        "Simplify control flow, reduce nesting, or extract helper functions"
                            .to_string(),
                    )
                } else {
                    None
                },
            });
        }
    }
}

fn build_inheritance_report(path: &Path, needs_tier2: bool) -> Option<InheritanceReport> {
    if !needs_tier2 {
        return None;
    }
    use crate::inheritance::{extract_inheritance, InheritanceOptions};
    let options = InheritanceOptions::default();
    extract_inheritance(path, None, &options).ok()
}

fn collect_middle_man_smells(
    shared_call_graph_ir: Option<&CallGraphIR>,
    thresholds: &Thresholds,
    suggest: bool,
    all_smells: &mut Vec<SmellFinding>,
) {
    let Some(shared_call_graph_ir) = shared_call_graph_ir else {
        return;
    };
    for file_ir in shared_call_graph_ir.files.values() {
        let lang = inferred_language_name(&file_ir.path);
        let findings = detect_middle_man_from_callgraph(file_ir, thresholds, &lang, suggest);
        all_smells.extend(findings);
    }
}

fn collect_refused_bequest_smells(
    shared_call_graph_ir: Option<&CallGraphIR>,
    inheritance_report: Option<&InheritanceReport>,
    thresholds: &Thresholds,
    suggest: bool,
    all_smells: &mut Vec<SmellFinding>,
) {
    let (Some(shared_call_graph_ir), Some(inheritance_report)) =
        (shared_call_graph_ir, inheritance_report)
    else {
        return;
    };
    let findings =
        detect_refused_bequest_from_callgraph(shared_call_graph_ir, inheritance_report, thresholds, suggest);
    all_smells.extend(findings);
}

fn collect_feature_envy_smells(
    shared_call_graph_ir: Option<&CallGraphIR>,
    thresholds: &Thresholds,
    suggest: bool,
    all_smells: &mut Vec<SmellFinding>,
) {
    let Some(shared_call_graph_ir) = shared_call_graph_ir else {
        return;
    };
    for file_ir in shared_call_graph_ir.files.values() {
        let lang = inferred_language_name(&file_ir.path);
        let findings = detect_feature_envy_from_callgraph(file_ir, thresholds, &lang, suggest);
        all_smells.extend(findings);
    }
}

fn collect_inappropriate_intimacy_smells(
    shared_call_graph_ir: Option<&CallGraphIR>,
    inheritance_report: Option<&InheritanceReport>,
    thresholds: &Thresholds,
    suggest: bool,
    all_smells: &mut Vec<SmellFinding>,
) {
    let (Some(shared_call_graph_ir), Some(inheritance_report)) =
        (shared_call_graph_ir, inheritance_report)
    else {
        return;
    };
    let findings = detect_inappropriate_intimacy_from_callgraph(
        shared_call_graph_ir,
        inheritance_report,
        thresholds,
        suggest,
    );
    all_smells.extend(findings);
}

fn inferred_language_name(path: &Path) -> String {
    Language::from_path(path)
        .map(|l| l.to_string().to_lowercase())
        .unwrap_or_else(|| "python".to_string())
}

fn sort_smells(all_smells: &mut [SmellFinding]) {
    all_smells.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
}

fn build_smells_by_file(all_smells: &[SmellFinding]) -> HashMap<PathBuf, Vec<SmellFinding>> {
    let mut by_file: HashMap<PathBuf, Vec<SmellFinding>> = HashMap::new();
    for smell in all_smells {
        by_file
            .entry(smell.file.clone())
            .or_default()
            .push(smell.clone());
    }
    by_file
}

fn build_smells_summary(all_smells: &[SmellFinding], files_scanned: usize) -> SmellsSummary {
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for smell in all_smells {
        *by_type.entry(smell.smell_type.to_string()).or_insert(0) += 1;
    }
    SmellsSummary {
        total_smells: all_smells.len(),
        by_type,
        avg_smells_per_file: if files_scanned > 0 {
            all_smells.len() as f64 / files_scanned as f64
        } else {
            0.0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thresholds_default() {
        let t = Thresholds::from_preset(ThresholdPreset::Default);
        assert_eq!(t.god_class_methods, 20);
        assert_eq!(t.god_class_loc, 500);
        assert_eq!(t.long_method_loc, 50);
        assert_eq!(t.long_method_complexity, 10);
        assert_eq!(t.long_param_count, 5);
    }

    #[test]
    fn test_thresholds_strict() {
        let t = Thresholds::from_preset(ThresholdPreset::Strict);
        assert!(t.god_class_methods < 20);
        assert!(t.long_method_loc < 50);
    }

    #[test]
    fn test_thresholds_relaxed() {
        let t = Thresholds::from_preset(ThresholdPreset::Relaxed);
        assert!(t.god_class_methods > 20);
        assert!(t.long_method_loc > 50);
    }

    #[test]
    fn test_severity_calculation() {
        assert_eq!(calculate_severity(6, 5), 1);  // 20% over
        assert_eq!(calculate_severity(8, 5), 2);  // 60% over
        assert_eq!(calculate_severity(11, 5), 3); // 120% over
    }

    #[test]
    fn test_smell_type_display() {
        assert_eq!(SmellType::GodClass.to_string(), "God Class");
        assert_eq!(SmellType::LongMethod.to_string(), "Long Method");
        assert_eq!(SmellType::LongParameterList.to_string(), "Long Parameter List");
    }

    // =========================================================================
    // Tests for new aggregated smell types
    // =========================================================================

    #[test]
    fn test_new_smell_type_variants_exist() {
        // Verify the new SmellType variants are defined
        let _ = SmellType::LowCohesion;
        let _ = SmellType::TightCoupling;
        let _ = SmellType::DeadCode;
        let _ = SmellType::CodeClone;
        let _ = SmellType::HighCognitiveComplexity;
    }

    #[test]
    fn test_new_smell_type_display() {
        assert_eq!(SmellType::LowCohesion.to_string(), "Low Cohesion");
        assert_eq!(SmellType::TightCoupling.to_string(), "Tight Coupling");
        assert_eq!(SmellType::DeadCode.to_string(), "Dead Code");
        assert_eq!(SmellType::CodeClone.to_string(), "Code Clone");
        assert_eq!(SmellType::HighCognitiveComplexity.to_string(), "High Cognitive Complexity");
    }

    #[test]
    fn test_new_smell_types_serialize() {
        // Verify serde rename works for new variants
        let json = serde_json::to_string(&SmellType::LowCohesion).unwrap();
        assert_eq!(json, "\"low_cohesion\"");
        let json = serde_json::to_string(&SmellType::TightCoupling).unwrap();
        assert_eq!(json, "\"tight_coupling\"");
        let json = serde_json::to_string(&SmellType::DeadCode).unwrap();
        assert_eq!(json, "\"dead_code\"");
        let json = serde_json::to_string(&SmellType::CodeClone).unwrap();
        assert_eq!(json, "\"code_clone\"");
        let json = serde_json::to_string(&SmellType::HighCognitiveComplexity).unwrap();
        assert_eq!(json, "\"high_cognitive_complexity\"");
    }

    #[test]
    fn test_cohesion_severity_mapping() {
        // LCOM4 >= 6 => severity 3
        // LCOM4 >= 4 => severity 2
        // LCOM4 >= 2 => severity 1
        assert_eq!(cohesion_severity(6), 3);
        assert_eq!(cohesion_severity(7), 3);
        assert_eq!(cohesion_severity(4), 2);
        assert_eq!(cohesion_severity(5), 2);
        assert_eq!(cohesion_severity(2), 1);
        assert_eq!(cohesion_severity(3), 1);
    }

    #[test]
    fn test_coupling_severity_mapping() {
        // score >= 0.8 => severity 2
        // score >= 0.6 => severity 1
        assert_eq!(coupling_severity(0.9), 2);
        assert_eq!(coupling_severity(0.8), 2);
        assert_eq!(coupling_severity(0.7), 1);
        assert_eq!(coupling_severity(0.6), 1);
    }

    #[test]
    fn test_cognitive_severity_mapping() {
        // cognitive >= 30 => severity 3
        // cognitive >= 20 => severity 2
        // cognitive >= 15 => severity 1
        assert_eq!(cognitive_severity(30), 3);
        assert_eq!(cognitive_severity(35), 3);
        assert_eq!(cognitive_severity(20), 2);
        assert_eq!(cognitive_severity(25), 2);
        assert_eq!(cognitive_severity(15), 1);
        assert_eq!(cognitive_severity(18), 1);
    }

    #[test]
    fn test_clone_severity_mapping() {
        // score > 0.8 => severity 2
        // score > 0.6 => severity 1
        assert_eq!(clone_severity(0.85), 2);
        assert_eq!(clone_severity(0.81), 2);
        assert_eq!(clone_severity(0.7), 1);
        assert_eq!(clone_severity(0.61), 1);
    }

    #[test]
    fn test_analyze_smells_aggregated_exists() {
        // The aggregated function should exist and return a SmellsReport
        // Run on an empty temp directory to verify the function signature works
        let dir = std::env::temp_dir().join("tldr_smells_test_empty");
        let _ = std::fs::create_dir_all(&dir);

        let result = analyze_smells_aggregated(
            &dir,
            ThresholdPreset::Default,
            None,
            false,
        );
        assert!(result.is_ok());
        let report = result.unwrap();
        assert_eq!(report.smells.len(), 0);
        assert_eq!(report.files_scanned, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_analyze_smells_aggregated_filter_new_types() {
        // When filtering by a new smell type, existing smells should not appear
        let dir = std::env::temp_dir().join("tldr_smells_test_filter");
        let _ = std::fs::create_dir_all(&dir);

        let result = analyze_smells_aggregated(
            &dir,
            ThresholdPreset::Default,
            Some(SmellType::DeadCode),
            false,
        );
        assert!(result.is_ok());
        let report = result.unwrap();
        // All findings (if any) should be DeadCode type only
        for smell in &report.smells {
            assert_eq!(smell.smell_type, SmellType::DeadCode);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_backward_compatibility_detect_smells_unchanged() {
        // The original detect_smells should still work the same way
        let dir = std::env::temp_dir().join("tldr_smells_test_compat");
        let _ = std::fs::create_dir_all(&dir);

        let result = detect_smells(
            &dir,
            ThresholdPreset::Default,
            None,
            false,
        );
        assert!(result.is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // =========================================================================
    // Tests for 5 new Tier-1 AST-based smell detectors
    // =========================================================================

    // --- Deep Nesting tests ---

    #[test]
    fn test_deep_nesting_variant_exists() {
        let _ = SmellType::DeepNesting;
        assert_eq!(SmellType::DeepNesting.to_string(), "Deep Nesting");
    }

    #[test]
    fn test_deep_nesting_serialize() {
        let json = serde_json::to_string(&SmellType::DeepNesting).unwrap();
        assert_eq!(json, "\"deep_nesting\"");
    }

    #[test]
    fn test_detect_deep_nesting_python() {
        let source = r#"
def deeply_nested():
    if True:
        for i in range(10):
            while True:
                try:
                    if x > 0:
                        print("deep")
                except:
                    pass
"#;
        let findings = detect_deep_nesting(source, "python");
        assert!(!findings.is_empty(), "Should detect deep nesting (depth >= 5)");
        assert_eq!(findings[0].smell_type, SmellType::DeepNesting);
        assert!(findings[0].severity >= 1);
    }

    #[test]
    fn test_detect_deep_nesting_rust() {
        let source = r#"
fn deeply_nested() {
    if true {
        for i in 0..10 {
            while true {
                if let Some(x) = foo() {
                    if x > 0 {
                        println!("deep");
                    }
                }
            }
        }
    }
}
"#;
        let findings = detect_deep_nesting(source, "rust");
        assert!(!findings.is_empty(), "Should detect deep nesting in Rust");
    }

    #[test]
    fn test_no_deep_nesting_shallow() {
        let source = r#"
def shallow():
    if True:
        for i in range(10):
            print(i)
"#;
        let findings = detect_deep_nesting(source, "python");
        assert!(findings.is_empty(), "Shallow code should not trigger deep nesting smell");
    }

    #[test]
    fn test_deep_nesting_severity_levels() {
        // depth >= 8 => severity 3
        // depth >= 6 => severity 2
        // depth >= 5 => severity 1
        assert_eq!(nesting_severity(8), 3);
        assert_eq!(nesting_severity(10), 3);
        assert_eq!(nesting_severity(6), 2);
        assert_eq!(nesting_severity(7), 2);
        assert_eq!(nesting_severity(5), 1);
    }

    // --- Data Class tests ---

    #[test]
    fn test_data_class_variant_exists() {
        let _ = SmellType::DataClass;
        assert_eq!(SmellType::DataClass.to_string(), "Data Class");
    }

    #[test]
    fn test_data_class_serialize() {
        let json = serde_json::to_string(&SmellType::DataClass).unwrap();
        assert_eq!(json, "\"data_class\"");
    }

    #[test]
    fn test_detect_data_class_python() {
        let source = r#"
class UserData:
    def __init__(self):
        self.name = ""
        self.email = ""
        self.age = 0
        self.address = ""
        self.phone = ""
"#;
        let findings = detect_data_classes(source, "python");
        assert!(!findings.is_empty(), "Class with 5 fields and 1 method should be a data class");
        assert_eq!(findings[0].smell_type, SmellType::DataClass);
    }

    #[test]
    fn test_no_data_class_with_methods() {
        let source = r#"
class UserService:
    def __init__(self):
        self.name = ""
        self.email = ""

    def validate(self):
        pass

    def save(self):
        pass

    def send_email(self):
        pass
"#;
        let findings = detect_data_classes(source, "python");
        assert!(findings.is_empty(), "Class with many methods should not be a data class");
    }

    #[test]
    fn test_data_class_severity_levels() {
        // >= 8 fields, 0 methods => severity 2
        // >= 4 fields, <= 2 methods => severity 1
        assert_eq!(data_class_severity(8, 0), 2);
        assert_eq!(data_class_severity(10, 0), 2);
        assert_eq!(data_class_severity(4, 1), 1);
        assert_eq!(data_class_severity(5, 2), 1);
    }

    // --- Lazy Element tests ---

    #[test]
    fn test_lazy_element_variant_exists() {
        let _ = SmellType::LazyElement;
        assert_eq!(SmellType::LazyElement.to_string(), "Lazy Element");
    }

    #[test]
    fn test_lazy_element_serialize() {
        let json = serde_json::to_string(&SmellType::LazyElement).unwrap();
        assert_eq!(json, "\"lazy_element\"");
    }

    #[test]
    fn test_detect_lazy_element_python() {
        let source = r#"
class Wrapper:
    def do_thing(self):
        pass
"#;
        let findings = detect_lazy_elements(source, "python");
        assert!(!findings.is_empty(), "Class with 1 method and 0 fields should be a lazy element");
        assert_eq!(findings[0].smell_type, SmellType::LazyElement);
        assert_eq!(findings[0].severity, 1);
    }

    #[test]
    fn test_no_lazy_element_with_enough_content() {
        let source = r#"
class Service:
    def __init__(self):
        self.name = ""
        self.id = 0

    def run(self):
        pass

    def stop(self):
        pass
"#;
        let findings = detect_lazy_elements(source, "python");
        assert!(findings.is_empty(), "Class with 2+ methods and 2+ fields should not be lazy");
    }

    // --- Message Chain tests ---

    #[test]
    fn test_message_chain_variant_exists() {
        let _ = SmellType::MessageChain;
        assert_eq!(SmellType::MessageChain.to_string(), "Message Chain");
    }

    #[test]
    fn test_message_chain_serialize() {
        let json = serde_json::to_string(&SmellType::MessageChain).unwrap();
        assert_eq!(json, "\"message_chain\"");
    }

    #[test]
    fn test_detect_message_chains_python() {
        let source = r#"
def process():
    result = obj.get_manager().get_department().get_employees().get_first().name
"#;
        let findings = detect_message_chains(source, "python");
        assert!(!findings.is_empty(), "Should detect long method chain (> 3 calls)");
        assert_eq!(findings[0].smell_type, SmellType::MessageChain);
    }

    #[test]
    fn test_no_message_chain_short() {
        let source = r#"
def simple():
    result = obj.get_name().strip()
"#;
        let findings = detect_message_chains(source, "python");
        assert!(findings.is_empty(), "Short chains (<=3) should not trigger");
    }

    #[test]
    fn test_message_chain_severity_levels() {
        // chain >= 6 => severity 2
        // chain >= 4 => severity 1
        assert_eq!(chain_severity(6), 2);
        assert_eq!(chain_severity(8), 2);
        assert_eq!(chain_severity(4), 1);
        assert_eq!(chain_severity(5), 1);
    }

    // --- Primitive Obsession tests ---

    #[test]
    fn test_primitive_obsession_variant_exists() {
        let _ = SmellType::PrimitiveObsession;
        assert_eq!(SmellType::PrimitiveObsession.to_string(), "Primitive Obsession");
    }

    #[test]
    fn test_primitive_obsession_serialize() {
        let json = serde_json::to_string(&SmellType::PrimitiveObsession).unwrap();
        assert_eq!(json, "\"primitive_obsession\"");
    }

    #[test]
    fn test_detect_primitive_obsession_python() {
        let source = r#"
def create_user(name: str, email: str, age: int, phone: str, active: bool):
    pass
"#;
        let findings = detect_primitive_obsession(source, "python");
        assert!(!findings.is_empty(), "Function with 5 primitive params should trigger");
        assert_eq!(findings[0].smell_type, SmellType::PrimitiveObsession);
    }

    #[test]
    fn test_detect_primitive_obsession_rust() {
        let source = r#"
fn create_user(name: &str, email: String, age: i32, phone: String, active: bool, score: f64) {
}
"#;
        let findings = detect_primitive_obsession(source, "rust");
        assert!(!findings.is_empty(), "Function with 6 primitive params should trigger in Rust");
        assert!(findings[0].severity >= 2, "6 primitives should have severity >= 2");
    }

    #[test]
    fn test_no_primitive_obsession_with_domain_types() {
        let source = r#"
def create_user(config: UserConfig, permissions: PermissionSet):
    pass
"#;
        let findings = detect_primitive_obsession(source, "python");
        assert!(findings.is_empty(), "Non-primitive params should not trigger");
    }

    #[test]
    fn test_primitive_obsession_severity_levels() {
        // >= 6 primitives => severity 2
        // >= 4 primitives => severity 1
        assert_eq!(primitive_obsession_severity(6), 2);
        assert_eq!(primitive_obsession_severity(8), 2);
        assert_eq!(primitive_obsession_severity(4), 1);
        assert_eq!(primitive_obsession_severity(5), 1);
    }

    // --- Integration: new smells in detect_smells and aggregator ---

    #[test]
    fn test_detect_smells_includes_new_types_on_files() {
        // Create a temp file with deep nesting to verify detect_smells picks it up
        let dir = std::env::temp_dir().join("tldr_smells_new_types_test");
        let _ = std::fs::create_dir_all(&dir);
        let py_file = dir.join("nested.py");
        std::fs::write(&py_file, r#"
def deeply_nested():
    if True:
        for i in range(10):
            while True:
                try:
                    if x > 0:
                        print("deep")
                except:
                    pass
"#).unwrap();

        let result = detect_smells(
            &dir,
            ThresholdPreset::Default,
            Some(SmellType::DeepNesting),
            false,
        );
        assert!(result.is_ok());
        let report = result.unwrap();
        for smell in &report.smells {
            assert_eq!(smell.smell_type, SmellType::DeepNesting);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_aggregated_includes_new_types() {
        // When --deep mode runs, new smell types should be detected
        let dir = std::env::temp_dir().join("tldr_smells_new_agg_test");
        let _ = std::fs::create_dir_all(&dir);
        let py_file = dir.join("data.py");
        std::fs::write(&py_file, r#"
class BigDataBag:
    def __init__(self):
        self.a = 1
        self.b = 2
        self.c = 3
        self.d = 4
        self.e = 5
        self.f = 6
        self.g = 7
        self.h = 8
"#).unwrap();

        let result = analyze_smells_aggregated(
            &dir,
            ThresholdPreset::Default,
            Some(SmellType::DataClass),
            false,
        );
        assert!(result.is_ok());
        let report = result.unwrap();
        for smell in &report.smells {
            assert_eq!(smell.smell_type, SmellType::DataClass);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Parse failure resilience ---

    #[test]
    fn test_detectors_handle_invalid_source() {
        let bad_source = "{{{{not valid code at all}}}}";
        assert!(detect_deep_nesting(bad_source, "python").is_empty());
        assert!(detect_data_classes(bad_source, "python").is_empty());
        assert!(detect_lazy_elements(bad_source, "python").is_empty());
        assert!(detect_message_chains(bad_source, "python").is_empty());
        assert!(detect_primitive_obsession(bad_source, "python").is_empty());
    }

    #[test]
    fn test_detectors_handle_unknown_language() {
        let source = "print('hello')";
        assert!(detect_deep_nesting(source, "brainfuck").is_empty());
        assert!(detect_data_classes(source, "brainfuck").is_empty());
        assert!(detect_lazy_elements(source, "brainfuck").is_empty());
        assert!(detect_message_chains(source, "brainfuck").is_empty());
        assert!(detect_primitive_obsession(source, "brainfuck").is_empty());
    }

    // =========================================================================
    // Tests for 4 new Tier-2 Fowler smell detectors
    // =========================================================================

    // --- Middle Man tests ---

    #[test]
    fn test_middle_man_variant_exists() {
        let _ = SmellType::MiddleMan;
        assert_eq!(SmellType::MiddleMan.to_string(), "Middle Man");
    }

    #[test]
    fn test_middle_man_serialize() {
        let json = serde_json::to_string(&SmellType::MiddleMan).unwrap();
        assert_eq!(json, "\"middle_man\"");
    }

    // --- Helper to build FileIR for Middle Man tests ---

    /// Build a FileIR with a class, its methods (as FuncDefs), and call data.
    /// Each method entry is (name, calls) where calls is a list of
    /// (target, receiver, receiver_type) representing method calls made.
    type MethodCallTriple<'a> = (&'a str, &'a str, &'a str);
    type MiddleManMethod<'a> = (&'a str, Vec<MethodCallTriple<'a>>);

    fn build_middle_man_file_ir(
        class_name: &str,
        constructor_name: Option<&str>,
        methods: Vec<MiddleManMethod<'_>>,
    ) -> FileIR {
        use crate::callgraph::cross_file_types::{CallSite, ClassDef};
        let mut file_ir = FileIR::new(PathBuf::from("test.py"));

        let mut method_names: Vec<String> = Vec::new();
        let mut line = 1u32;

        // Add constructor if specified
        if let Some(ctor) = constructor_name {
            method_names.push(ctor.to_string());
            file_ir.funcs.push(FuncDef::method(ctor, class_name, line, line + 2));
            line += 3;
        }

        // Add each method as a FuncDef and its calls
        for (method_name, calls) in &methods {
            method_names.push(method_name.to_string());
            file_ir.funcs.push(FuncDef::method(*method_name, class_name, line, line + 2));

            // Add calls for this method
            let qualified = format!("{}.{}", class_name, method_name);
            let call_sites: Vec<CallSite> = calls.iter().map(|(target, receiver, receiver_type)| {
                CallSite::method(
                    qualified.clone(),
                    *target,
                    *receiver,
                    if receiver_type.is_empty() { None } else { Some(receiver_type.to_string()) },
                    Some(line + 1),
                )
            }).collect();

            if !call_sites.is_empty() {
                file_ir.calls.insert(qualified, call_sites);
            }

            line += 3;
        }

        // Add ClassDef
        file_ir.classes.push(ClassDef::new(
            class_name.to_string(),
            1,
            line,
            method_names,
            vec![],
        ));

        file_ir
    }

    #[test]
    fn test_detect_middle_man_pure_delegator() {
        // 4/4 non-constructor methods delegate to order => 100% delegation
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("__init__"),
            vec![
                ("get_total", vec![("get_total", "order", "Order")]),
                ("get_items", vec![("get_items", "order", "Order")]),
                ("get_customer", vec![("get_customer", "order", "Order")]),
                ("get_status", vec![("get_status", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "Class with 4/4 delegating methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
        assert!(findings[0].severity >= 2, "100% delegation with 4 methods should have severity >= 2");
    }

    #[test]
    fn test_no_middle_man_mixed_logic() {
        // 1 delegator, 2 with self-calls or multiple calls => 33% delegation
        let file_ir = {
            use crate::callgraph::cross_file_types::{CallSite, ClassDef};
            let mut fir = FileIR::new(PathBuf::from("test.py"));
            // __init__
            fir.funcs.push(FuncDef::method("__init__", "OrderService", 1, 3));
            // get_total: pure delegation to order.get_total()
            fir.funcs.push(FuncDef::method("get_total", "OrderService", 4, 6));
            fir.calls.insert("OrderService.get_total".to_string(), vec![
                CallSite::method("OrderService.get_total", "get_total", "order", Some("Order".to_string()), Some(5)),
            ]);
            // validate: calls self and order (2 method calls, not pure delegation)
            fir.funcs.push(FuncDef::method("validate", "OrderService", 7, 10));
            fir.calls.insert("OrderService.validate".to_string(), vec![
                CallSite::method("OrderService.validate", "get_total", "self", None, Some(8)),
                CallSite::method("OrderService.validate", "get_total", "order", Some("Order".to_string()), Some(9)),
            ]);
            // process_payment: calls self.get_total() (self-call, not delegation to external)
            fir.funcs.push(FuncDef::method("process_payment", "OrderService", 11, 14));
            fir.calls.insert("OrderService.process_payment".to_string(), vec![
                CallSite::method("OrderService.process_payment", "get_total", "self", None, Some(12)),
            ]);
            fir.classes.push(ClassDef::new(
                "OrderService".to_string(), 1, 14,
                vec!["__init__".to_string(), "get_total".to_string(), "validate".to_string(), "process_payment".to_string()],
                vec![],
            ));
            fir
        };
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Class with 1/3 delegation (33%) should not be middle man");
    }

    #[test]
    fn test_middle_man_threshold_boundary() {
        // 3 out of 5 non-constructor methods delegate = 60%, exactly at default threshold
        let file_ir = {
            use crate::callgraph::cross_file_types::{CallSite, ClassDef};
            let mut fir = FileIR::new(PathBuf::from("test.py"));
            fir.funcs.push(FuncDef::method("__init__", "Delegator", 1, 3));
            // 3 pure delegators
            for (i, name) in ["method1", "method2", "method3"].iter().enumerate() {
                let line = (4 + i * 3) as u32;
                fir.funcs.push(FuncDef::method(*name, "Delegator", line, line + 2));
                fir.calls.insert(format!("Delegator.{}", name), vec![
                    CallSite::method(
                        format!("Delegator.{}", name), *name, "backend",
                        Some("Backend".to_string()), Some(line + 1),
                    ),
                ]);
            }
            // 2 non-delegators (no calls = real logic or empty)
            fir.funcs.push(FuncDef::method("method4", "Delegator", 13, 16));
            fir.funcs.push(FuncDef::method("method5", "Delegator", 17, 19));
            fir.classes.push(ClassDef::new(
                "Delegator".to_string(), 1, 19,
                vec!["__init__".to_string(), "method1".to_string(), "method2".to_string(),
                     "method3".to_string(), "method4".to_string(), "method5".to_string()],
                vec![],
            ));
            fir
        };
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        // 3/5 = 60% >= 60% threshold => should trigger
        assert!(!findings.is_empty(), "60% delegation at default threshold (60%) should trigger middle man");
    }

    #[test]
    fn test_middle_man_too_small_class() {
        // Only 1 non-constructor method - below min_methods (3)
        let file_ir = build_middle_man_file_ir(
            "TinyHelper",
            Some("__init__"),
            vec![
                ("do_thing", vec![("do_thing", "obj", "Target")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Single non-constructor method class should not trigger middle man (below min_methods)");
    }

    #[test]
    fn test_middle_man_constructor_excluded() {
        // 3 delegating methods (excluding __init__) = 100% of non-constructor methods
        let file_ir = build_middle_man_file_ir(
            "Forwarder",
            Some("__init__"),
            vec![
                ("forward1", vec![("method1", "target", "Target")]),
                ("forward2", vec![("method2", "target", "Target")]),
                ("forward3", vec![("method3", "target", "Target")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "Constructor should be excluded from method count; 3/3 delegation = 100%");
    }

    // --- New Middle Man boundary tests ---

    #[test]
    fn test_middle_man_facade_exclusion() {
        // Class named "UserFacade" with 100% delegation should NOT trigger (facade exclusion)
        let file_ir = build_middle_man_file_ir(
            "UserFacade",
            Some("__init__"),
            vec![
                ("get_user", vec![("get_user", "repo", "UserRepo")]),
                ("save_user", vec![("save_user", "repo", "UserRepo")]),
                ("delete_user", vec![("delete_user", "repo", "UserRepo")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Class named 'UserFacade' should be excluded by facade heuristic");
    }

    #[test]
    fn test_middle_man_at_threshold() {
        // Exactly at 60% boundary: 3/5 = 60% at Default threshold (0.60)
        // This should trigger (>= threshold)
        let file_ir = build_middle_man_file_ir(
            "ExactBoundary",
            None, // no constructor
            vec![
                ("delegate1", vec![("op1", "svc", "Service")]),
                ("delegate2", vec![("op2", "svc", "Service")]),
                ("delegate3", vec![("op3", "svc", "Service")]),
                ("real_logic1", vec![]), // no calls = not a delegator
                ("real_logic2", vec![]), // no calls = not a delegator
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "3/5 = 60% should trigger at default 60% threshold");

        // Just below threshold: 2/5 = 40%
        let file_ir_below = build_middle_man_file_ir(
            "BelowBoundary",
            None,
            vec![
                ("delegate1", vec![("op1", "svc", "Service")]),
                ("delegate2", vec![("op2", "svc", "Service")]),
                ("real_logic1", vec![]),
                ("real_logic2", vec![]),
                ("real_logic3", vec![]),
            ],
        );
        let findings_below = detect_middle_man_from_callgraph(&file_ir_below, &thresholds, "python", false);
        assert!(findings_below.is_empty(), "2/5 = 40% should NOT trigger at default 60% threshold");
    }

    #[test]
    fn test_middle_man_severity_levels_integration() {
        // Severity 1: 60% delegation, 3 delegators
        let file_ir_sev1 = build_middle_man_file_ir(
            "MildDelegator",
            None,
            vec![
                ("d1", vec![("op1", "svc", "Service")]),
                ("d2", vec![("op2", "svc", "Service")]),
                ("d3", vec![("op3", "svc", "Service")]),
                ("real1", vec![]),
                ("real2", vec![]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings1 = detect_middle_man_from_callgraph(&file_ir_sev1, &thresholds, "python", false);
        assert!(!findings1.is_empty());
        assert_eq!(findings1[0].severity, 1, "60% with 3 delegators = severity 1");

        // Severity 2: 80% delegation, 4 delegators
        let file_ir_sev2 = build_middle_man_file_ir(
            "HeavyDelegator",
            None,
            vec![
                ("d1", vec![("op1", "svc", "Service")]),
                ("d2", vec![("op2", "svc", "Service")]),
                ("d3", vec![("op3", "svc", "Service")]),
                ("d4", vec![("op4", "svc", "Service")]),
                ("real1", vec![]),
            ],
        );
        let findings2 = detect_middle_man_from_callgraph(&file_ir_sev2, &thresholds, "python", false);
        assert!(!findings2.is_empty());
        assert_eq!(findings2[0].severity, 2, "80% with 4 delegators = severity 2");

        // Severity 3: 100% delegation, 6 delegators
        let file_ir_sev3 = build_middle_man_file_ir(
            "TotalDelegator",
            None,
            vec![
                ("d1", vec![("op1", "svc", "Service")]),
                ("d2", vec![("op2", "svc", "Service")]),
                ("d3", vec![("op3", "svc", "Service")]),
                ("d4", vec![("op4", "svc", "Service")]),
                ("d5", vec![("op5", "svc", "Service")]),
                ("d6", vec![("op6", "svc", "Service")]),
            ],
        );
        let findings3 = detect_middle_man_from_callgraph(&file_ir_sev3, &thresholds, "python", false);
        assert!(!findings3.is_empty());
        assert_eq!(findings3[0].severity, 3, "100% with 6 delegators = severity 3");
    }

    // --- Refused Bequest tests ---

    #[test]
    fn test_refused_bequest_variant_exists() {
        let _ = SmellType::RefusedBequest;
        assert_eq!(SmellType::RefusedBequest.to_string(), "Refused Bequest");
    }

    #[test]
    fn test_refused_bequest_serialize() {
        let json = serde_json::to_string(&SmellType::RefusedBequest).unwrap();
        assert_eq!(json, "\"refused_bequest\"");
    }

    // Helper: build a CallGraphIR + InheritanceReport for Refused Bequest tests.
    // Parent class has `parent_methods` methods in parent_file.
    // Child class has `child_methods` methods in child_file.
    // The inheritance edge connects child -> parent.
    fn build_refused_bequest_test_data(
        parent_name: &str,
        parent_methods: &[&str],
        child_name: &str,
        child_methods: &[&str],
        edge_builder: impl FnOnce() -> crate::types::inheritance::InheritanceEdge,
        parent_node_builder: impl FnOnce() -> crate::types::inheritance::InheritanceNode,
        child_calls: &[(/* child_method */ &str, /* call_target */ &str)],
    ) -> (CallGraphIR, InheritanceReport) {
        use crate::callgraph::cross_file_types::{ClassDef, FuncDef, CallSite, CallType};
        use crate::types::inheritance::{InheritanceReport, InheritanceNode};

        let parent_file_path = PathBuf::from("parent.py");
        let child_file_path = PathBuf::from("child.py");

        // Build parent FileIR
        let mut parent_file_ir = FileIR::new(parent_file_path.clone());
        parent_file_ir.classes.push(ClassDef::new(
            parent_name.to_string(),
            1,
            (parent_methods.len() as u32 * 3) + 1,
            parent_methods.iter().map(|m| m.to_string()).collect(),
            vec![],
        ));
        for (i, method_name) in parent_methods.iter().enumerate() {
            let line = (i as u32 * 3) + 2;
            parent_file_ir.funcs.push(FuncDef::method(
                *method_name,
                parent_name,
                line,
                line + 2,
            ));
        }

        // Build child FileIR
        let mut child_file_ir = FileIR::new(child_file_path.clone());
        child_file_ir.classes.push(ClassDef::new(
            child_name.to_string(),
            1,
            (child_methods.len() as u32 * 3) + 1,
            child_methods.iter().map(|m| m.to_string()).collect(),
            vec![parent_name.to_string()],
        ));
        for (i, method_name) in child_methods.iter().enumerate() {
            let line = (i as u32 * 3) + 2;
            child_file_ir.funcs.push(FuncDef::method(
                *method_name,
                child_name,
                line,
                line + 2,
            ));
        }

        // Add child calls (e.g., calls to parent methods via super())
        for (child_method, call_target) in child_calls {
            let qualified = format!("{}.{}", child_name, child_method);
            child_file_ir.add_call(
                &qualified,
                CallSite::new(
                    qualified.clone(),
                    call_target.to_string(),
                    CallType::Method,
                    Some(2),
                    None,
                    Some("super".to_string()),
                    Some(parent_name.to_string()),
                ),
            );
        }

        // Build CallGraphIR
        let mut cg = CallGraphIR::new(PathBuf::from("."), "python");
        cg.files.insert(parent_file_path, parent_file_ir);
        cg.files.insert(child_file_path, child_file_ir);

        // Build InheritanceReport
        let edge = edge_builder();
        let parent_node = parent_node_builder();
        let child_node = InheritanceNode::new(
            child_name,
            PathBuf::from("child.py"),
            1,
            crate::types::Language::Python,
        ).with_base(parent_name.to_string());

        let ir = InheritanceReport {
            edges: vec![edge],
            nodes: vec![parent_node, child_node],
            roots: vec![parent_name.to_string()],
            leaves: vec![child_name.to_string()],
            count: 2,
            languages: vec![crate::types::Language::Python],
            diamonds: vec![],
            project_path: PathBuf::from("."),
            scan_time_ms: 0,
        };

        (cg, ir)
    }

    #[test]
    fn test_detect_refused_bequest() {
        // ChildService overrides only method1 out of 10 parent methods = 10% usage
        // Below 33% threshold -> should trigger
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let parent_methods: Vec<&str> = (1..=10).map(|i| match i {
            1 => "method1", 2 => "method2", 3 => "method3", 4 => "method4",
            5 => "method5", 6 => "method6", 7 => "method7", 8 => "method8",
            9 => "method9", _ => "method10",
        }).collect();

        let (cg, ir) = build_refused_bequest_test_data(
            "BaseService",
            &parent_methods,
            "ChildService",
            &["method1", "custom_method"],
            || InheritanceEdge::project(
                "ChildService", "BaseService",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "BaseService", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[], // method1 is overridden (name match), no explicit calls
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(!findings.is_empty(), "Subclass using 1/10 (10%) of inherited methods should trigger");
        assert_eq!(findings[0].smell_type, SmellType::RefusedBequest);
    }

    #[test]
    fn test_no_refused_bequest_good_inheritance() {
        // Dog overrides all 3 of Animal's methods = 100% usage -> no smell
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "Animal",
            &["eat", "sleep", "move_around"],
            "Dog",
            &["eat", "sleep", "move_around", "bark"],
            || InheritanceEdge::project(
                "Dog", "Animal",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "Animal", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[], // All 3 methods overridden by name match
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Subclass using 100% of inherited methods should not trigger");
    }

    #[test]
    fn test_refused_bequest_skip_external() {
        // External base class -> edge.external = true -> should skip
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "SomeExternalBase",
            &["ext_method1", "ext_method2", "ext_method3", "ext_method4"],
            "MyClass",
            &["my_method"],
            || InheritanceEdge::unresolved(
                "MyClass", "SomeExternalBase",
                PathBuf::from("child.py"), 1,
            ),
            || InheritanceNode::new(
                "SomeExternalBase", PathBuf::from("external.py"), 1,
                crate::types::Language::Python,
            ),
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Should skip when base class is external/unresolved");
    }

    #[test]
    fn test_refused_bequest_multiple_bases() {
        // Child extends Base1 (method1, method2) - overrides method1 -> 1/2 = 50% per edge
        // Child extends Base2 (method3, method4) - overrides nothing -> 0/2 = 0% per edge
        // With min_inherited=3, each edge has only 2 methods, so neither triggers individually.
        // But if we increase parent methods to 4 each, then:
        // Base1: 1/4 = 25% usage -> below 33% -> triggers
        // Base2: 0/4 = 0% usage -> below 33% -> triggers
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode, InheritanceReport};
        use crate::callgraph::cross_file_types::{ClassDef, FuncDef};

        let parent1_path = PathBuf::from("base1.py");
        let _parent2_path = PathBuf::from("base2.py");
        let child_path = PathBuf::from("child.py");

        // Build Base1 FileIR
        let mut base1_ir = FileIR::new(parent1_path.clone());
        base1_ir.classes.push(ClassDef::new(
            "Base1".to_string(), 1, 15,
            vec!["method1".into(), "method2".into(), "method3".into(), "method4".into()],
            vec![],
        ));
        for (i, name) in ["method1", "method2", "method3", "method4"].iter().enumerate() {
            let line = (i as u32 * 3) + 2;
            base1_ir.funcs.push(FuncDef::method(*name, "Base1", line, line + 2));
        }

        // Build child FileIR with method1 override
        let mut child_ir = FileIR::new(child_path.clone());
        child_ir.classes.push(ClassDef::new(
            "Child".to_string(), 1, 10,
            vec!["method1".into(), "custom".into()],
            vec!["Base1".into()],
        ));
        child_ir.funcs.push(FuncDef::method("method1", "Child", 2, 4));
        child_ir.funcs.push(FuncDef::method("custom", "Child", 5, 7));

        // Build CallGraphIR
        let mut cg = CallGraphIR::new(PathBuf::from("."), "python");
        cg.files.insert(parent1_path.clone(), base1_ir);
        cg.files.insert(child_path, child_ir);

        // Build InheritanceReport with one edge: Child -> Base1
        let edge = InheritanceEdge::project(
            "Child", "Base1",
            PathBuf::from("child.py"), 1,
            parent1_path, 1,
        );
        let base1_node = InheritanceNode::new(
            "Base1", PathBuf::from("base1.py"), 1, crate::types::Language::Python,
        );
        let child_node = InheritanceNode::new(
            "Child", PathBuf::from("child.py"), 1, crate::types::Language::Python,
        ).with_base("Base1".to_string());

        let ir = InheritanceReport {
            edges: vec![edge],
            nodes: vec![base1_node, child_node],
            roots: vec!["Base1".to_string()],
            leaves: vec!["Child".to_string()],
            count: 2,
            languages: vec![crate::types::Language::Python],
            diamonds: vec![],
            project_path: PathBuf::from("."),
            scan_time_ms: 0,
        };

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        // Child overrides method1 out of 4 = 25% usage < 33% threshold
        assert!(!findings.is_empty(), "1/4 methods used (25%) should trigger refused bequest");
    }

    #[test]
    fn test_refused_bequest_override_is_use() {
        // Child overrides 2 out of 5 parent methods = 40% usage
        // Above 33% threshold -> should NOT trigger
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "Base",
            &["method1", "method2", "method3", "method4", "method5"],
            "Child",
            &["method1", "method2"], // overrides 2 of 5
            || InheritanceEdge::project(
                "Child", "Base",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "Base", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[], // Overrides counted by name match, no explicit calls needed
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Overriding counts as using - 2/5 = 40% should not trigger (threshold 33%)");
    }

    // --- New Refused Bequest tests (Phase 3) ---

    #[test]
    fn test_refused_bequest_abstract_parent_excluded() {
        // Parent is abstract -> should be skipped entirely (C5)
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "AbstractBase",
            &["method1", "method2", "method3", "method4", "method5"],
            "ConcreteChild",
            &["custom_only"], // uses 0/5 = 0% -> would trigger if not excluded
            || InheritanceEdge::project(
                "ConcreteChild", "AbstractBase",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "AbstractBase", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ).as_abstract(), // Mark parent as abstract
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Abstract parent should be excluded from refused bequest check");
    }

    #[test]
    fn test_refused_bequest_go_embedding_excluded() {
        // Go struct embedding (kind = Embeds) -> should be skipped (XL2)
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode, InheritanceKind};

        let (cg, ir) = build_refused_bequest_test_data(
            "BaseStruct",
            &["Method1", "Method2", "Method3", "Method4"],
            "ChildStruct",
            &["CustomMethod"], // uses 0/4 = 0% -> would trigger if not excluded
            || InheritanceEdge::project(
                "ChildStruct", "BaseStruct",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ).with_kind(InheritanceKind::Embeds), // Go embedding
            || InheritanceNode::new(
                "BaseStruct", PathBuf::from("parent.py"), 1,
                crate::types::Language::Go,
            ),
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Go embedding (Embeds kind) should be excluded from refused bequest check");
    }

    #[test]
    fn test_refused_bequest_override_counts_as_usage() {
        // Explicit test: child overriding a parent method counts as "using" it.
        // 3 out of 5 parent methods overridden = 60% usage -> above 33% -> no smell
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "Parent",
            &["render", "validate", "save", "delete", "archive"],
            "SpecialChild",
            &["render", "validate", "save"], // overrides 3 of 5 = 60%
            || InheritanceEdge::project(
                "SpecialChild", "Parent",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "Parent", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[], // Overrides by name match only
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "3/5 overrides (60%) should not trigger (threshold 33%)");
    }

    #[test]
    fn test_refused_bequest_mixin_parent_excluded() {
        // Parent is a mixin -> should be skipped (C5)
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "LoggingMixin",
            &["log_info", "log_debug", "log_error", "log_warning"],
            "MyService",
            &["process"], // uses 0/4 = 0% -> would trigger if not excluded
            || InheritanceEdge::project(
                "MyService", "LoggingMixin",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "LoggingMixin", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ).as_mixin(), // Mark parent as mixin
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Mixin parent should be excluded from refused bequest check");
    }

    #[test]
    fn test_refused_bequest_protocol_parent_excluded() {
        // Parent is a Protocol -> should be skipped (C5)
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "Renderable",
            &["render", "to_html", "to_json", "to_xml"],
            "SimpleRenderer",
            &["custom_render"], // uses 0/4 = 0% -> would trigger if not excluded
            || InheritanceEdge::project(
                "SimpleRenderer", "Renderable",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "Renderable", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ).as_protocol(), // Mark parent as protocol
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Protocol parent should be excluded from refused bequest check");
    }

    #[test]
    fn test_refused_bequest_implements_kind_excluded() {
        // Interface implementation (kind = Implements) -> should be skipped
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode, InheritanceKind};

        let (cg, ir) = build_refused_bequest_test_data(
            "Repository",
            &["find", "save", "delete", "update"],
            "UserRepo",
            &["custom_find"], // uses 0/4 = 0%
            || InheritanceEdge::project(
                "UserRepo", "Repository",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ).with_kind(InheritanceKind::Implements),
            || InheritanceNode::new(
                "Repository", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ).as_interface(),
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Interface implementation (Implements kind) should be excluded");
    }

    #[test]
    fn test_refused_bequest_with_suggestion() {
        // Same as basic test but with suggest=true to verify suggestion text
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let parent_methods: Vec<&str> = (1..=10).map(|i| match i {
            1 => "method1", 2 => "method2", 3 => "method3", 4 => "method4",
            5 => "method5", 6 => "method6", 7 => "method7", 8 => "method8",
            9 => "method9", _ => "method10",
        }).collect();

        let (cg, ir) = build_refused_bequest_test_data(
            "BaseService",
            &parent_methods,
            "ChildService",
            &["method1", "custom_method"],
            || InheritanceEdge::project(
                "ChildService", "BaseService",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "BaseService", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, true);
        assert!(!findings.is_empty(), "Should trigger with suggest=true");
        assert!(findings[0].suggestion.is_some(), "Should include a suggestion");
        let suggestion = findings[0].suggestion.as_ref().unwrap();
        assert!(suggestion.contains("composition"), "Suggestion should mention composition");
    }

    #[test]
    fn test_refused_bequest_child_calling_parent_method_is_usage() {
        // Child has no override but calls parent methods through call graph
        // Calls 2 out of 5 = 40% usage -> above 33% threshold -> no smell
        use crate::types::inheritance::{InheritanceEdge, InheritanceNode};

        let (cg, ir) = build_refused_bequest_test_data(
            "Base",
            &["method1", "method2", "method3", "method4", "method5"],
            "Child",
            &["do_work", "do_other"], // No name matches with parent
            || InheritanceEdge::project(
                "Child", "Base",
                PathBuf::from("child.py"), 1,
                PathBuf::from("parent.py"), 1,
            ),
            || InheritanceNode::new(
                "Base", PathBuf::from("parent.py"), 1,
                crate::types::Language::Python,
            ),
            &[
                ("do_work", "method1"),     // Calls parent's method1
                ("do_other", "method2"),    // Calls parent's method2
            ],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Calling 2/5 parent methods (40%) should not trigger");
    }

    // --- Feature Envy tests ---

    #[test]
    fn test_feature_envy_variant_exists() {
        let _ = SmellType::FeatureEnvy;
        assert_eq!(SmellType::FeatureEnvy.to_string(), "Feature Envy");
    }

    #[test]
    fn test_feature_envy_serialize() {
        let json = serde_json::to_string(&SmellType::FeatureEnvy).unwrap();
        assert_eq!(json, "\"feature_envy\"");
    }

    /// Helper to build FileIR for feature envy tests.
    ///
    /// Each method entry is (method_name, is_method_flag, calls) where calls is
    /// Vec<(target, receiver, receiver_type)>.
    type FeatureEnvyMethod<'a> = (&'a str, bool, Vec<MethodCallTriple<'a>>);

    fn build_feature_envy_file_ir(
        class_name: &str,
        constructor_name: Option<&str>,
        methods: Vec<FeatureEnvyMethod<'_>>,
    ) -> FileIR {
        use crate::callgraph::cross_file_types::{CallSite, ClassDef};
        let mut file_ir = FileIR::new(PathBuf::from("test.py"));

        let mut method_names: Vec<String> = Vec::new();
        let mut line = 1u32;

        // Add constructor if specified
        if let Some(ctor) = constructor_name {
            method_names.push(ctor.to_string());
            file_ir.funcs.push(FuncDef::method(ctor, class_name, line, line + 2));
            line += 3;
        }

        // Add each method and its calls
        for (method_name, is_method, calls) in &methods {
            method_names.push(method_name.to_string());
            if *is_method {
                file_ir.funcs.push(FuncDef::method(*method_name, class_name, line, line + 5));
            } else {
                // Static/standalone function -- not a method (is_method=false, no class_name)
                file_ir.funcs.push(FuncDef::new(
                    method_name.to_string(),
                    line,
                    line + 5,
                    false,
                    None,
                    None,
                    None,
                ));
            }

            let qualified = format!("{}.{}", class_name, method_name);
            let call_sites: Vec<CallSite> = calls.iter().map(|(target, receiver, receiver_type)| {
                CallSite::method(
                    qualified.clone(),
                    *target,
                    *receiver,
                    if receiver_type.is_empty() { None } else { Some(receiver_type.to_string()) },
                    Some(line + 1),
                )
            }).collect();

            if !call_sites.is_empty() {
                file_ir.calls.insert(qualified, call_sites);
            }

            line += 6;
        }

        // Add ClassDef
        file_ir.classes.push(ClassDef::new(
            class_name.to_string(),
            1,
            line,
            method_names,
            vec![],
        ));

        file_ir
    }

    #[test]
    fn test_detect_feature_envy_field_access() {
        // 4 foreign accesses to Customer, 1 own access => should trigger
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("__init__"),
            vec![
                ("calculate_discount", true, vec![
                    ("loyalty_points", "customer", "Customer"),
                    ("discount_rate", "customer", "Customer"),
                    ("years_active", "customer", "Customer"),
                    ("bonus_multiplier", "customer", "Customer"),
                    ("amount", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "Method accessing other class 4 times vs own 1 time should trigger");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_no_feature_envy_own_fields() {
        // 0 foreign, 3 own => should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Account",
            Some("__init__"),
            vec![
                ("calculate_interest", true, vec![
                    ("balance", "self", ""),
                    ("rate", "self", ""),
                    ("fees", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Method using only own fields should not trigger");
    }

    #[test]
    fn test_feature_envy_method_calls() {
        // 4 foreign method calls to DataSource, 1 own => should trigger
        let file_ir = build_feature_envy_file_ir(
            "Report",
            None,
            vec![
                ("generate", true, vec![
                    ("get_title", "data", "DataSource"),
                    ("get_author", "data", "DataSource"),
                    ("get_content", "data", "DataSource"),
                    ("get_footer", "data", "DataSource"),
                    ("format_output", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "4 external vs 1 own method call should trigger");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_mixed_ratio() {
        // 2 foreign, 2 own => ratio 1:1, below 2.0 threshold => should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Processor",
            Some("__init__"),
            vec![
                ("process", true, vec![
                    ("value", "item", "Item"),
                    ("weight", "item", "Item"),
                    ("get_config", "self", ""),
                    ("transform", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "2 foreign vs 2 own (ratio 1:1) should not trigger at default threshold (2.0)");
    }

    #[test]
    fn test_feature_envy_static_excluded() {
        // Static function (is_method=false) with many foreign accesses => should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Util",
            None,
            vec![
                ("helper", false, vec![
                    ("x", "data", "Data"),
                    ("y", "data", "Data"),
                    ("z", "data", "Data"),
                    ("w", "data", "Data"),
                    ("v", "data", "Data"),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Static methods should be excluded from feature envy");
    }

    #[test]
    fn test_feature_envy_formatter_excluded() {
        // Class named "UserFormatter" with 5 foreign accesses => excluded by role-based filter (C4)
        let file_ir = build_feature_envy_file_ir(
            "UserFormatter",
            None,
            vec![
                ("format_user", true, vec![
                    ("get_name", "user", "User"),
                    ("get_email", "user", "User"),
                    ("get_age", "user", "User"),
                    ("get_address", "user", "User"),
                    ("get_phone", "user", "User"),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Classes with 'Formatter' in name should be excluded (role-based C4)");
    }

    #[test]
    fn test_feature_envy_at_threshold() {
        // Exactly at boundary: 4 foreign (= min_foreign default), 2 own (ratio 2.0 = threshold)
        let file_ir = build_feature_envy_file_ir(
            "Analyzer",
            None,
            vec![
                ("analyze", true, vec![
                    ("metric1", "stats", "Stats"),
                    ("metric2", "stats", "Stats"),
                    ("metric3", "stats", "Stats"),
                    ("metric4", "stats", "Stats"),
                    ("get_base", "self", ""),
                    ("get_factor", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        // 4 foreign, 2 own => ratio 2.0 >= 2.0 threshold, and foreign 4 >= min_foreign 4
        assert!(!findings.is_empty(), "Exactly at threshold (4 foreign, ratio 2.0) should trigger");
    }

    #[test]
    fn test_feature_envy_below_min_foreign() {
        // Only 2 foreign accesses (below min_foreign=4 default), high ratio => should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "SmallEnvy",
            None,
            vec![
                ("process", true, vec![
                    ("get_x", "other", "OtherClass"),
                    ("get_y", "other", "OtherClass"),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "2 foreign accesses (below min_foreign=4) should not trigger");
    }

    #[test]
    fn test_feature_envy_constructor_excluded() {
        // __init__ method with many foreign accesses => should NOT trigger (constructor excluded)
        let file_ir = build_feature_envy_file_ir(
            "Service",
            None,
            vec![
                ("__init__", true, vec![
                    ("get_config", "db", "Database"),
                    ("get_pool", "db", "Database"),
                    ("get_timeout", "db", "Database"),
                    ("get_retries", "db", "Database"),
                    ("get_host", "db", "Database"),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Constructor methods should be excluded from feature envy analysis");
    }

    #[test]
    fn test_feature_envy_detection_severity_levels() {
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);

        // Severity 1: mild envy (4 foreign, 1 own)
        let file_ir_mild = build_feature_envy_file_ir(
            "MildEnvy",
            None,
            vec![
                ("envious_method", true, vec![
                    ("a", "other", "Other"),
                    ("b", "other", "Other"),
                    ("c", "other", "Other"),
                    ("d", "other", "Other"),
                    ("own_method", "self", ""),
                ]),
            ],
        );
        let findings = detect_feature_envy_from_callgraph(&file_ir_mild, &thresholds, "python", false);
        assert!(!findings.is_empty(), "4 foreign, 1 own should trigger");
        assert_eq!(findings[0].severity, 1, "4 foreign, 1 own should be severity 1");

        // Severity 2: strong envy (6 foreign, 1 own => ratio 6.0, foreign >= 5, ratio > 2.5)
        let file_ir_strong = build_feature_envy_file_ir(
            "StrongEnvy",
            None,
            vec![
                ("very_envious", true, vec![
                    ("a", "other", "Other"),
                    ("b", "other", "Other"),
                    ("c", "other", "Other"),
                    ("d", "other", "Other"),
                    ("e", "other", "Other"),
                    ("f", "other", "Other"),
                    ("own_method", "self", ""),
                ]),
            ],
        );
        let findings = detect_feature_envy_from_callgraph(&file_ir_strong, &thresholds, "python", false);
        assert!(!findings.is_empty(), "6 foreign, 1 own should trigger");
        assert_eq!(findings[0].severity, 2, "6 foreign, 1 own should be severity 2");

        // Severity 3: extreme envy (10 foreign, 1 own => foreign >= 8, ratio > 4.0)
        let file_ir_extreme = build_feature_envy_file_ir(
            "ExtremeEnvy",
            None,
            vec![
                ("extremely_envious", true, vec![
                    ("a", "other", "Other"),
                    ("b", "other", "Other"),
                    ("c", "other", "Other"),
                    ("d", "other", "Other"),
                    ("e", "other", "Other"),
                    ("f", "other", "Other"),
                    ("g", "other", "Other"),
                    ("h", "other", "Other"),
                    ("i", "other", "Other"),
                    ("j", "other", "Other"),
                    ("own_method", "self", ""),
                ]),
            ],
        );
        let findings = detect_feature_envy_from_callgraph(&file_ir_extreme, &thresholds, "python", false);
        assert!(!findings.is_empty(), "10 foreign, 1 own should trigger");
        assert_eq!(findings[0].severity, 3, "10 foreign, 1 own should be severity 3");
    }

    #[test]
    fn test_feature_envy_with_suggestion() {
        let file_ir = build_feature_envy_file_ir(
            "Reporter",
            None,
            vec![
                ("generate", true, vec![
                    ("get_title", "data", "DataSource"),
                    ("get_author", "data", "DataSource"),
                    ("get_content", "data", "DataSource"),
                    ("get_footer", "data", "DataSource"),
                    ("format_output", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", true);
        assert!(!findings.is_empty());
        assert!(findings[0].suggestion.is_some(), "suggestion should be present when suggest=true");
        let suggestion = findings[0].suggestion.as_ref().unwrap();
        assert!(suggestion.contains("DataSource"), "Suggestion should mention the envied class");
    }

    #[test]
    fn test_feature_envy_zero_own_access() {
        // 5 foreign, 0 own => division by zero guard => should trigger
        let file_ir = build_feature_envy_file_ir(
            "PureEnvy",
            None,
            vec![
                ("all_foreign", true, vec![
                    ("a", "other", "Other"),
                    ("b", "other", "Other"),
                    ("c", "other", "Other"),
                    ("d", "other", "Other"),
                    ("e", "other", "Other"),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(!findings.is_empty(), "5 foreign, 0 own should trigger (division by zero handled)");
    }

    #[test]
    fn test_feature_envy_no_calls() {
        // Method with no calls at all => should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Silent",
            None,
            vec![
                ("no_calls_method", true, vec![]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Method with no calls should not trigger");
    }

    #[test]
    fn test_feature_envy_role_exclusions() {
        let excluded_names = [
            "DataSerializer", "JsonDeserializer", "RequestHandler",
            "AstVisitor", "HtmlRenderer", "UserValidator",
            "TypeConverter", "ObjectMapper", "FormBuilder",
        ];
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        for class_name in &excluded_names {
            let file_ir = build_feature_envy_file_ir(
                class_name,
                None,
                vec![
                    ("process", true, vec![
                        ("a", "other", "Other"),
                        ("b", "other", "Other"),
                        ("c", "other", "Other"),
                        ("d", "other", "Other"),
                        ("e", "other", "Other"),
                    ]),
                ],
            );
            let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
            assert!(findings.is_empty(), "Class '{}' should be excluded by role-based filter", class_name);
        }
    }

    #[test]
    fn test_feature_envy_typescript_this() {
        // TypeScript uses "this" instead of "self" for own-class references
        let file_ir = build_feature_envy_file_ir(
            "TsComponent",
            None,
            vec![
                ("render", true, vec![
                    ("getData", "service", "ApiService"),
                    ("getConfig", "service", "ApiService"),
                    ("getHeaders", "service", "ApiService"),
                    ("getTimeout", "service", "ApiService"),
                    ("setState", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "typescript", false);
        assert!(!findings.is_empty(), "4 foreign vs 1 own (this) should trigger in TypeScript");
    }

    // --- Inappropriate Intimacy tests ---

    #[test]
    fn test_inappropriate_intimacy_variant_exists() {
        let _ = SmellType::InappropriateIntimacy;
        assert_eq!(SmellType::InappropriateIntimacy.to_string(), "Inappropriate Intimacy");
    }

    #[test]
    fn test_inappropriate_intimacy_serialize() {
        let json = serde_json::to_string(&SmellType::InappropriateIntimacy).unwrap();
        assert_eq!(json, "\"inappropriate_intimacy\"");
    }

    // Helper: build a CallGraphIR + InheritanceReport for Inappropriate Intimacy tests.
    // class_a has `a_methods` and makes `a_to_b_calls` calls to class_b.
    // class_b has `b_methods` and makes `b_to_a_calls` calls to class_a.
    // If `same_file` is true, both classes are in the same file.
    // `inheritance_edges` can add parent-child relationships for exclusion testing.
    type IntimacyCall<'a> = (&'a str, &'a str);

    struct IntimacyClassSpec<'a> {
        name: &'a str,
        methods: &'a [&'a str],
        outbound_calls: &'a [IntimacyCall<'a>],
    }

    fn build_intimacy_test_data(
        class_a: IntimacyClassSpec<'_>,
        class_b: IntimacyClassSpec<'_>,
        same_file: bool,
        inheritance_edges: Vec<crate::types::inheritance::InheritanceEdge>,
    ) -> (CallGraphIR, InheritanceReport) {
        use crate::callgraph::cross_file_types::{ClassDef, FuncDef, CallSite, CallType};
        use crate::types::inheritance::InheritanceReport;

        let file_a_path = PathBuf::from("file_a.py");
        let file_b_path = if same_file {
            PathBuf::from("file_a.py")
        } else {
            PathBuf::from("file_b.py")
        };

        // Build FileIR for class A
        let mut file_a_ir = FileIR::new(file_a_path.clone());
        file_a_ir.classes.push(ClassDef::new(
            class_a.name.to_string(),
            1,
            (class_a.methods.len() as u32 * 3) + 1,
            class_a.methods.iter().map(|m| m.to_string()).collect(),
            vec![],
        ));
        for (i, method_name) in class_a.methods.iter().enumerate() {
            let line = (i as u32 * 3) + 2;
            file_a_ir.funcs.push(FuncDef::method(
                *method_name,
                class_a.name,
                line,
                line + 2,
            ));
        }
        // Add A->B calls
        for (from_method, target) in class_a.outbound_calls {
            let qualified = format!("{}.{}", class_a.name, from_method);
            file_a_ir.add_call(
                &qualified,
                CallSite::new(
                    qualified.clone(),
                    target.to_string(),
                    CallType::Method,
                    Some(2),
                    None,
                    Some("b".to_string()),
                    Some(class_b.name.to_string()),
                ),
            );
        }

        if same_file {
            // Add class B to the same FileIR
            let offset = (class_a.methods.len() as u32 * 3) + 2;
            file_a_ir.classes.push(ClassDef::new(
                class_b.name.to_string(),
                offset,
                offset + (class_b.methods.len() as u32 * 3),
                class_b.methods.iter().map(|m| m.to_string()).collect(),
                vec![],
            ));
            for (i, method_name) in class_b.methods.iter().enumerate() {
                let line = offset + (i as u32 * 3) + 1;
                file_a_ir.funcs.push(FuncDef::method(
                    *method_name,
                    class_b.name,
                    line,
                    line + 2,
                ));
            }
            // Add B->A calls
            for (from_method, target) in class_b.outbound_calls {
                let qualified = format!("{}.{}", class_b.name, from_method);
                file_a_ir.add_call(
                    &qualified,
                    CallSite::new(
                        qualified.clone(),
                        target.to_string(),
                        CallType::Method,
                        Some(2),
                        None,
                        Some("a".to_string()),
                        Some(class_a.name.to_string()),
                    ),
                );
            }

            let mut cg = CallGraphIR::new(PathBuf::from("."), "python");
            cg.files.insert(file_a_path, file_a_ir);

            let ir = InheritanceReport {
                edges: inheritance_edges,
                nodes: vec![],
                roots: vec![],
                leaves: vec![],
                count: 2,
                languages: vec![crate::types::Language::Python],
                diamonds: vec![],
                project_path: PathBuf::from("."),
                scan_time_ms: 0,
            };

            (cg, ir)
        } else {
            // Build separate FileIR for class B
            let mut file_b_ir = FileIR::new(file_b_path.clone());
            file_b_ir.classes.push(ClassDef::new(
                class_b.name.to_string(),
                1,
                (class_b.methods.len() as u32 * 3) + 1,
                class_b.methods.iter().map(|m| m.to_string()).collect(),
                vec![],
            ));
            for (i, method_name) in class_b.methods.iter().enumerate() {
                let line = (i as u32 * 3) + 2;
                file_b_ir.funcs.push(FuncDef::method(
                    *method_name,
                    class_b.name,
                    line,
                    line + 2,
                ));
            }
            // Add B->A calls
            for (from_method, target) in class_b.outbound_calls {
                let qualified = format!("{}.{}", class_b.name, from_method);
                file_b_ir.add_call(
                    &qualified,
                    CallSite::new(
                        qualified.clone(),
                        target.to_string(),
                        CallType::Method,
                        Some(2),
                        None,
                        Some("a".to_string()),
                        Some(class_a.name.to_string()),
                    ),
                );
            }

            let mut cg = CallGraphIR::new(PathBuf::from("."), "python");
            cg.files.insert(file_a_path, file_a_ir);
            cg.files.insert(file_b_path, file_b_ir);

            let ir = InheritanceReport {
                edges: inheritance_edges,
                nodes: vec![],
                roots: vec![],
                leaves: vec![],
                count: 2,
                languages: vec![crate::types::Language::Python],
                diamonds: vec![],
                project_path: PathBuf::from("."),
                scan_time_ms: 0,
            };

            (cg, ir)
        }
    }

    #[test]
    fn test_detect_inappropriate_intimacy() {
        // Bidirectional coupling: Order -> Item (6 calls), Item -> Order (6 calls)
        // Total = 12, per-direction min = 6, both >= 3 -> should trigger
        let a_to_b: Vec<(&str, &str)> = vec![
            ("add_item", "_get_price"),
            ("add_item", "_set_ref"),
            ("get_total", "_price"),
            ("get_total", "_weight"),
            ("process", "_validate"),
            ("process", "_update"),
        ];
        let b_to_a: Vec<(&str, &str)> = vec![
            ("update_order", "_items"),
            ("update_order", "_total"),
            ("collaborate", "_count"),
            ("collaborate", "_status"),
            ("sync", "_refresh"),
            ("sync", "_notify"),
        ];
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "Order",
                methods: &["add_item", "get_total", "process"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "Item",
                methods: &["update_order", "collaborate", "sync"],
                outbound_calls: &b_to_a,
            },
            true,
            vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(!findings.is_empty(), "Bidirectional coupling (6+6=12) should trigger at default threshold (10)");
        assert_eq!(findings[0].smell_type, SmellType::InappropriateIntimacy);
    }

    #[test]
    fn test_no_intimacy_one_direction() {
        // Customer -> Order (5 calls), Order -> Customer (0 calls)
        // Only one direction: should NOT trigger (not bidirectional)
        let a_to_b: Vec<(&str, &str)> = vec![
            ("get_total", "amount"),
            ("get_total", "tax"),
            ("get_total", "discount"),
            ("list_orders", "status"),
            ("list_orders", "date"),
        ];
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "Customer",
                methods: &["get_total", "list_orders"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "Order",
                methods: &["process"],
                outbound_calls: &[], // No calls back to Customer
            },
            true,
            vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "One-way access should not trigger (need bidirectional)");
    }

    #[test]
    fn test_intimacy_below_threshold() {
        // A -> B (1 call), B -> A (1 call)
        // Total = 2, per-direction = 1, below defaults (total=10, per_dir=3) -> no trigger
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "A",
                methods: &["method1"],
                outbound_calls: &[("method1", "_internal")],
            },
            IntimacyClassSpec {
                name: "B",
                methods: &["method1"],
                outbound_calls: &[("method1", "_state")],
            },
            true,
            vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Low-count bidirectional access (1+1=2) should not trigger");
    }

    #[test]
    fn test_intimacy_method_calls_count() {
        // ClassA -> ClassB (5 calls), ClassB -> ClassA (5 calls)
        // Total = 10, per-direction = 5, meets defaults -> should trigger
        let a_to_b: Vec<(&str, &str)> = vec![
            ("work", "_internal_method1"),
            ("work", "_internal_method2"),
            ("work", "_internal_method3"),
            ("process", "_internal_method4"),
            ("process", "_internal_method5"),
        ];
        let b_to_a: Vec<(&str, &str)> = vec![
            ("collaborate", "_helper1"),
            ("collaborate", "_helper2"),
            ("collaborate", "_helper3"),
            ("assist", "_helper4"),
            ("assist", "_helper5"),
        ];
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "ClassA",
                methods: &["work", "process"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "ClassB",
                methods: &["collaborate", "assist"],
                outbound_calls: &b_to_a,
            },
            true,
            vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(!findings.is_empty(), "Multiple bidirectional method calls (5+5=10) should trigger");
    }

    #[test]
    fn test_intimacy_parent_child_expected() {
        // Parent -> Child (5 calls), Child -> Parent (5 calls)
        // Total = 10, per-direction = 5 -> would trigger BUT inheritance edge excludes them
        use crate::types::inheritance::InheritanceEdge;

        let a_to_b: Vec<(&str, &str)> = vec![
            ("method1", "_child_a"),
            ("method1", "_child_b"),
            ("method2", "_child_c"),
            ("method2", "_child_d"),
            ("method3", "_child_e"),
        ];
        let b_to_a: Vec<(&str, &str)> = vec![
            ("use_parent1", "_protected_a"),
            ("use_parent1", "_protected_b"),
            ("use_parent2", "_protected_c"),
            ("use_parent2", "_protected_d"),
            ("use_parent3", "_protected_e"),
        ];
        let edge = InheritanceEdge::project(
            "Child", "Parent",
            PathBuf::from("file_a.py"), 1,
            PathBuf::from("file_a.py"), 1,
        );
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "Child",
                methods: &["use_parent1", "use_parent2", "use_parent3"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "Parent",
                methods: &["method1", "method2", "method3"],
                outbound_calls: &b_to_a,
            },
            true,
            vec![edge],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Parent-child access should be excluded by inheritance edge");
    }

    // --- New Inappropriate Intimacy tests (Phase 5) ---

    #[test]
    fn test_intimacy_high_total_low_per_dir() {
        // A -> B (9 calls), B -> A (1 call)
        // Total = 10, but min per-direction = 1 < 3 -> should NOT trigger
        let a_to_b: Vec<(&str, &str)> = vec![
            ("m1", "t1"), ("m1", "t2"), ("m1", "t3"),
            ("m2", "t4"), ("m2", "t5"), ("m2", "t6"),
            ("m3", "t7"), ("m3", "t8"), ("m3", "t9"),
        ];
        let b_to_a: Vec<(&str, &str)> = vec![
            ("m1", "s1"),
        ];
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "A",
                methods: &["m1", "m2", "m3"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "B",
                methods: &["m1"],
                outbound_calls: &b_to_a,
            },
            true, vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "High total (10) but low per-direction (1) should not trigger");
    }

    #[test]
    fn test_intimacy_severity_tiers() {
        // Use Strict thresholds: intimacy_min_total=6, intimacy_min_per_direction=2
        // Test severity 1: 3+3=6 total, min_dir=3
        // Severity from intimacy_severity(6, 3) -> 1 (below 12/3 threshold for sev 2)
        let a_to_b_1: Vec<(&str, &str)> = vec![
            ("m1", "t1"), ("m1", "t2"), ("m2", "t3"),
        ];
        let b_to_a_1: Vec<(&str, &str)> = vec![
            ("m1", "s1"), ("m1", "s2"), ("m2", "s3"),
        ];
        let (cg1, ir1) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "A1",
                methods: &["m1", "m2"],
                outbound_calls: &a_to_b_1,
            },
            IntimacyClassSpec {
                name: "B1",
                methods: &["m1", "m2"],
                outbound_calls: &b_to_a_1,
            },
            true, vec![],
        );
        let thresholds_strict = Thresholds::from_preset(ThresholdPreset::Strict);
        let findings1 = detect_inappropriate_intimacy_from_callgraph(&cg1, &ir1, &thresholds_strict, false);
        assert!(!findings1.is_empty(), "3+3=6 should trigger at strict threshold (6)");
        assert_eq!(findings1[0].severity, 1, "6 total, 3 min_dir -> severity 1");

        // Test severity 2: 7+7=14 total, min_dir=7
        // Severity from intimacy_severity(14, 7) -> 2
        let a_to_b_2: Vec<(&str, &str)> = vec![
            ("m1", "t1"), ("m1", "t2"), ("m1", "t3"), ("m1", "t4"),
            ("m2", "t5"), ("m2", "t6"), ("m2", "t7"),
        ];
        let b_to_a_2: Vec<(&str, &str)> = vec![
            ("m1", "s1"), ("m1", "s2"), ("m1", "s3"), ("m1", "s4"),
            ("m2", "s5"), ("m2", "s6"), ("m2", "s7"),
        ];
        let (cg2, ir2) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "A2",
                methods: &["m1", "m2"],
                outbound_calls: &a_to_b_2,
            },
            IntimacyClassSpec {
                name: "B2",
                methods: &["m1", "m2"],
                outbound_calls: &b_to_a_2,
            },
            true, vec![],
        );
        let findings2 = detect_inappropriate_intimacy_from_callgraph(&cg2, &ir2, &thresholds_strict, false);
        assert!(!findings2.is_empty(), "7+7=14 should trigger");
        assert_eq!(findings2[0].severity, 2, "14 total, 7 min_dir -> severity 2");

        // Test severity 3: 12+12=24 total, min_dir=12
        // Severity from intimacy_severity(24, 12) -> 3
        let mut a_to_b_3: Vec<(&str, &str)> = Vec::new();
        let mut b_to_a_3: Vec<(&str, &str)> = Vec::new();
        for _ in 0..12 {
            a_to_b_3.push(("m1", "t1"));
            b_to_a_3.push(("m1", "s1"));
        }
        let (cg3, ir3) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "A3",
                methods: &["m1"],
                outbound_calls: &a_to_b_3,
            },
            IntimacyClassSpec {
                name: "B3",
                methods: &["m1"],
                outbound_calls: &b_to_a_3,
            },
            true, vec![],
        );
        let findings3 = detect_inappropriate_intimacy_from_callgraph(&cg3, &ir3, &thresholds_strict, false);
        assert!(!findings3.is_empty(), "12+12=24 should trigger");
        assert_eq!(findings3[0].severity, 3, "24 total, 12 min_dir -> severity 3");
    }

    #[test]
    fn test_intimacy_cross_file() {
        // Classes in different files that reference each other -> should find intimacy
        let a_to_b: Vec<(&str, &str)> = vec![
            ("m1", "t1"), ("m1", "t2"), ("m1", "t3"),
            ("m2", "t4"), ("m2", "t5"),
        ];
        let b_to_a: Vec<(&str, &str)> = vec![
            ("m1", "s1"), ("m1", "s2"), ("m1", "s3"),
            ("m2", "s4"), ("m2", "s5"),
        ];
        let (cg, ir) = build_intimacy_test_data(
            IntimacyClassSpec {
                name: "ServiceA",
                methods: &["m1", "m2"],
                outbound_calls: &a_to_b,
            },
            IntimacyClassSpec {
                name: "ServiceB",
                methods: &["m1", "m2"],
                outbound_calls: &b_to_a,
            },
            false,  // Different files
            vec![],
        );

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(!findings.is_empty(), "Cross-file bidirectional coupling (5+5=10) should trigger");
        assert_eq!(findings[0].smell_type, SmellType::InappropriateIntimacy);
    }

    // =========================================================================
    // Phase 1 Infrastructure Tests
    // =========================================================================

    // --- Thresholds Tier-2 fields ---

    #[test]
    fn test_thresholds_tier2_strict() {
        let t = Thresholds::from_preset(ThresholdPreset::Strict);
        assert!((t.middle_man_delegation_ratio - 0.50).abs() < f64::EPSILON);
        assert_eq!(t.middle_man_min_methods, 3);
        assert!((t.refused_bequest_usage_ratio - 0.33).abs() < f64::EPSILON);
        assert_eq!(t.refused_bequest_min_inherited, 3);
        assert_eq!(t.feature_envy_min_foreign, 3);
        assert!((t.feature_envy_ratio - 1.5).abs() < f64::EPSILON);
        assert_eq!(t.intimacy_min_total, 6);
        assert_eq!(t.intimacy_min_per_direction, 2);
    }

    #[test]
    fn test_thresholds_tier2_default() {
        let t = Thresholds::from_preset(ThresholdPreset::Default);
        assert!((t.middle_man_delegation_ratio - 0.60).abs() < f64::EPSILON);
        assert_eq!(t.middle_man_min_methods, 3);
        assert!((t.refused_bequest_usage_ratio - 0.33).abs() < f64::EPSILON);
        assert_eq!(t.refused_bequest_min_inherited, 3);
        assert_eq!(t.feature_envy_min_foreign, 4);
        assert!((t.feature_envy_ratio - 2.0).abs() < f64::EPSILON);
        assert_eq!(t.intimacy_min_total, 10);
        assert_eq!(t.intimacy_min_per_direction, 3);
    }

    #[test]
    fn test_thresholds_tier2_relaxed() {
        let t = Thresholds::from_preset(ThresholdPreset::Relaxed);
        assert!((t.middle_man_delegation_ratio - 0.75).abs() < f64::EPSILON);
        assert_eq!(t.middle_man_min_methods, 3);
        assert!((t.refused_bequest_usage_ratio - 0.15).abs() < f64::EPSILON);
        assert_eq!(t.refused_bequest_min_inherited, 5);
        assert_eq!(t.feature_envy_min_foreign, 5);
        assert!((t.feature_envy_ratio - 3.0).abs() < f64::EPSILON);
        assert_eq!(t.intimacy_min_total, 15);
        assert_eq!(t.intimacy_min_per_direction, 4);
    }

    // --- get_class_methods_robust ---

    #[test]
    fn test_get_class_methods_robust_python() {
        // Python-like: ClassDef.methods populated
        use crate::callgraph::cross_file_types::{ClassDef, FuncDef, FileIR};

        let mut file_ir = FileIR::new(PathBuf::from("test.py"));
        file_ir.classes.push(ClassDef::new(
            "MyClass".into(),
            1,
            20,
            vec!["method_a".into(), "method_b".into()],
            vec![],
        ));
        file_ir.funcs.push(FuncDef::method("method_a", "MyClass", 2, 5));
        file_ir.funcs.push(FuncDef::method("method_b", "MyClass", 6, 10));

        let methods = get_class_methods_robust(&file_ir, "MyClass");
        assert_eq!(methods.len(), 2);
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"method_a"));
        assert!(names.contains(&"method_b"));
    }

    #[test]
    fn test_get_class_methods_robust_go_fallback() {
        // Go-like: ClassDef.methods is empty, methods are in FuncDef with class_name
        use crate::callgraph::cross_file_types::{ClassDef, FuncDef, FileIR};

        let mut file_ir = FileIR::new(PathBuf::from("server.go"));
        file_ir.classes.push(ClassDef::simple("Server", 1, 30));
        file_ir.funcs.push(FuncDef::method("Start", "Server", 5, 10));
        file_ir.funcs.push(FuncDef::method("Stop", "Server", 12, 18));
        file_ir.funcs.push(FuncDef::function("main", 20, 30));

        let methods = get_class_methods_robust(&file_ir, "Server");
        assert_eq!(methods.len(), 2, "Should find 2 methods via FuncDef.class_name fallback");
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"Start"));
        assert!(names.contains(&"Stop"));
    }

    // --- is_self_reference ---

    #[test]
    fn test_is_self_reference_python() {
        assert!(is_self_reference("self", "python"));
        assert!(!is_self_reference("this", "python"));
        assert!(!is_self_reference("me", "python"));
    }

    #[test]
    fn test_is_self_reference_typescript() {
        assert!(is_self_reference("this", "typescript"));
        assert!(!is_self_reference("self", "typescript"));
    }

    #[test]
    fn test_is_self_reference_rust() {
        assert!(is_self_reference("self", "rust"));
        assert!(!is_self_reference("this", "rust"));
    }

    #[test]
    fn test_is_self_reference_java() {
        assert!(is_self_reference("this", "java"));
        assert!(!is_self_reference("self", "java"));
    }

    #[test]
    fn test_is_self_reference_unknown_language() {
        // Unknown language accepts both
        assert!(is_self_reference("self", "unknown_lang"));
        assert!(is_self_reference("this", "unknown_lang"));
        assert!(!is_self_reference("me", "unknown_lang"));
    }

    // --- is_constructor ---

    #[test]
    fn test_is_constructor_python() {
        assert!(is_constructor("__init__", "python"));
        assert!(!is_constructor("process", "python"));
        assert!(!is_constructor("constructor", "python"));
    }

    #[test]
    fn test_is_constructor_javascript() {
        assert!(is_constructor("constructor", "javascript"));
        assert!(!is_constructor("__init__", "javascript"));
    }

    #[test]
    fn test_is_constructor_typescript() {
        assert!(is_constructor("constructor", "typescript"));
        assert!(!is_constructor("new", "typescript"));
    }

    #[test]
    fn test_is_constructor_rust() {
        assert!(is_constructor("new", "rust"));
        assert!(!is_constructor("__init__", "rust"));
    }

    #[test]
    fn test_is_constructor_go() {
        assert!(is_constructor("NewServer", "go"));
        assert!(is_constructor("NewClient", "go"));
        assert!(!is_constructor("processRequest", "go"));
    }

    #[test]
    fn test_is_constructor_excludes_regular() {
        // Regular method names should never be constructors
        for lang in &["python", "javascript", "typescript", "rust", "go", "java", "csharp"] {
            assert!(!is_constructor("process", lang), "process should not be constructor for {}", lang);
            assert!(!is_constructor("get_data", lang), "get_data should not be constructor for {}", lang);
        }
    }

    // --- Tier-2 severity functions ---

    #[test]
    fn test_middle_man_severity_levels() {
        // Severity 1: moderate delegation
        assert_eq!(middle_man_severity(0.60, 3), 1);
        // Severity 2: heavy delegation (ratio >= 0.75)
        assert_eq!(middle_man_severity(0.80, 3), 2);
        // Severity 2: heavy delegation (count >= 4)
        assert_eq!(middle_man_severity(0.65, 4), 2);
        // Severity 3: near-total delegation
        assert_eq!(middle_man_severity(0.95, 5), 3);
        assert_eq!(middle_man_severity(0.90, 6), 3);
    }

    #[test]
    fn test_refused_bequest_severity_levels() {
        // Severity 1: low usage
        assert_eq!(refused_bequest_severity(0.15, 4), 1);
        // Severity 2: very low usage (< 0.10)
        assert_eq!(refused_bequest_severity(0.05, 4), 2);
        // Severity 2: zero usage with small parent
        assert_eq!(refused_bequest_severity(0.0, 3), 2);
        // Severity 3: zero usage with large parent
        assert_eq!(refused_bequest_severity(0.0, 5), 3);
        assert_eq!(refused_bequest_severity(0.0, 10), 3);
    }

    #[test]
    fn test_feature_envy_severity_levels() {
        // Severity 1: mild envy
        assert_eq!(feature_envy_severity(4, 2), 1);
        // Severity 2: strong envy (foreign >= 5, ratio > 2.5)
        assert_eq!(feature_envy_severity(6, 1), 2);
        assert_eq!(feature_envy_severity(5, 1), 2);
        // Severity 3: extreme envy (foreign >= 8, ratio > 4.0)
        assert_eq!(feature_envy_severity(10, 1), 3);
        assert_eq!(feature_envy_severity(9, 2), 3); // 9>=8 and ratio 4.5>4.0 => sev 3
        // Verify boundary: foreign=8 but ratio too low stays at 2
        assert_eq!(feature_envy_severity(8, 3), 2); // ratio 2.67, foreign>=5 and ratio>2.5 => sev 2
    }

    #[test]
    fn test_intimacy_severity_levels() {
        // Severity 1: mild intimacy
        assert_eq!(intimacy_severity(8, 2), 1);
        // Severity 2: strong intimacy
        assert_eq!(intimacy_severity(14, 4), 2);
        assert_eq!(intimacy_severity(12, 3), 2);
        // Severity 3: extreme intimacy
        assert_eq!(intimacy_severity(20, 5), 3);
        assert_eq!(intimacy_severity(24, 8), 3);
    }

    // --- Stub functions compile and return empty ---

    #[test]
    fn test_stub_detect_middle_man_from_callgraph_compiles() {
        use crate::callgraph::cross_file_types::FileIR;
        let file_ir = FileIR::new(PathBuf::from("test.py"));
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Stub should return empty Vec");
    }

    #[test]
    fn test_stub_detect_refused_bequest_from_callgraph_compiles() {
        use crate::callgraph::cross_file_types::CallGraphIR;
        use crate::types::inheritance::InheritanceReport;
        let cg = CallGraphIR::new(PathBuf::from("."), "python");
        let ir = InheritanceReport::new(PathBuf::from("."));
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_refused_bequest_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Stub should return empty Vec");
    }

    #[test]
    fn test_stub_detect_feature_envy_from_callgraph_compiles() {
        use crate::callgraph::cross_file_types::FileIR;
        let file_ir = FileIR::new(PathBuf::from("test.py"));
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "python", false);
        assert!(findings.is_empty(), "Stub should return empty Vec");
    }

    #[test]
    fn test_stub_detect_inappropriate_intimacy_from_callgraph_compiles() {
        use crate::callgraph::cross_file_types::CallGraphIR;
        use crate::types::inheritance::InheritanceReport;
        let cg = CallGraphIR::new(PathBuf::from("."), "python");
        let ir = InheritanceReport::new(PathBuf::from("."));
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_inappropriate_intimacy_from_callgraph(&cg, &ir, &thresholds, false);
        assert!(findings.is_empty(), "Stub should return empty Vec");
    }

    #[test]
    fn test_tier2_threshold_ordering() {
        let strict = Thresholds::from_preset(ThresholdPreset::Strict);
        let default = Thresholds::from_preset(ThresholdPreset::Default);
        let relaxed = Thresholds::from_preset(ThresholdPreset::Relaxed);

        // Middle Man: lower ratio = stricter (triggers on less delegation)
        assert!(strict.middle_man_delegation_ratio <= default.middle_man_delegation_ratio,
            "Strict MM ratio ({}) should be <= Default ({})",
            strict.middle_man_delegation_ratio, default.middle_man_delegation_ratio);
        assert!(default.middle_man_delegation_ratio <= relaxed.middle_man_delegation_ratio,
            "Default MM ratio ({}) should be <= Relaxed ({})",
            default.middle_man_delegation_ratio, relaxed.middle_man_delegation_ratio);

        // Refused Bequest: HIGHER usage_ratio = stricter (triggers when more methods unused)
        assert!(strict.refused_bequest_usage_ratio >= relaxed.refused_bequest_usage_ratio,
            "Strict RB usage_ratio ({}) should be >= Relaxed ({})",
            strict.refused_bequest_usage_ratio, relaxed.refused_bequest_usage_ratio);

        // Refused Bequest: lower min_inherited can be stricter or same
        assert!(strict.refused_bequest_min_inherited <= default.refused_bequest_min_inherited,
            "Strict RB min_inherited ({}) should be <= Default ({})",
            strict.refused_bequest_min_inherited, default.refused_bequest_min_inherited);
        assert!(default.refused_bequest_min_inherited <= relaxed.refused_bequest_min_inherited,
            "Default RB min_inherited ({}) should be <= Relaxed ({})",
            default.refused_bequest_min_inherited, relaxed.refused_bequest_min_inherited);

        // Feature Envy: lower min_foreign = stricter (triggers on fewer foreign calls)
        assert!(strict.feature_envy_min_foreign <= default.feature_envy_min_foreign,
            "Strict FE min_foreign ({}) should be <= Default ({})",
            strict.feature_envy_min_foreign, default.feature_envy_min_foreign);
        assert!(default.feature_envy_min_foreign <= relaxed.feature_envy_min_foreign,
            "Default FE min_foreign ({}) should be <= Relaxed ({})",
            default.feature_envy_min_foreign, relaxed.feature_envy_min_foreign);

        // Feature Envy: lower ratio = stricter
        assert!(strict.feature_envy_ratio <= default.feature_envy_ratio,
            "Strict FE ratio ({}) should be <= Default ({})",
            strict.feature_envy_ratio, default.feature_envy_ratio);
        assert!(default.feature_envy_ratio <= relaxed.feature_envy_ratio,
            "Default FE ratio ({}) should be <= Relaxed ({})",
            default.feature_envy_ratio, relaxed.feature_envy_ratio);

        // Intimacy: lower min = stricter
        assert!(strict.intimacy_min_total <= default.intimacy_min_total,
            "Strict II min_total ({}) should be <= Default ({})",
            strict.intimacy_min_total, default.intimacy_min_total);
        assert!(default.intimacy_min_total <= relaxed.intimacy_min_total,
            "Default II min_total ({}) should be <= Relaxed ({})",
            default.intimacy_min_total, relaxed.intimacy_min_total);
        assert!(strict.intimacy_min_per_direction <= default.intimacy_min_per_direction,
            "Strict II min_per_direction ({}) should be <= Default ({})",
            strict.intimacy_min_per_direction, default.intimacy_min_per_direction);
        assert!(default.intimacy_min_per_direction <= relaxed.intimacy_min_per_direction,
            "Default II min_per_direction ({}) should be <= Relaxed ({})",
            default.intimacy_min_per_direction, relaxed.intimacy_min_per_direction);
    }

    // =========================================================================
    // E. Comprehensive is_constructor coverage across all 18 languages
    // =========================================================================

    #[test]
    fn test_is_constructor_all_languages() {
        // Python
        assert!(is_constructor("__init__", "python"));
        assert!(is_constructor("__init__", "py"));
        assert!(!is_constructor("process", "python"));

        // JavaScript / TypeScript / TSX / JSX
        assert!(is_constructor("constructor", "typescript"));
        assert!(is_constructor("constructor", "javascript"));
        assert!(is_constructor("constructor", "tsx"));
        assert!(is_constructor("constructor", "jsx"));
        assert!(is_constructor("constructor", "ts"));
        assert!(is_constructor("constructor", "js"));
        assert!(!is_constructor("process", "typescript"));
        assert!(!is_constructor("process", "javascript"));

        // Rust
        assert!(is_constructor("new", "rust"));
        assert!(is_constructor("new", "rs"));
        assert!(!is_constructor("process", "rust"));

        // Go
        assert!(is_constructor("NewService", "go"));
        assert!(is_constructor("NewOrderForwarder", "go"));
        assert!(is_constructor("New", "go"));
        assert!(!is_constructor("process", "go"));
        assert!(!is_constructor("newThing", "go")); // lowercase 'n' should not match

        // Ruby
        assert!(is_constructor("initialize", "ruby"));
        assert!(is_constructor("initialize", "rb"));
        assert!(!is_constructor("process", "ruby"));

        // PHP
        assert!(is_constructor("__construct", "php"));
        assert!(!is_constructor("process", "php"));

        // Swift
        assert!(is_constructor("init", "swift"));
        assert!(!is_constructor("process", "swift"));

        // Scala
        assert!(is_constructor("<init>", "scala"));
        assert!(is_constructor("this", "scala"));
        assert!(!is_constructor("process", "scala"));

        // Java/C#/Kotlin: constructor detection returns false (needs class name context)
        assert!(!is_constructor("MyClass", "java"));
        assert!(!is_constructor("MyClass", "csharp"));
        assert!(!is_constructor("MyClass", "cs"));
        assert!(!is_constructor("MyClass", "kotlin"));
        assert!(!is_constructor("MyClass", "kt"));

        // C/C++: constructor detection returns false (needs class name context)
        assert!(!is_constructor("MyClass", "c"));
        assert!(!is_constructor("MyClass", "cpp"));
        assert!(!is_constructor("MyClass", "c++"));

        // Elixir/Lua: no traditional constructors
        assert!(!is_constructor("new", "elixir"));
        assert!(!is_constructor("new", "ex"));
        assert!(!is_constructor("new", "lua"));
    }

    #[test]
    fn test_is_self_reference_all_languages() {
        // Python/Rust/Ruby/Swift use "self"
        assert!(is_self_reference("self", "python"));
        assert!(is_self_reference("self", "py"));
        assert!(is_self_reference("self", "rust"));
        assert!(is_self_reference("self", "rs"));
        assert!(is_self_reference("self", "ruby"));
        assert!(is_self_reference("self", "rb"));
        assert!(is_self_reference("self", "swift"));
        // "this" is NOT self-reference for these languages
        assert!(!is_self_reference("this", "python"));
        assert!(!is_self_reference("this", "rust"));
        assert!(!is_self_reference("this", "ruby"));
        assert!(!is_self_reference("this", "swift"));

        // TypeScript/JavaScript/TSX/JSX/Java/C#/Kotlin/Scala/C++/PHP use "this"
        assert!(is_self_reference("this", "typescript"));
        assert!(is_self_reference("this", "ts"));
        assert!(is_self_reference("this", "javascript"));
        assert!(is_self_reference("this", "js"));
        assert!(is_self_reference("this", "tsx"));
        assert!(is_self_reference("this", "jsx"));
        assert!(is_self_reference("this", "java"));
        assert!(is_self_reference("this", "csharp"));
        assert!(is_self_reference("this", "cs"));
        assert!(is_self_reference("this", "kotlin"));
        assert!(is_self_reference("this", "kt"));
        assert!(is_self_reference("this", "scala"));
        assert!(is_self_reference("this", "cpp"));
        assert!(is_self_reference("this", "c++"));
        assert!(is_self_reference("this", "php"));
        // "self" is NOT self-reference for these languages
        assert!(!is_self_reference("self", "typescript"));
        assert!(!is_self_reference("self", "java"));
        assert!(!is_self_reference("self", "cpp"));
        assert!(!is_self_reference("self", "php"));

        // Go: neither self nor this (receiver is a named variable)
        assert!(!is_self_reference("self", "go"));
        assert!(!is_self_reference("this", "go"));
        assert!(!is_self_reference("s", "go"));

        // C: no self reference
        assert!(!is_self_reference("self", "c"));
        assert!(!is_self_reference("this", "c"));

        // Elixir/Lua: no self reference
        assert!(!is_self_reference("self", "elixir"));
        assert!(!is_self_reference("this", "elixir"));
        assert!(!is_self_reference("self", "ex"));
        assert!(!is_self_reference("self", "lua"));
        assert!(!is_self_reference("this", "lua"));
    }

    // =========================================================================
    // A. Constructor exclusion tests for Middle Man (multi-language)
    // =========================================================================

    #[test]
    fn test_middle_man_constructor_excluded_typescript() {
        // TypeScript uses "constructor" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("constructor"),
            vec![
                ("getTotal", vec![("getTotal", "order", "Order")]),
                ("getItems", vec![("getItems", "order", "Order")]),
                ("getCustomer", vec![("getCustomer", "order", "Order")]),
                ("getStatus", vec![("getStatus", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "typescript", false);
        assert!(!findings.is_empty(), "TypeScript class with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_rust() {
        // Rust uses "new" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("new"),
            vec![
                ("get_total", vec![("get_total", "order", "Order")]),
                ("get_items", vec![("get_items", "order", "Order")]),
                ("get_customer", vec![("get_customer", "order", "Order")]),
                ("get_status", vec![("get_status", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "rust", false);
        assert!(!findings.is_empty(), "Rust struct with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_go() {
        // Go uses NewXxx — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("NewOrderForwarder"),
            vec![
                ("GetTotal", vec![("GetTotal", "order", "Order")]),
                ("GetItems", vec![("GetItems", "order", "Order")]),
                ("GetCustomer", vec![("GetCustomer", "order", "Order")]),
                ("GetStatus", vec![("GetStatus", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "go", false);
        assert!(!findings.is_empty(), "Go struct with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_ruby() {
        // Ruby uses "initialize" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("initialize"),
            vec![
                ("get_total", vec![("get_total", "order", "Order")]),
                ("get_items", vec![("get_items", "order", "Order")]),
                ("get_customer", vec![("get_customer", "order", "Order")]),
                ("get_status", vec![("get_status", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "ruby", false);
        assert!(!findings.is_empty(), "Ruby class with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_php() {
        // PHP uses "__construct" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("__construct"),
            vec![
                ("getTotal", vec![("getTotal", "order", "Order")]),
                ("getItems", vec![("getItems", "order", "Order")]),
                ("getCustomer", vec![("getCustomer", "order", "Order")]),
                ("getStatus", vec![("getStatus", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "php", false);
        assert!(!findings.is_empty(), "PHP class with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_swift() {
        // Swift uses "init" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("init"),
            vec![
                ("getTotal", vec![("getTotal", "order", "Order")]),
                ("getItems", vec![("getItems", "order", "Order")]),
                ("getCustomer", vec![("getCustomer", "order", "Order")]),
                ("getStatus", vec![("getStatus", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "swift", false);
        assert!(!findings.is_empty(), "Swift class with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_constructor_excluded_scala() {
        // Scala uses "<init>" — should be excluded from delegation count
        let file_ir = build_middle_man_file_ir(
            "OrderForwarder",
            Some("<init>"),
            vec![
                ("getTotal", vec![("getTotal", "order", "Order")]),
                ("getItems", vec![("getItems", "order", "Order")]),
                ("getCustomer", vec![("getCustomer", "order", "Order")]),
                ("getStatus", vec![("getStatus", "order", "Order")]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "scala", false);
        assert!(!findings.is_empty(), "Scala class with 4/4 delegating non-constructor methods should be middle man");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    // =========================================================================
    // B. Self-reference tests for Feature Envy (multi-language)
    // =========================================================================

    #[test]
    fn test_feature_envy_self_reference_typescript() {
        // TypeScript uses "this" as self-reference — own-class calls should use "this"
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("constructor"),
            vec![
                ("calculateDiscount", true, vec![
                    ("loyaltyPoints", "customer", "Customer"),
                    ("discountRate", "customer", "Customer"),
                    ("yearsActive", "customer", "Customer"),
                    ("bonusMultiplier", "customer", "Customer"),
                    ("amount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "typescript", false);
        assert!(!findings.is_empty(), "TypeScript method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_java() {
        // Java uses "this" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            None, // Java constructor needs class name context, so skip
            vec![
                ("calculateDiscount", true, vec![
                    ("getLoyaltyPoints", "customer", "Customer"),
                    ("getDiscountRate", "customer", "Customer"),
                    ("getYearsActive", "customer", "Customer"),
                    ("getBonusMultiplier", "customer", "Customer"),
                    ("getAmount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "java", false);
        assert!(!findings.is_empty(), "Java method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_ruby() {
        // Ruby uses "self" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("initialize"),
            vec![
                ("calculate_discount", true, vec![
                    ("loyalty_points", "customer", "Customer"),
                    ("discount_rate", "customer", "Customer"),
                    ("years_active", "customer", "Customer"),
                    ("bonus_multiplier", "customer", "Customer"),
                    ("amount", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "ruby", false);
        assert!(!findings.is_empty(), "Ruby method using 'self' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_swift() {
        // Swift uses "self" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("init"),
            vec![
                ("calculateDiscount", true, vec![
                    ("loyaltyPoints", "customer", "Customer"),
                    ("discountRate", "customer", "Customer"),
                    ("yearsActive", "customer", "Customer"),
                    ("bonusMultiplier", "customer", "Customer"),
                    ("amount", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "swift", false);
        assert!(!findings.is_empty(), "Swift method using 'self' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    // =========================================================================
    // C. Go/Rust struct method fallback tests (get_class_methods_robust)
    // =========================================================================

    #[test]
    fn test_middle_man_go_struct_methods() {
        // Go: ClassDef with EMPTY methods vec, but FuncDefs have class_name set.
        // get_class_methods_robust() should find them via the fallback path.
        use crate::callgraph::cross_file_types::{CallSite, ClassDef};
        let mut file_ir = FileIR::new(PathBuf::from("order_forwarder.go"));

        // Add FuncDefs with class_name (Go receiver methods)
        file_ir.funcs.push(FuncDef::method("GetTotal", "OrderForwarder", 1, 3));
        file_ir.funcs.push(FuncDef::method("GetItems", "OrderForwarder", 4, 6));
        file_ir.funcs.push(FuncDef::method("GetCustomer", "OrderForwarder", 7, 9));
        file_ir.funcs.push(FuncDef::method("GetStatus", "OrderForwarder", 10, 12));

        // Add delegation calls for each method
        for (method, target) in [("GetTotal", "GetTotal"), ("GetItems", "GetItems"),
                                  ("GetCustomer", "GetCustomer"), ("GetStatus", "GetStatus")] {
            let qualified = format!("OrderForwarder.{}", method);
            file_ir.calls.insert(qualified.clone(), vec![
                CallSite::method(qualified, target, "order", Some("Order".to_string()), Some(2)),
            ]);
        }

        // ClassDef with EMPTY methods vec — simulating Go struct (no methods list)
        file_ir.classes.push(ClassDef::new(
            "OrderForwarder".to_string(), 1, 12,
            vec![], // empty methods — triggers fallback path
            vec![],
        ));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "go", false);
        assert!(!findings.is_empty(),
            "Go struct with empty methods vec but FuncDefs with class_name should be detected via fallback");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_middle_man_rust_impl_methods() {
        // Rust: ClassDef with EMPTY methods vec, but FuncDefs have class_name set.
        // get_class_methods_robust() should find them via the fallback path.
        use crate::callgraph::cross_file_types::{CallSite, ClassDef};
        let mut file_ir = FileIR::new(PathBuf::from("order_forwarder.rs"));

        // Add "new" constructor and 4 delegating methods
        file_ir.funcs.push(FuncDef::method("new", "OrderForwarder", 1, 3));
        file_ir.funcs.push(FuncDef::method("get_total", "OrderForwarder", 4, 6));
        file_ir.funcs.push(FuncDef::method("get_items", "OrderForwarder", 7, 9));
        file_ir.funcs.push(FuncDef::method("get_customer", "OrderForwarder", 10, 12));
        file_ir.funcs.push(FuncDef::method("get_status", "OrderForwarder", 13, 15));

        // Add delegation calls
        for (method, target) in [("get_total", "get_total"), ("get_items", "get_items"),
                                  ("get_customer", "get_customer"), ("get_status", "get_status")] {
            let qualified = format!("OrderForwarder.{}", method);
            file_ir.calls.insert(qualified.clone(), vec![
                CallSite::method(qualified, target, "order", Some("Order".to_string()), Some(5)),
            ]);
        }

        // ClassDef with EMPTY methods vec — simulating Rust impl block
        file_ir.classes.push(ClassDef::new(
            "OrderForwarder".to_string(), 1, 15,
            vec![], // empty methods — triggers fallback path
            vec![],
        ));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "rust", false);
        assert!(!findings.is_empty(),
            "Rust struct with empty methods vec but FuncDefs with class_name should be detected via fallback");
        assert_eq!(findings[0].smell_type, SmellType::MiddleMan);
    }

    #[test]
    fn test_feature_envy_go_receiver() {
        // Go uses a named receiver variable (e.g., "i" for Invoice), not "self" or "this".
        // Foreign calls should be correctly identified since the receiver is neither self nor this.
        use crate::callgraph::cross_file_types::{CallSite, ClassDef};
        let mut file_ir = FileIR::new(PathBuf::from("invoice.go"));

        // Method with Go receiver pattern
        file_ir.funcs.push(FuncDef::method("CalculateDiscount", "Invoice", 1, 8));

        let qualified = "Invoice.CalculateDiscount".to_string();
        file_ir.calls.insert(qualified.clone(), vec![
            // Foreign calls to customer (receiver is "c" — a named Go variable)
            CallSite::method(qualified.clone(), "GetLoyaltyPoints", "c", Some("Customer".to_string()), Some(2)),
            CallSite::method(qualified.clone(), "GetDiscountRate", "c", Some("Customer".to_string()), Some(3)),
            CallSite::method(qualified.clone(), "GetYearsActive", "c", Some("Customer".to_string()), Some(4)),
            CallSite::method(qualified.clone(), "GetBonusMultiplier", "c", Some("Customer".to_string()), Some(5)),
            // Own-class call using named receiver "i" — Go has no "self"
            // Since Go's is_self_reference returns false for all receivers,
            // this would be classified as foreign unless receiver_type matches class name
            CallSite::method(qualified.clone(), "GetAmount", "i", Some("Invoice".to_string()), Some(6)),
        ]);

        file_ir.classes.push(ClassDef::new(
            "Invoice".to_string(), 1, 8,
            vec![], // Go uses empty methods (fallback path)
            vec![],
        ));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "go", false);
        assert!(!findings.is_empty(),
            "Go method with 4 foreign calls (Customer) and 1 own (Invoice via type match) should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    // =========================================================================
    // D. Non-OOP language tests (should return empty results)
    // =========================================================================

    #[test]
    fn test_middle_man_c_no_classes() {
        // C has no classes — FileIR with only free functions should produce no middle man findings
        let mut file_ir = FileIR::new(PathBuf::from("utils.c"));
        file_ir.funcs.push(FuncDef::function("process_data", 1, 10));
        file_ir.funcs.push(FuncDef::function("validate_input", 11, 20));
        file_ir.funcs.push(FuncDef::function("format_output", 21, 30));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "c", false);
        assert!(findings.is_empty(), "C file with no classes should produce no middle man findings");
    }

    #[test]
    fn test_feature_envy_lua_no_classes() {
        // Lua has no traditional classes — should produce no feature envy findings
        let mut file_ir = FileIR::new(PathBuf::from("utils.lua"));
        file_ir.funcs.push(FuncDef::function("process", 1, 10));
        file_ir.funcs.push(FuncDef::function("validate", 11, 20));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "lua", false);
        assert!(findings.is_empty(), "Lua file with no classes should produce no feature envy findings");
    }

    #[test]
    fn test_middle_man_elixir_no_classes() {
        // Elixir uses modules, not classes — should produce no middle man findings
        let mut file_ir = FileIR::new(PathBuf::from("order.ex"));
        file_ir.funcs.push(FuncDef::function("process_order", 1, 15));
        file_ir.funcs.push(FuncDef::function("validate_order", 16, 30));
        file_ir.funcs.push(FuncDef::function("format_receipt", 31, 45));

        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_middle_man_from_callgraph(&file_ir, &thresholds, "elixir", false);
        assert!(findings.is_empty(), "Elixir file with no classes should produce no middle man findings");
    }

    // =========================================================================
    // Additional: Feature Envy no-trigger with correct self-reference per language
    // =========================================================================

    #[test]
    fn test_no_feature_envy_own_fields_typescript() {
        // TypeScript: all calls use "this" (own-class) — should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Account",
            Some("constructor"),
            vec![
                ("calculateInterest", true, vec![
                    ("balance", "this", ""),
                    ("rate", "this", ""),
                    ("fees", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "typescript", false);
        assert!(findings.is_empty(), "TypeScript method using only 'this' (own) should not trigger feature envy");
    }

    #[test]
    fn test_no_feature_envy_own_fields_ruby() {
        // Ruby: all calls use "self" (own-class) — should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Account",
            Some("initialize"),
            vec![
                ("calculate_interest", true, vec![
                    ("balance", "self", ""),
                    ("rate", "self", ""),
                    ("fees", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "ruby", false);
        assert!(findings.is_empty(), "Ruby method using only 'self' (own) should not trigger feature envy");
    }

    #[test]
    fn test_no_feature_envy_own_fields_swift() {
        // Swift: all calls use "self" (own-class) — should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Account",
            Some("init"),
            vec![
                ("calculateInterest", true, vec![
                    ("balance", "self", ""),
                    ("rate", "self", ""),
                    ("fees", "self", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "swift", false);
        assert!(findings.is_empty(), "Swift method using only 'self' (own) should not trigger feature envy");
    }

    #[test]
    fn test_no_feature_envy_own_fields_cpp() {
        // C++: all calls use "this" (own-class) — should NOT trigger
        let file_ir = build_feature_envy_file_ir(
            "Account",
            None, // C++ constructor needs class name context
            vec![
                ("calculateInterest", true, vec![
                    ("balance", "this", ""),
                    ("rate", "this", ""),
                    ("fees", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "cpp", false);
        assert!(findings.is_empty(), "C++ method using only 'this' (own) should not trigger feature envy");
    }

    #[test]
    fn test_feature_envy_self_reference_php() {
        // PHP uses "this" (actual PHP uses $this, but IR normalizes to "this")
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("__construct"),
            vec![
                ("calculateDiscount", true, vec![
                    ("getLoyaltyPoints", "customer", "Customer"),
                    ("getDiscountRate", "customer", "Customer"),
                    ("getYearsActive", "customer", "Customer"),
                    ("getBonusMultiplier", "customer", "Customer"),
                    ("getAmount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "php", false);
        assert!(!findings.is_empty(), "PHP method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_scala() {
        // Scala uses "this" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            Some("<init>"),
            vec![
                ("calculateDiscount", true, vec![
                    ("loyaltyPoints", "customer", "Customer"),
                    ("discountRate", "customer", "Customer"),
                    ("yearsActive", "customer", "Customer"),
                    ("bonusMultiplier", "customer", "Customer"),
                    ("amount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "scala", false);
        assert!(!findings.is_empty(), "Scala method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_kotlin() {
        // Kotlin uses "this" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            None, // Kotlin constructor needs class name context
            vec![
                ("calculateDiscount", true, vec![
                    ("getLoyaltyPoints", "customer", "Customer"),
                    ("getDiscountRate", "customer", "Customer"),
                    ("getYearsActive", "customer", "Customer"),
                    ("getBonusMultiplier", "customer", "Customer"),
                    ("getAmount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "kotlin", false);
        assert!(!findings.is_empty(), "Kotlin method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }

    #[test]
    fn test_feature_envy_self_reference_csharp() {
        // C# uses "this" as self-reference
        let file_ir = build_feature_envy_file_ir(
            "Invoice",
            None, // C# constructor needs class name context
            vec![
                ("CalculateDiscount", true, vec![
                    ("GetLoyaltyPoints", "customer", "Customer"),
                    ("GetDiscountRate", "customer", "Customer"),
                    ("GetYearsActive", "customer", "Customer"),
                    ("GetBonusMultiplier", "customer", "Customer"),
                    ("GetAmount", "this", ""),
                ]),
            ],
        );
        let thresholds = Thresholds::from_preset(ThresholdPreset::Default);
        let findings = detect_feature_envy_from_callgraph(&file_ir, &thresholds, "csharp", false);
        assert!(!findings.is_empty(), "C# method using 'this' for own + 4 foreign should trigger feature envy");
        assert_eq!(findings[0].smell_type, SmellType::FeatureEnvy);
    }
}
