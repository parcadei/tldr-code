//! Martin Package Metrics Analyzer
//!
//! This module computes Robert Martin's package coupling metrics for
//! evaluating package design quality.
//!
//! # Metrics
//!
//! - **Ca (Afferent Coupling)**: Number of packages that depend on this package
//! - **Ce (Efferent Coupling)**: Number of packages this package depends on
//! - **I (Instability)**: Ce / (Ca + Ce) - 0=stable, 1=unstable
//! - **A (Abstractness)**: abstract_types / total_types - 0=concrete, 1=abstract
//! - **D (Distance)**: |A + I - 1| - distance from main sequence
//!
//! # Main Sequence Rule
//!
//! The "main sequence" is the line A + I = 1. Packages should ideally fall
//! on or near this line:
//!
//! - Stable packages (low I) should be abstract (high A)
//! - Unstable packages (high I) can be concrete (low A)
//!
//! # Zones
//!
//! - **Zone of Pain**: I < 0.3 AND A < 0.3 AND D > 0.5
//!   Stable but concrete - hard to change, painful to maintain
//!
//! - **Zone of Uselessness**: I > 0.7 AND A > 0.7
//!   Unstable and abstract - over-engineered, rarely used
//!
//! # T10 Mitigation: Divide-by-Zero
//!
//! When Ca + Ce = 0 (isolated package):
//! - Instability I = None (undefined)
//! - Distance D = None (undefined)
//! - Zone = None (cannot determine)
//! - Health = Isolated
//!
//! # References
//!
//! - Robert C. Martin, "Agile Software Development: Principles, Patterns, and Practices"
//! - Health spec section 4.4

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::walker::{walk_project, ProjectWalker};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::ast::extract::extract_file;
use crate::error::TldrError;
use crate::types::{Language, ModuleInfo};
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Health status of a package based on Martin metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricsHealth {
    /// Distance D <= 0.2 - on or near the main sequence
    Healthy,
    /// Distance 0.2 < D <= 0.4 - slight deviation from main sequence
    Warning,
    /// Distance D > 0.4 - significant deviation from main sequence
    Unhealthy,
    /// No dependencies (Ca = 0 and Ce = 0) - instability undefined
    Isolated,
}

/// Zone classification for packages with problematic metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    /// Healthy zone - near the main sequence
    Healthy,
    /// Warning zone - slight deviation
    WarningLow,
    /// Zone of Pain: I < 0.3, A < 0.3, D > 0.5
    /// Stable and concrete - rigid, hard to change
    PainZone,
    /// Zone of Uselessness: I > 0.7, A > 0.7
    /// Unstable and abstract - over-engineered
    UselessnessZone,
}

/// Metrics for a single package (module/directory)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetrics {
    /// Package name (directory name or module path)
    pub name: String,
    /// Package path
    pub path: PathBuf,
    /// Afferent coupling: packages that depend on this one
    pub ca: usize,
    /// Efferent coupling: packages this one depends on
    pub ce: usize,
    /// Instability: Ce / (Ca + Ce), None if isolated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instability: Option<f64>,
    /// Abstractness: abstract_types / total_types
    pub abstractness: f64,
    /// Distance from main sequence: |A + I - 1|, None if isolated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    /// Total number of types (classes, interfaces)
    pub total_types: usize,
    /// Number of abstract types (ABC, Protocol, interface)
    pub abstract_types: usize,
    /// Packages that depend on this one
    pub incoming_packages: Vec<String>,
    /// Packages this one depends on
    pub outgoing_packages: Vec<String>,
    /// Health status based on distance
    pub health: MetricsHealth,
    /// Zone classification (if in a problematic zone)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone: Option<Zone>,
}

/// Summary statistics for Martin metrics analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSummary {
    /// Total number of packages analyzed
    pub total_packages: usize,
    /// Average instability across packages (None if all isolated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_instability: Option<f64>,
    /// Average abstractness across packages
    pub avg_abstractness: f64,
    /// Average distance from main sequence (None if all isolated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_distance: Option<f64>,
    /// Number of healthy packages
    pub healthy_count: usize,
    /// Number of warning packages
    pub warning_count: usize,
    /// Number of unhealthy packages
    pub unhealthy_count: usize,
    /// Number of isolated packages
    pub isolated_count: usize,
}

impl Default for MetricsSummary {
    fn default() -> Self {
        Self {
            total_packages: 0,
            avg_instability: None,
            avg_abstractness: 0.0,
            avg_distance: None,
            healthy_count: 0,
            warning_count: 0,
            unhealthy_count: 0,
            isolated_count: 0,
        }
    }
}

/// Problem zones identified in the analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricsProblems {
    /// Packages in the zone of pain (stable, concrete, hard to change)
    pub zone_of_pain: Vec<String>,
    /// Packages in the zone of uselessness (unstable, abstract, over-engineered)
    pub zone_of_uselessness: Vec<String>,
}

/// Complete Martin metrics report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MartinReport {
    /// Number of packages analyzed
    pub packages_analyzed: usize,
    /// Average instability (None if all isolated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_instability: Option<f64>,
    /// Average abstractness
    pub avg_abstractness: f64,
    /// Average distance from main sequence (None if all isolated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_distance: Option<f64>,
    /// Number of packages in zone of pain
    pub packages_in_pain_zone: usize,
    /// Number of packages in zone of uselessness
    pub packages_in_uselessness_zone: usize,
    /// All packages with metrics
    pub packages: Vec<PackageMetrics>,
    /// Summary statistics
    pub summary: MetricsSummary,
    /// Identified problems
    pub problems: MetricsProblems,
}

impl Default for MartinReport {
    fn default() -> Self {
        Self {
            packages_analyzed: 0,
            avg_instability: None,
            avg_abstractness: 0.0,
            avg_distance: None,
            packages_in_pain_zone: 0,
            packages_in_uselessness_zone: 0,
            packages: Vec::new(),
            summary: MetricsSummary::default(),
            problems: MetricsProblems::default(),
        }
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Compute Robert Martin's package coupling metrics
///
/// Analyzes package-level dependencies and computes:
/// - Ca (afferent coupling)
/// - Ce (efferent coupling)
/// - I (instability)
/// - A (abstractness)
/// - D (distance from main sequence)
///
/// # Arguments
/// * `path` - Directory to analyze
/// * `language` - Optional language filter (auto-detect if None)
///
/// # Returns
/// * `Ok(MartinReport)` - Report with package metrics
/// * `Err(TldrError)` - On file system errors
///
/// # T10 Mitigation
///
/// When Ca + Ce = 0 (isolated package):
/// - I = None (not 0/0 which would NaN/panic)
/// - D = None (cannot compute without I)
/// - Zone = None
/// - Health = Isolated
///
/// # Example
/// ```ignore
/// use tldr_core::quality::martin::compute_martin_metrics;
/// use std::path::Path;
///
/// let report = compute_martin_metrics(Path::new("src/"), None)?;
/// for pkg in &report.packages {
///     println!("{}: D={:?}, health={:?}", pkg.name, pkg.distance, pkg.health);
/// }
/// ```
pub fn compute_martin_metrics(path: &Path, language: Option<Language>) -> TldrResult<MartinReport> {
    // Detect language if not specified
    let lang = match language {
        Some(l) => l,
        None => detect_language(path)?,
    };

    // Collect package information
    let packages_info = collect_packages(path, lang)?;

    if packages_info.is_empty() {
        return Ok(MartinReport::default());
    }

    // Build dependency graph: package -> set of packages it depends on
    let (dependencies, reverse_deps) = build_dependency_graph(&packages_info);

    // Compute metrics for each package
    let mut packages: Vec<PackageMetrics> = Vec::new();
    let mut problems = MetricsProblems::default();

    for (pkg_name, pkg_info) in &packages_info {
        // Get coupling counts
        let outgoing: HashSet<&String> = dependencies
            .get(pkg_name)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        let incoming: HashSet<&String> = reverse_deps
            .get(pkg_name)
            .map(|s| s.iter().collect())
            .unwrap_or_default();

        let ca = incoming.len();
        let ce = outgoing.len();

        // Compute instability with T10 guard
        let instability = compute_instability(ca, ce);

        // Compute abstractness
        let abstractness = if pkg_info.total_types > 0 {
            pkg_info.abstract_types as f64 / pkg_info.total_types as f64
        } else {
            0.0
        };

        // Compute distance with T10 guard
        let distance = instability.map(|i| (abstractness + i - 1.0).abs());

        // Determine health
        let health = match (instability, distance) {
            (None, None) => MetricsHealth::Isolated,
            (_, Some(d)) if d <= 0.2 => MetricsHealth::Healthy,
            (_, Some(d)) if d <= 0.4 => MetricsHealth::Warning,
            _ => MetricsHealth::Unhealthy,
        };

        // Detect zones
        let zone = detect_zone(instability, abstractness, distance);

        // Track problems
        if zone == Some(Zone::PainZone) {
            problems.zone_of_pain.push(pkg_name.clone());
        } else if zone == Some(Zone::UselessnessZone) {
            problems.zone_of_uselessness.push(pkg_name.clone());
        }

        packages.push(PackageMetrics {
            name: pkg_name.clone(),
            path: pkg_info.path.clone(),
            ca,
            ce,
            instability,
            abstractness,
            distance,
            total_types: pkg_info.total_types,
            abstract_types: pkg_info.abstract_types,
            incoming_packages: incoming.iter().map(|s| (*s).clone()).collect(),
            outgoing_packages: outgoing.iter().map(|s| (*s).clone()).collect(),
            health,
            zone,
        });
    }

    // Sort packages by distance (problems first), then by name
    packages.sort_by(|a, b| {
        let da = a.distance.unwrap_or(f64::MAX);
        let db = b.distance.unwrap_or(f64::MAX);
        db.partial_cmp(&da)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });

    // Compute summary statistics
    let total_packages = packages.len();

    let (instability_sum, instability_count): (f64, usize) = packages
        .iter()
        .filter_map(|p| p.instability)
        .fold((0.0, 0), |(sum, count), i| (sum + i, count + 1));

    let avg_instability = if instability_count > 0 {
        Some(instability_sum / instability_count as f64)
    } else {
        None
    };

    let avg_abstractness = if total_packages > 0 {
        packages.iter().map(|p| p.abstractness).sum::<f64>() / total_packages as f64
    } else {
        0.0
    };

    let (distance_sum, distance_count): (f64, usize) = packages
        .iter()
        .filter_map(|p| p.distance)
        .fold((0.0, 0), |(sum, count), d| (sum + d, count + 1));

    let avg_distance = if distance_count > 0 {
        Some(distance_sum / distance_count as f64)
    } else {
        None
    };

    let healthy_count = packages
        .iter()
        .filter(|p| p.health == MetricsHealth::Healthy)
        .count();
    let warning_count = packages
        .iter()
        .filter(|p| p.health == MetricsHealth::Warning)
        .count();
    let unhealthy_count = packages
        .iter()
        .filter(|p| p.health == MetricsHealth::Unhealthy)
        .count();
    let isolated_count = packages
        .iter()
        .filter(|p| p.health == MetricsHealth::Isolated)
        .count();

    let packages_in_pain_zone = problems.zone_of_pain.len();
    let packages_in_uselessness_zone = problems.zone_of_uselessness.len();

    Ok(MartinReport {
        packages_analyzed: total_packages,
        avg_instability,
        avg_abstractness,
        avg_distance,
        packages_in_pain_zone,
        packages_in_uselessness_zone,
        packages,
        summary: MetricsSummary {
            total_packages,
            avg_instability,
            avg_abstractness,
            avg_distance,
            healthy_count,
            warning_count,
            unhealthy_count,
            isolated_count,
        },
        problems,
    })
}

// =============================================================================
// Helper Types and Functions
// =============================================================================

/// Information about a package for metrics calculation
struct PackageInfo {
    path: PathBuf,
    total_types: usize,
    abstract_types: usize,
    imports: HashSet<String>,
}

/// Compute instability with divide-by-zero guard (T10)
///
/// I = Ce / (Ca + Ce)
///
/// Returns None if Ca + Ce = 0 (isolated package).
fn compute_instability(ca: usize, ce: usize) -> Option<f64> {
    let total = ca + ce;
    if total == 0 {
        None // T10: Isolated package - instability undefined
    } else {
        Some(ce as f64 / total as f64)
    }
}

/// Detect which zone a package falls into
///
/// - Zone of Pain: I < 0.3 AND A < 0.3 AND D > 0.5
/// - Zone of Uselessness: I > 0.7 AND A > 0.7
fn detect_zone(instability: Option<f64>, abstractness: f64, distance: Option<f64>) -> Option<Zone> {
    let i = instability?;
    let d = distance?;

    // Zone of Pain: stable (low I), concrete (low A), far from main sequence
    if i < 0.3 && abstractness < 0.3 && d > 0.5 {
        return Some(Zone::PainZone);
    }

    // Zone of Uselessness: unstable (high I), abstract (high A)
    if i > 0.7 && abstractness > 0.7 {
        return Some(Zone::UselessnessZone);
    }

    // Healthy or warning based on distance
    if d <= 0.2 {
        Some(Zone::Healthy)
    } else if d <= 0.5 {
        Some(Zone::WarningLow)
    } else {
        None
    }
}

/// Collect package information from the codebase
fn collect_packages(path: &Path, language: Language) -> TldrResult<IndexMap<String, PackageInfo>> {
    let mut packages: IndexMap<String, PackageInfo> = IndexMap::new();

    let extensions: &[&str] = language.extensions();

    // Walk the directory tree
    for entry in walk_project(path) {
        let file_path = entry.path();
        if !file_path.is_file() {
            continue;
        }

        // Check if file has supported extension
        if !matches!(
            file_path.extension().and_then(|e| e.to_str()),
            Some(ext) if extensions.contains(&ext)
        ) {
            continue;
        }

        // Get package name (parent directory relative to root)
        let pkg_name = get_package_name(file_path, path);

        // Extract module info
        let info = match extract_file(file_path, Some(path)) {
            Ok(info) => info,
            Err(_) => continue, // Skip files that fail to parse
        };

        // Update or create package info
        let pkg_info = packages
            .entry(pkg_name.clone())
            .or_insert_with(|| PackageInfo {
                path: file_path.parent().unwrap_or(path).to_path_buf(),
                total_types: 0,
                abstract_types: 0,
                imports: HashSet::new(),
            });

        // Count types
        pkg_info.total_types += info.classes.len();

        // Count abstract types based on language
        for class in &info.classes {
            if is_abstract_type(&class.name, &info, language) {
                pkg_info.abstract_types += 1;
            }
        }

        // Collect imports
        for import in &info.imports {
            // Normalize import to package level
            if let Some(pkg) = normalize_import(&import.module, language) {
                if pkg != pkg_name {
                    pkg_info.imports.insert(pkg);
                }
            }
        }
    }

    Ok(packages)
}

/// Get package name from file path relative to root
fn get_package_name(file_path: &Path, root: &Path) -> String {
    // Get parent directory relative to root
    let parent = file_path.parent().unwrap_or(file_path);
    let relative = parent.strip_prefix(root).unwrap_or(parent);

    if relative.as_os_str().is_empty() {
        // File is directly in root
        "<root>".to_string()
    } else {
        relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, ".")
    }
}

/// Normalize an import path to a package name
fn normalize_import(module_path: &str, language: Language) -> Option<String> {
    match language {
        Language::Python => {
            // Python: "package.subpackage.module" -> "package.subpackage"
            // But only if it's a multi-part path
            let parts: Vec<&str> = module_path.split('.').collect();
            if parts.len() > 1 {
                // Return all but the last component (the module itself)
                Some(parts[..parts.len() - 1].join("."))
            } else {
                // Single-part import is likely a standard library or installed package
                Some(module_path.to_string())
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // TS/JS: "@scope/package/path" -> "@scope/package"
            // or "package/path" -> "package"
            if module_path.starts_with('@') {
                // Scoped package
                let parts: Vec<&str> = module_path.splitn(3, '/').collect();
                if parts.len() >= 2 {
                    Some(format!("{}/{}", parts[0], parts[1]))
                } else {
                    Some(module_path.to_string())
                }
            } else if module_path.starts_with('.') {
                // Relative import - extract package from path
                None // Skip relative imports for now
            } else {
                // Regular package
                let parts: Vec<&str> = module_path.splitn(2, '/').collect();
                Some(parts[0].to_string())
            }
        }
        Language::Go => {
            // Go: "github.com/user/repo/package" -> "github.com/user/repo"
            let parts: Vec<&str> = module_path.splitn(4, '/').collect();
            if parts.len() >= 3 {
                Some(format!("{}/{}/{}", parts[0], parts[1], parts[2]))
            } else {
                Some(module_path.to_string())
            }
        }
        Language::Rust => {
            // Rust: "crate::module::submodule" -> "crate::module"
            let parts: Vec<&str> = module_path.split("::").collect();
            if parts.len() > 1 {
                Some(parts[..parts.len() - 1].join("::"))
            } else {
                Some(module_path.to_string())
            }
        }
        _ => Some(module_path.to_string()),
    }
}

/// Check if a class is abstract based on language conventions
fn is_abstract_type(class_name: &str, info: &ModuleInfo, language: Language) -> bool {
    match language {
        Language::Python => {
            // Python: class inherits from ABC or has @abstractmethod
            // Check if class name ends with "ABC" or "Base" or "Interface"
            // or if any method has "abstractmethod" in decorators
            if class_name.ends_with("ABC")
                || class_name.ends_with("Base")
                || class_name.ends_with("Interface")
                || class_name.ends_with("Protocol")
            {
                return true;
            }

            // Check for ABC in imports
            for import in &info.imports {
                if import.module == "abc" || import.module.ends_with(".abc") {
                    // Likely uses ABC
                    return true;
                }
            }

            false
        }
        Language::TypeScript | Language::JavaScript => {
            // TypeScript: interface or abstract class
            class_name.starts_with('I')
                && class_name
                    .chars()
                    .nth(1)
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                || class_name.ends_with("Interface")
        }
        Language::Go => {
            // Go: interfaces typically end with "er" or "Interface"
            class_name.ends_with("er") || class_name.ends_with("Interface")
        }
        Language::Rust => {
            // Rust: traits are abstract
            // Since we're looking at classes, check for common trait naming
            class_name.ends_with("Trait") || class_name.starts_with("dyn ")
        }
        _ => false,
    }
}

/// Build dependency graph from package info
fn build_dependency_graph(
    packages: &IndexMap<String, PackageInfo>,
) -> (
    HashMap<String, HashSet<String>>,
    HashMap<String, HashSet<String>>,
) {
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut reverse_deps: HashMap<String, HashSet<String>> = HashMap::new();

    let package_names: HashSet<&String> = packages.keys().collect();

    for (pkg_name, pkg_info) in packages {
        for import in &pkg_info.imports {
            // Only count internal dependencies (within the analyzed codebase)
            // Check if any package name starts with or equals the import
            let is_internal = package_names.iter().any(|&p| {
                p == import
                    || p.starts_with(&format!("{}.", import))
                    || import.starts_with(&format!("{}.", p))
            });

            if is_internal || package_names.contains(import) {
                // This package depends on the import
                dependencies
                    .entry(pkg_name.clone())
                    .or_default()
                    .insert(import.clone());

                // The import is depended upon by this package
                reverse_deps
                    .entry(import.clone())
                    .or_default()
                    .insert(pkg_name.clone());
            }
        }
    }

    (dependencies, reverse_deps)
}

/// Detect language from path
fn detect_language(path: &Path) -> TldrResult<Language> {
    if path.is_file() {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            return Language::from_extension(ext)
                .ok_or_else(|| TldrError::UnsupportedLanguage(ext.to_string()));
        }
    }

    // For directories, scan for common file types
    let mut counts: HashMap<Language, usize> = HashMap::new();

    for entry in ProjectWalker::new(path).max_depth(3).iter() {
        if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                if let Some(lang) = Language::from_extension(ext) {
                    *counts.entry(lang).or_default() += 1;
                }
            }
        }
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang)
        .ok_or_else(|| TldrError::NoSupportedFiles(path.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_instability() {
        // Normal case
        assert_eq!(compute_instability(0, 10), Some(1.0)); // All outgoing
        assert_eq!(compute_instability(10, 0), Some(0.0)); // All incoming
        assert_eq!(compute_instability(5, 5), Some(0.5)); // Balanced

        // T10: Isolated package
        assert_eq!(compute_instability(0, 0), None);
    }

    #[test]
    fn test_detect_zone() {
        // Zone of Pain: low I, low A, high D
        let zone = detect_zone(Some(0.1), 0.1, Some(0.8));
        assert_eq!(zone, Some(Zone::PainZone));

        // Zone of Uselessness: high I, high A
        let zone = detect_zone(Some(0.8), 0.8, Some(0.6));
        assert_eq!(zone, Some(Zone::UselessnessZone));

        // Healthy: near main sequence
        let zone = detect_zone(Some(0.5), 0.5, Some(0.0));
        assert_eq!(zone, Some(Zone::Healthy));

        // Isolated: no instability
        let zone = detect_zone(None, 0.5, None);
        assert_eq!(zone, None);
    }

    #[test]
    fn test_metrics_health_from_distance() {
        // Test health classification
        assert_eq!(
            if 0.1_f64 <= 0.2 {
                MetricsHealth::Healthy
            } else {
                MetricsHealth::Warning
            },
            MetricsHealth::Healthy
        );
        assert_eq!(
            if 0.3_f64 <= 0.2 {
                MetricsHealth::Healthy
            } else if 0.3_f64 <= 0.4 {
                MetricsHealth::Warning
            } else {
                MetricsHealth::Unhealthy
            },
            MetricsHealth::Warning
        );
        assert_eq!(MetricsHealth::Unhealthy, MetricsHealth::Unhealthy);
    }

    #[test]
    fn test_martin_report_default() {
        let report = MartinReport::default();
        assert_eq!(report.packages_analyzed, 0);
        assert!(report.avg_instability.is_none());
        assert_eq!(report.avg_abstractness, 0.0);
        assert!(report.avg_distance.is_none());
    }

    #[test]
    fn test_get_package_name() {
        let root = Path::new("/project");

        // File in root
        assert_eq!(
            get_package_name(Path::new("/project/main.py"), root),
            "<root>"
        );

        // File in subdirectory
        assert_eq!(
            get_package_name(Path::new("/project/src/utils.py"), root),
            "src"
        );

        // Nested directory
        assert_eq!(
            get_package_name(Path::new("/project/src/core/module.py"), root),
            "src.core"
        );
    }
}
