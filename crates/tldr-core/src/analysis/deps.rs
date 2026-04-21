//! Dependency Analysis Core Types and Functions
//!
//! This module provides dependency analysis for the `deps` CLI command.
//!
//! # Type Overview
//!
//! - [`DepsReport`]: Complete dependency analysis report
//! - [`DepNode`]: A node in the dependency graph (file or package)
//! - [`DepEdge`]: An edge in the dependency graph
//! - [`DepCycle`]: A circular dependency cycle
//! - [`DepKind`]: Classification of a dependency (Internal, Stdlib, External)
//! - [`DepStats`]: Analysis statistics
//! - [`DepsOptions`]: Configuration for dependency analysis
//!
//! # Functions
//!
//! - [`analyze_dependencies`]: Build dependency graph for a directory
//!
//! # Risk Mitigations
//!
//! - S7-R3: Handle relative imports with current file context
//! - S7-R8: Use HashMap index for O(1) resolution (not O(n^2))
//! - S7-R14: Canonicalize paths before indexing, uses `PathBuf` for path handling
//! - S7-R15: `DepNode` derives `Hash` and `Eq` based on path only
//! - S7-R40: Uses `BTreeMap` for deterministic JSON output
//!
//! # References
//!
//! - Spec: session7-spec.md section 1.2 (Type Definitions)
//! - Phased plan: session7-phased-plan.yaml Phase 1, 2

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::ast::imports::get_imports;
use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{IgnoreSpec, ImportInfo, Language};
use std::str::FromStr as _;
use crate::TldrResult;

// =============================================================================
// Core Types
// =============================================================================

/// Complete dependency analysis report
///
/// Contains all information about a project's dependency structure including
/// internal dependencies, external dependencies, and circular dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepsReport {
    /// Root path analyzed (relative paths in report are relative to this)
    pub root: PathBuf,

    /// Language detected/specified
    pub language: String,

    /// Internal dependencies (file -> [imported files])
    /// Uses BTreeMap for deterministic JSON output (S7-R40)
    pub internal_dependencies: BTreeMap<PathBuf, Vec<PathBuf>>,

    /// External dependencies (file -> [package names])
    /// Uses BTreeMap for deterministic JSON output (S7-R40)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub external_dependencies: BTreeMap<PathBuf, Vec<String>>,

    /// Circular dependencies found
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub circular_dependencies: Vec<DepCycle>,

    /// Analysis statistics
    pub stats: DepStats,
}

impl Default for DepsReport {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            language: String::new(),
            internal_dependencies: BTreeMap::new(),
            external_dependencies: BTreeMap::new(),
            circular_dependencies: Vec::new(),
            stats: DepStats::default(),
        }
    }
}

/// A node in the dependency graph
///
/// Represents a file or package in the dependency graph.
/// Hash and Eq are implemented based on path only (S7-R15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepNode {
    /// Canonical file path (normalized, relative to project root)
    pub path: PathBuf,

    /// Module name (derived from path, used for display)
    pub name: String,

    /// Whether this is an internal (project) or external (stdlib/third-party) module
    pub kind: DepKind,
}

impl DepNode {
    /// Create a new DepNode with the given path and derive name from it
    pub fn new(path: PathBuf, kind: DepKind) -> Self {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        Self { path, name, kind }
    }

    /// Create a new DepNode with explicit path and name
    pub fn with_name(path: PathBuf, name: String, kind: DepKind) -> Self {
        Self { path, name, kind }
    }
}

// Implement Hash and Eq based on path only (S7-R15)
impl std::hash::Hash for DepNode {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl PartialEq for DepNode {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for DepNode {}

/// Edge in dependency graph
///
/// Represents an import relationship between two files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepEdge {
    /// Source file (importer)
    pub from: PathBuf,

    /// Target file/module (imported)
    pub to: PathBuf,

    /// Import statement line number (1-indexed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,

    /// The actual import statement text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_text: Option<String>,
}

impl DepEdge {
    /// Create a new edge with minimal information
    pub fn new(from: PathBuf, to: PathBuf) -> Self {
        Self {
            from,
            to,
            line: None,
            import_text: None,
        }
    }

    /// Create a new edge with line number
    pub fn with_line(from: PathBuf, to: PathBuf, line: usize) -> Self {
        Self {
            from,
            to,
            line: Some(line),
            import_text: None,
        }
    }

    /// Create a new edge with full information
    pub fn with_details(from: PathBuf, to: PathBuf, line: usize, import_text: String) -> Self {
        Self {
            from,
            to,
            line: Some(line),
            import_text: Some(import_text),
        }
    }
}

/// A circular dependency cycle
///
/// Represents a cycle in the dependency graph where files form a loop.
/// Example: A -> B -> C -> A is a cycle of length 3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepCycle {
    /// Ordered list of files in the cycle
    /// First element is the canonical start (lexicographically smallest after canonicalization)
    pub path: Vec<PathBuf>,

    /// Length of cycle (number of unique nodes)
    pub length: usize,
}

impl DepCycle {
    /// Create a new cycle from a list of paths
    pub fn new(path: Vec<PathBuf>) -> Self {
        let length = path.len();
        Self { path, length }
    }

    /// Return a canonical representation of this cycle for deduplication
    ///
    /// The canonical form:
    /// 1. Rotates the cycle so it starts with the lexicographically smallest path
    /// 2. This ensures the same cycle starting from different nodes compares equal
    ///
    /// Example: [B, C, A] and [A, B, C] both canonicalize to [A, B, C]
    pub fn canonical(&self) -> DepCycle {
        if self.path.is_empty() {
            return self.clone();
        }

        // Find the index of the lexicographically smallest path
        let min_idx = self
            .path
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| *p)
            .map(|(i, _)| i)
            .unwrap_or(0);

        // Rotate the path so it starts with the smallest element
        let mut canonical_path = Vec::with_capacity(self.path.len());
        canonical_path.extend(self.path[min_idx..].iter().cloned());
        canonical_path.extend(self.path[..min_idx].iter().cloned());

        DepCycle {
            path: canonical_path,
            length: self.length,
        }
    }
}

// Implement Hash and Eq for DepCycle based on canonical form
impl std::hash::Hash for DepCycle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the canonical representation
        let canonical = self.canonical();
        canonical.path.hash(state);
    }
}

impl PartialEq for DepCycle {
    fn eq(&self, other: &Self) -> bool {
        // Compare canonical representations
        self.canonical().path == other.canonical().path
    }
}

impl Eq for DepCycle {}

/// Dependency kind
///
/// Classification of where a dependency comes from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum DepKind {
    /// Internal project dependency (files within the project)
    #[default]
    Internal,

    /// External third-party package
    External,

    /// Standard library module
    Stdlib,
}

/// Analysis statistics
///
/// Summary statistics about the dependency analysis.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepStats {
    /// Total files analyzed
    pub total_files: usize,

    /// Total internal dependency edges
    pub total_internal_deps: usize,

    /// Total external dependencies (unique packages)
    pub total_external_deps: usize,

    /// Maximum dependency depth (longest path from any root)
    pub max_depth: usize,

    /// Number of circular dependencies found
    pub cycles_found: usize,

    /// Files with no outgoing dependencies (leaf nodes)
    pub leaf_files: usize,

    /// Files with no incoming dependencies (root nodes)
    pub root_files: usize,
}

impl DepStats {
    /// Create stats with only the basic counts set
    pub fn new(total_files: usize, total_internal_deps: usize, total_external_deps: usize) -> Self {
        Self {
            total_files,
            total_internal_deps,
            total_external_deps,
            ..Default::default()
        }
    }
}

/// Options for dependency analysis
///
/// Configuration options that control the behavior of dependency analysis.
#[derive(Debug, Clone, Default)]
pub struct DepsOptions {
    /// Include external (third-party) dependencies in the report
    pub include_external: bool,

    /// Collapse files into package-level nodes
    pub collapse_packages: bool,

    /// Maximum depth for transitive dependencies (None = unlimited)
    pub max_depth: Option<usize>,

    /// Only analyze and report circular dependencies
    pub show_cycles_only: bool,

    /// Maximum cycle length to report (cycles longer than this are excluded)
    pub max_cycle_length: Option<usize>,

    /// Language to analyze (None = auto-detect)
    pub language: Option<String>,
}

impl DepsOptions {
    /// Create options with external dependencies included
    pub fn with_external() -> Self {
        Self {
            include_external: true,
            ..Default::default()
        }
    }

    /// Create options focused on cycle detection
    pub fn cycles_only() -> Self {
        Self {
            show_cycles_only: true,
            ..Default::default()
        }
    }

    /// Set maximum cycle length
    pub fn with_max_cycle_length(mut self, max_length: usize) -> Self {
        self.max_cycle_length = Some(max_length);
        self
    }

    /// Set maximum depth for transitive dependencies
    pub fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = Some(max_depth);
        self
    }
}

// =============================================================================
// Analysis Functions (Phase 2)
// =============================================================================

/// Build dependency graph for a directory.
///
/// This function:
/// 1. Walks the directory to find source files for the given language
/// 2. Builds a module_name -> file_path index for O(1) lookup (S7-R8)
/// 3. For each file, parses imports and resolves them to file paths
/// 4. Handles relative imports with current file context (S7-R3)
/// 5. Calculates stats and returns a DepsReport
///
/// # Arguments
///
/// * `path` - Root directory to analyze
/// * `options` - Analysis configuration options
///
/// # Returns
///
/// * `Ok(DepsReport)` - Dependency analysis results
/// * `Err(TldrError)` - If path doesn't exist or other errors
///
/// # Example
///
/// ```ignore
/// use std::path::Path;
/// use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
///
/// let report = analyze_dependencies(Path::new("src"), &DepsOptions::default())?;
/// println!("Found {} files with {} internal deps",
///          report.stats.total_files,
///          report.stats.total_internal_deps);
/// ```
pub fn analyze_dependencies(path: &Path, options: &DepsOptions) -> TldrResult<DepsReport> {
    // Validate path exists
    if !path.exists() {
        return Err(crate::error::TldrError::PathNotFound(path.to_path_buf()));
    }

    // Canonicalize root path (S7-R14)
    let root = dunce::canonicalize(path)
        .map_err(|_| crate::error::TldrError::PathNotFound(path.to_path_buf()))?;

    // Detect language from options or auto-detect from files
    let language = if let Some(ref lang_str) = options.language {
        Language::from_str(lang_str).unwrap_or(Language::Python)
    } else {
        detect_dominant_language(&root)?
    };

    // Get extensions for this language
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Get file tree and collect files
    let tree = get_file_tree(&root, Some(&extensions), true, Some(&IgnoreSpec::default()))?;
    let files = collect_files(&tree, &root);

    // Handle empty directory case
    if files.is_empty() {
        return Ok(DepsReport {
            root: make_relative_path(&root, &root),
            language: language.as_str().to_string(),
            internal_dependencies: BTreeMap::new(),
            external_dependencies: BTreeMap::new(),
            circular_dependencies: Vec::new(),
            stats: DepStats::default(),
        });
    }

    // Build module index for O(1) lookup (S7-R8)
    let module_index = build_module_index(&root, &files, language);

    // Build dependency graph
    let mut internal_dependencies: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut external_dependencies: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let mut total_internal_deps = 0;

    for file_path in &files {
        let relative_path = make_relative_path(file_path, &root);

        // Parse imports from file
        let imports = match get_imports(file_path, language) {
            Ok(imports) => imports,
            Err(e) => {
                // Skip files with parse errors (recoverable)
                if is_recoverable_error(&e) {
                    internal_dependencies.insert(relative_path, Vec::new());
                    continue;
                }
                return Err(e);
            }
        };

        let mut file_internal_deps: Vec<PathBuf> = Vec::new();
        let mut file_external_deps: Vec<String> = Vec::new();

        for import in imports {
            // Classify the import (Phase 4)
            let dep_kind = classify_import(&import, &root, file_path, &module_index, language);

            match dep_kind {
                DepKind::Internal => {
                    // Try to resolve to get the actual file path
                    if let Some(target_path) =
                        resolve_import(&import, &root, file_path, &module_index, language)
                    {
                        let target_relative = make_relative_path(&target_path, &root);

                        // Skip self-imports
                        if target_relative != relative_path {
                            // Deduplicate within file
                            if !file_internal_deps.contains(&target_relative) {
                                file_internal_deps.push(target_relative);
                                total_internal_deps += 1;
                            }
                        }
                    }
                }
                DepKind::External | DepKind::Stdlib => {
                    // Only track external deps if include_external is true
                    if options.include_external {
                        // Use base module name (first component)
                        let module_name = import.module.split('.').next().unwrap_or(&import.module);
                        // For TS, strip leading ./ or ../
                        let clean_name = module_name
                            .trim_start_matches("./")
                            .trim_start_matches("../");
                        if !file_external_deps.contains(&clean_name.to_string()) {
                            file_external_deps.push(clean_name.to_string());
                        }
                    }
                }
            }
        }

        internal_dependencies.insert(relative_path.clone(), file_internal_deps);
        if options.include_external && !file_external_deps.is_empty() {
            external_dependencies.insert(relative_path, file_external_deps);
        }
    }

    // Go same-package implicit dependencies:
    // In Go, all files in the same directory share the same package scope.
    // Add edges between files in the same package directory.
    if language == Language::Go {
        let go_packages = group_go_files_by_package(&root, &files);
        for pkg_files in go_packages.values() {
            if pkg_files.len() < 2 {
                continue;
            }
            for file_a in pkg_files {
                let rel_a = make_relative_path(file_a, &root);
                for file_b in pkg_files {
                    let rel_b = make_relative_path(file_b, &root);
                    if rel_a == rel_b {
                        continue;
                    }
                    // Add implicit same-package dependency
                    if let Some(deps) = internal_dependencies.get_mut(&rel_a) {
                        if !deps.contains(&rel_b) {
                            deps.push(rel_b.clone());
                            total_internal_deps += 1;
                        }
                    }
                }
            }
        }
    }

    // Calculate stats
    let total_files = files.len();

    // Count unique external packages
    let mut unique_external: HashSet<&String> = HashSet::new();
    for deps in external_dependencies.values() {
        for dep in deps {
            unique_external.insert(dep);
        }
    }
    let total_external_deps = unique_external.len();

    // Calculate leaf and root files
    let mut incoming_count: HashMap<&PathBuf, usize> = HashMap::new();
    for deps in internal_dependencies.values() {
        for dep in deps {
            *incoming_count.entry(dep).or_insert(0) += 1;
        }
    }

    let leaf_files = internal_dependencies
        .iter()
        .filter(|(_, deps)| deps.is_empty())
        .count();

    let root_files = internal_dependencies
        .keys()
        .filter(|path| !incoming_count.contains_key(path))
        .count();

    // Collapse to packages if requested (Phase 7)
    let final_deps = if options.collapse_packages {
        collapse_to_packages(&internal_dependencies, &root)
    } else {
        internal_dependencies.clone()
    };

    // Detect circular dependencies (Phase 3) - use final_deps
    let max_cycle_length = options.max_cycle_length.unwrap_or(10);
    let circular_dependencies = detect_cycles(&final_deps, max_cycle_length);
    let cycles_found = circular_dependencies.len();

    // Calculate depth stats (Phase 7)
    let (max_depth, leaf_files_calc, root_files_calc) = calculate_depth_stats(&final_deps);

    let stats = DepStats {
        total_files,
        total_internal_deps,
        total_external_deps,
        max_depth,
        cycles_found,
        leaf_files: if options.collapse_packages { leaf_files_calc } else { leaf_files },
        root_files: if options.collapse_packages { root_files_calc } else { root_files },
    };

    Ok(DepsReport {
        root: make_relative_path(&root, &root),
        language: language.as_str().to_string(),
        internal_dependencies: final_deps,
        external_dependencies,
        circular_dependencies,
        stats,
    })
}

// =============================================================================
// Cycle Detection (Phase 3)
// =============================================================================

/// Detect circular dependencies in the import graph using DFS with back-edge detection.
///
/// This function finds all cycles in the dependency graph using depth-first search.
/// When we encounter a back-edge (an edge to a node already in the current recursion stack),
/// we've found a cycle.
///
/// # Risk Mitigations
///
/// - S7-R1: Cycles are canonicalized for deduplication (using DepCycle::canonical())
/// - S7-R2: Uses both visited set AND recursion stack separately
/// - S7-R6: Uses HashSet<DepCycle> to deduplicate identical cycles from different start nodes
///
/// # Arguments
///
/// * `deps` - The internal dependency graph as adjacency list
/// * `max_length` - Maximum cycle length to report (cycles longer than this are excluded)
///
/// # Returns
///
/// A vector of deduplicated cycles, each canonicalized to start from the lexicographically
/// smallest path.
fn detect_cycles(deps: &BTreeMap<PathBuf, Vec<PathBuf>>, max_length: usize) -> Vec<DepCycle> {
    // Use HashSet for deduplication (S7-R6)
    // DepCycle implements Hash and Eq based on canonical form
    let mut cycles: HashSet<DepCycle> = HashSet::new();

    // Track globally visited nodes (optimization: don't re-explore fully processed nodes)
    let mut visited: HashSet<PathBuf> = HashSet::new();

    // Process each node as a potential cycle start
    for start_node in deps.keys() {
        if visited.contains(start_node) {
            continue;
        }

        // Track recursion stack for this DFS tree (S7-R2)
        let mut rec_stack: Vec<PathBuf> = Vec::new();
        let mut rec_set: HashSet<PathBuf> = HashSet::new();

        dfs_find_cycles(
            start_node,
            deps,
            &mut visited,
            &mut rec_stack,
            &mut rec_set,
            &mut cycles,
            max_length,
        );
    }

    // Convert HashSet to Vec (cycles are already deduplicated)
    cycles.into_iter().collect()
}

/// DFS helper for cycle detection.
///
/// Performs depth-first search from `node`, tracking the recursion stack.
/// When we find a back-edge (edge to a node in rec_set), we extract the cycle.
fn dfs_find_cycles(
    node: &PathBuf,
    deps: &BTreeMap<PathBuf, Vec<PathBuf>>,
    visited: &mut HashSet<PathBuf>,
    rec_stack: &mut Vec<PathBuf>,
    rec_set: &mut HashSet<PathBuf>,
    cycles: &mut HashSet<DepCycle>,
    max_length: usize,
) {
    // Mark as visited and add to recursion stack
    visited.insert(node.clone());
    rec_stack.push(node.clone());
    rec_set.insert(node.clone());

    // Process all neighbors (dependencies)
    if let Some(neighbors) = deps.get(node) {
        for neighbor in neighbors {
            if rec_set.contains(neighbor) {
                // Back-edge found! Extract the cycle from the recursion stack
                if let Some(start_idx) = rec_stack.iter().position(|n| n == neighbor) {
                    let cycle_path: Vec<PathBuf> = rec_stack[start_idx..].to_vec();

                    // Only include cycles within max_length
                    if cycle_path.len() <= max_length {
                        let cycle = DepCycle::new(cycle_path);
                        // HashSet with DepCycle's canonical-based Eq handles deduplication
                        cycles.insert(cycle);
                    }
                }
            } else if !visited.contains(neighbor) {
                // Recurse to unvisited neighbor
                dfs_find_cycles(
                    neighbor,
                    deps,
                    visited,
                    rec_stack,
                    rec_set,
                    cycles,
                    max_length,
                );
            }
            // If visited but not in rec_set, it's a cross-edge or forward-edge, not a back-edge
        }
    }

    // Remove from recursion stack when backtracking
    rec_stack.pop();
    rec_set.remove(node);
}

// =============================================================================
// Advanced Features (Phase 7)
// =============================================================================

/// Compute transitive dependencies up to max_depth using BFS.
///
/// Returns a map from each node to its reachable nodes with their distances.
/// This is useful for computing transitive closure and depth statistics.
///
/// # Arguments
///
/// * `deps` - The dependency graph as adjacency list
/// * `max_depth` - Maximum depth to traverse (None = unlimited)
///
/// # Returns
///
/// Map of node -> {reachable_node -> distance}
pub fn compute_transitive_deps(
    deps: &BTreeMap<PathBuf, Vec<PathBuf>>,
    max_depth: Option<usize>,
) -> BTreeMap<PathBuf, BTreeMap<PathBuf, usize>> {
    let mut result: BTreeMap<PathBuf, BTreeMap<PathBuf, usize>> = BTreeMap::new();
    let effective_max = max_depth.unwrap_or(usize::MAX);

    for start_node in deps.keys() {
        let mut reachable: BTreeMap<PathBuf, usize> = BTreeMap::new();
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::new();
        let mut visited: HashSet<PathBuf> = HashSet::new();

        queue.push_back((start_node.clone(), 0));
        visited.insert(start_node.clone());

        while let Some((node, depth)) = queue.pop_front() {
            // Skip if we've exceeded max depth
            if depth > effective_max {
                continue;
            }

            // Record this node if it's not the start node (depth > 0)
            if depth > 0 {
                reachable.insert(node.clone(), depth);
            }

            // Don't explore beyond max_depth
            if depth >= effective_max {
                continue;
            }

            // Explore neighbors
            if let Some(neighbors) = deps.get(&node) {
                for neighbor in neighbors {
                    if !visited.contains(neighbor) {
                        visited.insert(neighbor.clone());
                        queue.push_back((neighbor.clone(), depth + 1));
                    }
                }
            }
        }

        result.insert(start_node.clone(), reachable);
    }

    result
}

/// Collapse file-level dependencies to package level.
///
/// This function merges files in the same directory into a single package node.
/// For Python, this means files in the same directory become one package.
///
/// # Arguments
///
/// * `deps` - The file-level dependency graph
/// * `root` - The project root path
///
/// # Returns
///
/// Package-level dependency graph where keys and values are directory paths
pub fn collapse_to_packages(
    deps: &BTreeMap<PathBuf, Vec<PathBuf>>,
    _root: &Path,
) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    let mut package_deps: BTreeMap<PathBuf, HashSet<PathBuf>> = BTreeMap::new();

    for (file, file_deps) in deps {
        // Get the package (parent directory) for this file
        let from_pkg = file.parent().map(|p| p.to_path_buf()).unwrap_or_default();

        for dep in file_deps {
            // Get the package for the dependency
            let to_pkg = dep.parent().map(|p| p.to_path_buf()).unwrap_or_default();

            // Only add if it's a cross-package dependency (not within same package)
            if from_pkg != to_pkg {
                package_deps
                    .entry(from_pkg.clone())
                    .or_default()
                    .insert(to_pkg);
            }
        }

        // Ensure the package exists in the map even if it has no cross-package deps
        package_deps.entry(from_pkg).or_default();
    }

    // Convert HashSet to Vec for the return type
    package_deps
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

/// Calculate dependency depth statistics.
///
/// Computes:
/// - max_depth: The longest path from any root to any leaf in the DAG
/// - leaf_files: Count of files with no outgoing dependencies
/// - root_files: Count of files with no incoming dependencies
///
/// # Arguments
///
/// * `deps` - The dependency graph as adjacency list
///
/// # Returns
///
/// Tuple of (max_depth, leaf_files, root_files)
pub fn calculate_depth_stats(deps: &BTreeMap<PathBuf, Vec<PathBuf>>) -> (usize, usize, usize) {
    if deps.is_empty() {
        return (0, 0, 0);
    }

    // Build incoming edges count
    let mut incoming: HashMap<&PathBuf, usize> = HashMap::new();
    for node in deps.keys() {
        incoming.entry(node).or_insert(0);
    }
    for file_deps in deps.values() {
        for dep in file_deps {
            *incoming.entry(dep).or_insert(0) += 1;
        }
    }

    // Calculate leaf files (no outgoing deps)
    let leaf_files = deps.iter().filter(|(_, d)| d.is_empty()).count();

    // Calculate root files (no incoming deps)
    let root_files = deps
        .keys()
        .filter(|k| incoming.get(k).copied().unwrap_or(0) == 0)
        .count();

    // Calculate max depth using BFS from all root nodes
    // This finds the longest path from any root to any node
    let mut max_depth = 0;

    // For each node, compute the maximum depth from any root to this node
    // We'll use dynamic programming with topological order

    // First, compute in-degrees for topological sort
    let mut in_degree: HashMap<&PathBuf, usize> = HashMap::new();
    for node in deps.keys() {
        in_degree.entry(node).or_insert(0);
    }
    for file_deps in deps.values() {
        for dep in file_deps {
            // Only count if the dep is actually in our graph
            if deps.contains_key(dep) {
                *in_degree.entry(dep).or_insert(0) += 1;
            }
        }
    }

    // Initialize distances from roots
    let mut distances: HashMap<&PathBuf, usize> = HashMap::new();
    let mut queue: VecDeque<&PathBuf> = VecDeque::new();

    // Start with root nodes (in-degree 0)
    for (node, &degree) in &in_degree {
        if degree == 0 {
            distances.insert(node, 0);
            queue.push_back(node);
        }
    }

    // Process in topological order
    while let Some(node) = queue.pop_front() {
        let current_dist = *distances.get(node).unwrap_or(&0);

        if let Some(neighbors) = deps.get(node) {
            for neighbor in neighbors {
                // Only process if neighbor is in our graph
                if let Some(in_deg) = in_degree.get_mut(&neighbor) {
                    // Update distance to neighbor (take max of all paths)
                    let new_dist = current_dist + 1;
                    let entry = distances.entry(neighbor).or_insert(0);
                    if new_dist > *entry {
                        *entry = new_dist;
                    }

                    // Update max_depth
                    if new_dist > max_depth {
                        max_depth = new_dist;
                    }

                    // Decrement in-degree and add to queue if ready
                    *in_deg -= 1;
                    if *in_deg == 0 {
                        queue.push_back(neighbor);
                    }
                }
            }
        }
    }

    (max_depth, leaf_files, root_files)
}

/// Build module name -> file path index for O(1) lookup (S7-R8).
///
/// Creates a mapping from module names to file paths to avoid O(n^2) resolution.
/// For Python: "src.utils" -> "src/utils.py", "src.utils" -> "src/utils/__init__.py"
/// For TypeScript: "./utils" -> "src/utils.ts"
/// For Java: "com.google.common.base.Preconditions" -> "com/google/common/base/Preconditions.java"
pub fn build_module_index(
    root: &Path,
    files: &[PathBuf],
    language: Language,
) -> HashMap<String, PathBuf> {
    let mut index: HashMap<String, PathBuf> = HashMap::new();

    // For Go, read the module path from go.mod once before iterating files
    let go_module_path = if language == Language::Go {
        read_go_module_path(root)
    } else {
        None
    };

    for file_path in files {
        let relative = match file_path.strip_prefix(root) {
            Ok(r) => r,
            Err(_) => continue,
        };

        index_module_for_language(
            &mut index,
            file_path,
            relative,
            language,
            go_module_path.as_deref(),
        );
    }

    index
}

fn index_module_for_language(
    index: &mut HashMap<String, PathBuf>,
    file_path: &Path,
    relative: &Path,
    language: Language,
    go_module_path: Option<&str>,
) {
    match language {
        Language::Python => index_python_module(index, file_path, relative),
        Language::TypeScript | Language::JavaScript => {
            index_ts_js_module(index, file_path, relative)
        }
        Language::Go => index_go_module(index, file_path, relative, go_module_path),
        Language::Rust => index_rust_module(index, file_path, relative),
        Language::Java => index_java_module(index, file_path, relative),
        Language::Kotlin => index_kotlin_module(index, file_path, relative),
        Language::C | Language::Cpp => index_c_cpp_module(index, file_path, relative),
        Language::Ruby => index_ruby_module(index, file_path, relative),
        Language::CSharp => index_csharp_module(index, file_path, relative),
        Language::Scala => index_scala_module(index, file_path, relative),
        Language::Elixir => index_elixir_module(index, file_path, relative),
        Language::Ocaml => index_ocaml_module(index, file_path, relative),
        Language::Php => index_php_module(index, file_path, relative),
        _ => {}
    }
}

fn index_python_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let module_path = path_to_module_name(&stem);
    index.insert(module_path, fp.clone());

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        if name_str != "__init__" {
            index.insert(name_str.to_string(), fp.clone());
        }
    }

    if relative.ends_with("__init__.py") {
        if let Some(parent) = stem.parent() {
            let parent_module = path_to_module_name(parent);
            index.insert(parent_module, fp.clone());
            if let Some(pkg_name) = parent.file_name() {
                index.insert(pkg_name.to_string_lossy().to_string(), fp.clone());
            }
        }
    }
}

fn index_ts_js_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy();
    index.insert(format!("./{}", stem_str), fp.clone());

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        if name_str != "index" {
            index.insert(format!("./{}", name_str), fp.clone());
        }
    }

    if relative.file_stem() == Some(std::ffi::OsStr::new("index")) {
        if let Some(parent) = stem.parent() {
            index.insert(format!("./{}", parent.display()), fp.clone());
        }
    }
}

fn index_go_module(
    index: &mut HashMap<String, PathBuf>,
    file_path: &Path,
    relative: &Path,
    go_module_path: Option<&str>,
) {
    let fp = file_path.to_path_buf();
    let pkg_dir = relative
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if !pkg_dir.is_empty() {
        index.insert(pkg_dir.clone(), fp.clone());
    }

    if let Some(mod_path) = go_module_path {
        if pkg_dir.is_empty() {
            index.insert(mod_path.to_string(), fp.clone());
        } else {
            index.insert(format!("{}/{}", mod_path, pkg_dir), fp.clone());
        }
    }
}

fn index_rust_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy();
    let crate_path = stem_str.replace('/', "::");
    if crate_path.starts_with("src::") {
        let without_src = crate_path.strip_prefix("src::").unwrap_or(&crate_path);
        index.insert(format!("crate::{}", without_src), fp.clone());
    }
    index.insert(format!("crate::{}", crate_path), fp.clone());

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        if name_str != "mod" && name_str != "lib" {
            index.insert(name_str.to_string(), fp.clone());
        }
    }

    if relative.file_stem() == Some(std::ffi::OsStr::new("mod")) {
        if let Some(parent) = stem.parent() {
            if let Some(pkg_name) = parent.file_name() {
                index.insert(pkg_name.to_string_lossy().to_string(), fp.clone());
                index.insert(
                    format!("crate::{}", pkg_name.to_string_lossy()),
                    fp.clone(),
                );
            }
        }
    }
}

/// Strip the longest known source-root prefix from a relative path, matching
/// anywhere on a `/` boundary (handles nested/multi-module projects like
/// `backend/src/main/java/com/example/Foo`).
fn strip_jvm_prefix<'a>(path: &'a str, prefixes: &[&str]) -> &'a str {
    let mut best_end: Option<usize> = None;
    let mut best_prefix_len: usize = 0;
    for prefix in prefixes {
        if let Some(pos) = path.find(prefix) {
            if (pos == 0 || path.as_bytes()[pos - 1] == b'/')
                && prefix.len() > best_prefix_len {
                    best_prefix_len = prefix.len();
                    best_end = Some(pos + prefix.len());
                }
        }
    }
    if let Some(end) = best_end {
        &path[end..]
    } else {
        path
    }
}

fn index_java_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let path_str = stem.to_string_lossy();
    let cleaned = strip_jvm_prefix(&path_str, &["src/main/java/", "src/test/java/", "src/", "lib/", "app/"]);
    let qualified_name = cleaned.replace(['/', '\\'], ".");
    if !qualified_name.is_empty() {
        index.insert(qualified_name, fp.clone());
    }
    if let Some(class_name) = stem.file_name() {
        let name_str = class_name.to_string_lossy();
        if !name_str.is_empty() {
            index.insert(name_str.to_string(), fp.clone());
        }
    }
}

fn index_kotlin_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem_path = relative.with_extension("");
    let path_str = stem_path.to_string_lossy();
    // Also strip .kts if with_extension("") didn't catch it (e.g. "build.gradle.kts")
    let path_str_ref: &str = &path_str;
    let stripped = path_str_ref.strip_suffix(".kts").unwrap_or(path_str_ref);
    let cleaned = strip_jvm_prefix(stripped, &["src/main/kotlin/", "src/test/kotlin/", "src/", "lib/", "app/"]);
    let qualified_name = cleaned.replace(['/', '\\'], ".");
    if !qualified_name.is_empty() {
        index.insert(qualified_name, fp.clone());
    }

    if let Some(class_name) = relative.file_stem() {
        let name_str = class_name.to_string_lossy();
        if !name_str.is_empty() {
            index.insert(name_str.to_string(), fp.clone());
        }
    }
}

fn index_c_cpp_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    index.insert(relative.to_string_lossy().to_string(), fp.clone());
    if let Some(name) = relative.file_name() {
        index.insert(name.to_string_lossy().to_string(), fp.clone());
    }

    let components: Vec<_> = relative.components().collect();
    for start in 1..components.len() {
        let sub_path: PathBuf = components[start..].iter().collect();
        let sub_str = sub_path.to_string_lossy().to_string();
        index.entry(sub_str).or_insert_with(|| fp.clone());
    }
}

fn index_ruby_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy().to_string();
    index.insert(stem_str.clone(), fp.clone());

    let stripped = stem_str
        .strip_prefix("lib/")
        .or_else(|| stem_str.strip_prefix("app/"));
    if let Some(s) = stripped {
        index.insert(s.to_string(), fp.clone());
    }

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        index
            .entry(name_str.to_string())
            .or_insert_with(|| fp.clone());
    }
}

fn index_csharp_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let path_str = stem.to_string_lossy();
    let cleaned = strip_jvm_prefix(&path_str, &["src/", "lib/", "app/"]);
    let qualified = cleaned.replace(['/', '\\'], ".");
    if !qualified.is_empty() {
        index.insert(qualified, fp.clone());
    }
    if let Some(parent) = Path::new(cleaned).parent() {
        let ns = parent.to_string_lossy().replace(['/', '\\'], ".");
        if !ns.is_empty() {
            index.entry(ns).or_insert_with(|| fp.clone());
        }
    }
    if let Some(class_name) = stem.file_name() {
        let name_str = class_name.to_string_lossy();
        if !name_str.is_empty() {
            index
                .entry(name_str.to_string())
                .or_insert_with(|| fp.clone());
        }
    }
}

fn index_scala_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let path_str = stem.to_string_lossy();
    let cleaned = strip_jvm_prefix(&path_str, &["src/main/scala/", "src/test/scala/", "src/", "lib/", "app/"]);
    let qualified = cleaned.replace(['/', '\\'], ".");
    if !qualified.is_empty() {
        index.insert(qualified, fp.clone());
    }
    if let Some(parent) = Path::new(cleaned).parent() {
        let pkg = parent.to_string_lossy().replace(['/', '\\'], ".");
        if !pkg.is_empty() {
            index.entry(pkg).or_insert_with(|| fp.clone());
        }
    }
    if let Some(class_name) = stem.file_name() {
        let name_str = class_name.to_string_lossy();
        if !name_str.is_empty() {
            index
                .entry(name_str.to_string())
                .or_insert_with(|| fp.clone());
        }
    }
}

fn index_elixir_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy().to_string();
    index.insert(stem_str.clone(), fp.clone());

    let stripped = stem_str
        .strip_prefix("lib/")
        .or_else(|| stem_str.strip_prefix("test/"));
    if let Some(s) = stripped {
        index.insert(s.to_string(), fp.clone());
        let module_name = s
            .split('/')
            .map(|part| part.split('_').map(capitalize_first).collect::<String>())
            .collect::<Vec<_>>()
            .join(".");
        if !module_name.is_empty() {
            index.insert(module_name, fp.clone());
        }
    }

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        index
            .entry(name_str.to_string())
            .or_insert_with(|| fp.clone());
    }
}

fn index_ocaml_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy().to_string();
    index.insert(stem_str.clone(), fp.clone());

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        let module_name = capitalize_first(&name_str);
        if !module_name.is_empty() {
            index.entry(module_name).or_insert_with(|| fp.clone());
        }
    }

    let dot_path = stem_str.replace('/', ".");
    if dot_path.contains('.') {
        let capitalized = dot_path
            .split('.')
            .map(capitalize_first)
            .collect::<Vec<_>>()
            .join(".");
        index.entry(capitalized).or_insert_with(|| fp.clone());
    }
}

fn index_php_module(index: &mut HashMap<String, PathBuf>, file_path: &Path, relative: &Path) {
    let fp = file_path.to_path_buf();
    let stem = relative.with_extension("");
    let stem_str = stem.to_string_lossy().to_string();
    index.insert(stem_str.clone(), fp.clone());

    let stripped = stem_str
        .strip_prefix("src/")
        .or_else(|| stem_str.strip_prefix("app/"))
        .or_else(|| stem_str.strip_prefix("lib/"));
    if let Some(s) = stripped {
        index.insert(s.to_string(), fp.clone());
    }

    let namespace = stem_str.replace('/', "\\");
    if !namespace.is_empty() {
        index.insert(namespace, fp.clone());
    }

    if let Some(name) = stem.file_name() {
        let name_str = name.to_string_lossy();
        index
            .entry(name_str.to_string())
            .or_insert_with(|| fp.clone());
    }
}

/// Resolve an import to a file path.
///
/// Handles relative imports (S7-R3) by using the current file's location
/// as context for resolving relative module paths.
fn resolve_import(
    import: &ImportInfo,
    root: &Path,
    current_file: &Path,
    index: &HashMap<String, PathBuf>,
    language: Language,
) -> Option<PathBuf> {
    let module = &import.module;

    match language {
        Language::Python => resolve_python_import(module, root, current_file, index),
        Language::TypeScript | Language::JavaScript => {
            resolve_ts_import(module, root, current_file, index)
        }
        Language::Go => resolve_go_import(module, index),
        Language::Rust => resolve_rust_import(module, index),
        Language::Java => resolve_java_import(module, root, current_file, index),
        Language::C | Language::Cpp => resolve_c_cpp_import(import, root, current_file, index),
        Language::Ruby => resolve_ruby_import(import, root, current_file, index),
        Language::CSharp => resolve_csharp_import(import, root, current_file, index),
        Language::Scala => resolve_scala_import(import, root, current_file, index),
        Language::Elixir => resolve_elixir_import(import, root, current_file, index),
        Language::Ocaml => resolve_ocaml_import(import, root, current_file, index),
        Language::Php => resolve_php_import(import, root, current_file, index),
        _ => None,
    }
}

/// Resolve Python import to file path.
///
/// Handles:
/// - Absolute imports: "from mypackage.utils import x" -> mypackage/utils.py
/// - Relative imports: "from .utils import x" -> current_dir/utils.py
/// - Deep relative: "from ..parent import x" -> parent_dir/parent.py
fn resolve_python_import(
    module: &str,
    root: &Path,
    current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    // Handle relative imports (S7-R3)
    if module.starts_with('.') {
        return resolve_python_relative_import(module, root, current_file, index);
    }

    // Try direct lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try first component (from pkg.submodule import X -> pkg/submodule.py)
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() > 1 {
        // Try progressively shorter prefixes
        for i in (1..=parts.len()).rev() {
            let prefix = parts[..i].join(".");
            if let Some(path) = index.get(&prefix) {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Resolve Python relative import.
///
/// Counts leading dots and walks up the directory tree accordingly.
fn resolve_python_relative_import(
    module: &str,
    root: &Path,
    current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    // Count leading dots
    let dot_count = module.chars().take_while(|c| *c == '.').count();
    let remainder = &module[dot_count..];

    // Start from current file's directory
    let current_dir = current_file.parent()?;

    // Walk up directories based on dot count
    // . = same directory, .. = parent, ... = grandparent, etc.
    let mut target_dir = current_dir.to_path_buf();
    for _ in 1..dot_count {
        target_dir = target_dir.parent()?.to_path_buf();
    }

    if remainder.is_empty() {
        // "from . import X" - look for __init__.py in current dir
        let init_path = target_dir.join("__init__.py");
        if init_path.exists() {
            return Some(init_path);
        }
        return None;
    }

    // Convert remainder to path components
    let parts: Vec<&str> = remainder.split('.').collect();
    for part in &parts {
        target_dir = target_dir.join(part);
    }

    // Try .py file
    let py_path = target_dir.with_extension("py");
    if py_path.exists() && py_path.starts_with(root) {
        return Some(py_path);
    }

    // Try __init__.py in directory
    let init_path = target_dir.join("__init__.py");
    if init_path.exists() && init_path.starts_with(root) {
        return Some(init_path);
    }

    // Try index lookup with relative path
    let relative_target = target_dir.strip_prefix(root).ok()?;
    let module_name = path_to_module_name(relative_target);
    index.get(&module_name).cloned()
}

/// Resolve TypeScript/JavaScript import to file path.
fn resolve_ts_import(
    module: &str,
    root: &Path,
    current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    // Handle relative imports
    if module.starts_with("./") || module.starts_with("../") {
        let current_dir = current_file.parent()?;
        let resolved = current_dir.join(module);
        let normalized = normalize_path(&resolved);

        // Try with various extensions
        for ext in &[".ts", ".tsx", ".js", ".jsx"] {
            let with_ext = normalized.with_extension(&ext[1..]);
            if with_ext.exists() && with_ext.starts_with(root) {
                return Some(with_ext);
            }
        }

        // Try index file
        for ext in &[".ts", ".tsx", ".js", ".jsx"] {
            let index_path = normalized.join(format!("index{}", ext));
            if index_path.exists() && index_path.starts_with(root) {
                return Some(index_path);
            }
        }
    }

    // Try index lookup
    index.get(module).cloned()
}

/// Resolve Go import to file path.
fn resolve_go_import(module: &str, index: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    // Go imports are package paths
    // Try the full path, then try last component
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try just the last component
    if let Some(last) = module.rsplit('/').next() {
        if let Some(path) = index.get(last) {
            return Some(path.clone());
        }
    }

    None
}

/// Resolve Rust import to file path.
fn resolve_rust_import(module: &str, index: &HashMap<String, PathBuf>) -> Option<PathBuf> {
    // Try direct lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Handle crate:: and self:: and super:: prefixes
    let normalized = if module.starts_with("crate::") {
        module.to_string()
    } else if module.starts_with("self::") || module.starts_with("super::") {
        // These need context - return None for now
        return None;
    } else {
        format!("crate::{}", module)
    };

    // Try progressively shorter prefixes
    let parts: Vec<&str> = normalized.split("::").collect();
    for i in (1..=parts.len()).rev() {
        let prefix = parts[..i].join("::");
        if let Some(path) = index.get(&prefix) {
            return Some(path.clone());
        }
    }

    // Try just the last component
    if let Some(last) = parts.last() {
        if let Some(path) = index.get(*last) {
            return Some(path.clone());
        }
    }

    None
}

/// Resolve Java import to file path.
///
/// Handles:
/// - Qualified imports: "com.google.common.base.Preconditions" -> direct index lookup
/// - Wildcard imports: "com.google.common.base.*" -> resolve to files in that package
/// - Static imports: "com.google.common.base.Preconditions.checkNotNull" -> strip method, resolve class
/// - Fallback: try simple class name (last component after '.')
pub fn resolve_java_import(
    module: &str,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    // Skip JDK/standard library imports
    if is_java_stdlib(module) {
        return None;
    }

    // Handle wildcard imports: "com.google.common.base.*"
    if let Some(package_prefix) = module.strip_suffix(".*") {
        // Find any file in the index whose qualified name starts with this package prefix
        for (key, path) in index {
            if key.starts_with(package_prefix)
                && key.len() > package_prefix.len()
                && key.as_bytes()[package_prefix.len()] == b'.'
            {
                // Check that what follows the prefix is a simple name (no more dots)
                let remainder = &key[package_prefix.len() + 1..];
                if !remainder.contains('.') {
                    return Some(path.clone());
                }
            }
        }
        return None;
    }

    // Try direct lookup of the full qualified name
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Handle static imports: the module string might include a method/field name
    // e.g., "com.google.common.base.Preconditions.checkNotNull"
    // Try stripping the last component (method name) and resolving the class
    if let Some(dot_pos) = module.rfind('.') {
        let class_part = &module[..dot_pos];
        if let Some(path) = index.get(class_part) {
            return Some(path.clone());
        }

        // Also try simple name of the class part (for narrow root scenarios where
        // the index only has simple class names, not fully-qualified names)
        // e.g., "com.google.common.base.Preconditions" -> try "Preconditions"
        if let Some(class_dot) = class_part.rfind('.') {
            let class_simple_name = &class_part[class_dot + 1..];
            if let Some(path) = index.get(class_simple_name) {
                return Some(path.clone());
            }
        }
    }

    // Fallback: try simple class name (last component after last '.')
    if let Some(last_dot) = module.rfind('.') {
        let simple_name = &module[last_dot + 1..];
        if let Some(path) = index.get(simple_name) {
            return Some(path.clone());
        }
    }

    None
}

/// Check if Java import is from the JDK standard library.
///
/// JDK packages start with java., javax., sun., com.sun., org.w3c., or org.xml.
pub fn is_java_stdlib(module_name: &str) -> bool {
    module_name.starts_with("java.")
        || module_name.starts_with("javax.")
        || module_name.starts_with("sun.")
        || module_name.starts_with("com.sun.")
        || module_name.starts_with("org.w3c.")
        || module_name.starts_with("org.xml.")
}

// =============================================================================
// C / C++ import resolution
// =============================================================================

/// Resolve C/C++ `#include` directive to a file path.
///
/// Handles:
/// - `#include "file.h"` (local) -> search index by filename and relative path
/// - `#include <header.h>` (system) -> return None (external/stdlib)
///
/// The `is_from` field on `ImportInfo` distinguishes system (`true`) from local
/// (`false`) includes, matching the extractor convention in `ast/imports.rs`.
fn resolve_c_cpp_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    // System includes (#include <header>) are external -- do not resolve
    if import.is_from {
        return None;
    }

    let module = &import.module;

    // Direct index lookup (handles both "utils.h" and "net/socket.h")
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    None
}

// =============================================================================
// Ruby import resolution
// =============================================================================

/// Resolve Ruby `require` / `require_relative` to a file path.
///
/// Handles:
/// - `require "module"` -> index lookup by module name
/// - `require_relative "file"` (`is_from=true`) -> index lookup; filesystem
///   resolution is handled by the index entries already containing relative paths
fn resolve_ruby_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Direct index lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try stripping leading "./" for relative requires
    let stripped = module.strip_prefix("./").unwrap_or(module);
    if stripped != module {
        if let Some(path) = index.get(stripped) {
            return Some(path.clone());
        }
    }

    None
}

// =============================================================================
// C# import resolution
// =============================================================================

/// Resolve C# `using` directive to a file path.
///
/// Handles:
/// - `using Namespace.SubNamespace;` -> index lookup by dot-separated name
/// - `using static Namespace.Class;` -> same lookup
/// - System namespaces (System.*, Microsoft.*) -> return None (stdlib)
fn resolve_csharp_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip well-known .NET standard library namespaces
    if is_csharp_stdlib(module) {
        return None;
    }

    // Direct index lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try progressively shorter prefixes (like Python/Java)
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() > 1 {
        for i in (1..parts.len()).rev() {
            let prefix = parts[..i].join(".");
            if let Some(path) = index.get(&prefix) {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Check if a C# namespace is part of the .NET standard library / framework.
fn is_csharp_stdlib(module_name: &str) -> bool {
    module_name.starts_with("System")
        || module_name.starts_with("Microsoft")
        || module_name.starts_with("Windows")
}

// =============================================================================
// Scala import resolution
// =============================================================================

/// Resolve Scala `import` statement to a file path.
///
/// Handles:
/// - `import package.Class` -> index lookup by qualified name
/// - `import package._` (wildcard, `is_from=true`) -> resolve to package directory
/// - Scala/Java stdlib imports -> return None
fn resolve_scala_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip Scala and Java standard library imports
    if is_scala_stdlib(module) {
        return None;
    }

    // Direct index lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try progressively shorter prefixes
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() > 1 {
        for i in (1..parts.len()).rev() {
            let prefix = parts[..i].join(".");
            if let Some(path) = index.get(&prefix) {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Check if a Scala import is from the standard library.
fn is_scala_stdlib(module_name: &str) -> bool {
    module_name.starts_with("scala.")
        || module_name.starts_with("java.")
        || module_name.starts_with("javax.")
}

// =============================================================================
// Elixir import resolution
// =============================================================================

/// Resolve Elixir `import`/`alias`/`require`/`use` to a file path.
///
/// Elixir modules use PascalCase dot-separated names (e.g., `Phoenix.Controller`).
/// The index maps these to file paths via path-to-module conversion.
fn resolve_elixir_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip Elixir standard library modules
    if is_elixir_stdlib(module) {
        return None;
    }

    // Direct index lookup (module name like "Phoenix.Controller")
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try progressively shorter prefixes
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() > 1 {
        for i in (1..parts.len()).rev() {
            let prefix = parts[..i].join(".");
            if let Some(path) = index.get(&prefix) {
                return Some(path.clone());
            }
        }
    }

    // Try just the last component (e.g., "Controller" from "Phoenix.Controller")
    if let Some(last) = parts.last() {
        if let Some(path) = index.get(*last) {
            return Some(path.clone());
        }
    }

    None
}

/// Check if an Elixir module is from the standard library.
fn is_elixir_stdlib(module_name: &str) -> bool {
    // Elixir stdlib modules
    let first_part = module_name.split('.').next().unwrap_or(module_name);
    matches!(
        first_part,
        "Kernel"
            | "Enum"
            | "Map"
            | "List"
            | "String"
            | "IO"
            | "File"
            | "Path"
            | "Process"
            | "Agent"
            | "Task"
            | "GenServer"
            | "Supervisor"
            | "Logger"
            | "Macro"
            | "Module"
            | "Access"
            | "Atom"
            | "Base"
            | "Bitwise"
            | "Code"
            | "Date"
            | "DateTime"
            | "Exception"
            | "Float"
            | "Function"
            | "Integer"
            | "Inspect"
            | "NaiveDateTime"
            | "Node"
            | "OptionParser"
            | "Port"
            | "Range"
            | "Regex"
            | "Registry"
            | "Stream"
            | "System"
            | "Time"
            | "Tuple"
            | "URI"
            | "Version"
    )
}

// =============================================================================
// OCaml import resolution
// =============================================================================

/// Resolve OCaml `open`/`include`/`module alias` to a file path.
///
/// OCaml modules are typically PascalCase names that correspond to filenames.
fn resolve_ocaml_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip OCaml standard library modules
    if is_ocaml_stdlib(module) {
        return None;
    }

    // Direct index lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // Try the first component for dotted names (e.g., "Stdlib.Map" -> "Stdlib")
    let parts: Vec<&str> = module.split('.').collect();
    if parts.len() > 1 {
        for i in (1..parts.len()).rev() {
            let prefix = parts[..i].join(".");
            if let Some(path) = index.get(&prefix) {
                return Some(path.clone());
            }
        }
    }

    None
}

/// Check if an OCaml module is from the standard library.
fn is_ocaml_stdlib(module_name: &str) -> bool {
    let first_part = module_name.split('.').next().unwrap_or(module_name);
    matches!(
        first_part,
        "Stdlib"
            | "List"
            | "Array"
            | "String"
            | "Bytes"
            | "Buffer"
            | "Char"
            | "Complex"
            | "Digest"
            | "Filename"
            | "Format"
            | "Fun"
            | "Gc"
            | "Hashtbl"
            | "Int32"
            | "Int64"
            | "Lazy"
            | "Lexing"
            | "Map"
            | "Marshal"
            | "Nativeint"
            | "Obj"
            | "Parsing"
            | "Printexc"
            | "Printf"
            | "Queue"
            | "Random"
            | "Scanf"
            | "Seq"
            | "Set"
            | "Stack"
            | "Stream"
            | "Sys"
            | "Uchar"
            | "Unit"
            | "Weak"
    )
}

// =============================================================================
// PHP import resolution
// =============================================================================

/// Resolve PHP `use`/`require`/`include` to a file path.
///
/// Handles:
/// - `use App\Models\User` -> index lookup by namespace
/// - `require 'file.php'` -> direct path lookup
fn resolve_php_import(
    import: &ImportInfo,
    _root: &Path,
    _current_file: &Path,
    index: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip PHP standard library / extensions
    if is_php_stdlib(module) {
        return None;
    }

    // Direct index lookup
    if let Some(path) = index.get(module) {
        return Some(path.clone());
    }

    // For namespace imports (backslash-separated), try path-style lookup
    if module.contains('\\') {
        let path_style = module.replace('\\', "/");
        if let Some(path) = index.get(&path_style) {
            return Some(path.clone());
        }

        // Try progressively shorter prefixes
        let parts: Vec<&str> = module.split('\\').collect();
        if parts.len() > 1 {
            // Try just the class name (last component)
            if let Some(last) = parts.last() {
                if let Some(path) = index.get(*last) {
                    return Some(path.clone());
                }
            }
        }
    }

    // For file paths (require/include), try stripping leading "./"
    let stripped = module.strip_prefix("./").unwrap_or(module);
    if stripped != module {
        if let Some(path) = index.get(stripped) {
            return Some(path.clone());
        }
    }

    None
}

/// Check if a PHP namespace/import is from the standard library or extensions.
fn is_php_stdlib(module_name: &str) -> bool {
    // PHP has no true stdlib namespace, but skip common built-in extensions
    let first_part = module_name.split('\\').next().unwrap_or(module_name);
    matches!(
        first_part,
        "PDO" | "DateTime" | "Exception" | "Error" | "Throwable" | "Iterator" | "Closure"
            | "stdClass" | "Generator" | "SplFixedArray" | "SplStack" | "SplQueue"
            | "SplHeap" | "SplPriorityQueue" | "ArrayObject" | "ArrayIterator"
    )
}

// =============================================================================
// External vs Internal Classification (Phase 4)
// =============================================================================

/// Classify an import as internal, stdlib, or external.
///
/// This function determines the category of an import:
/// - Internal: Part of the current project (resolvable to a file)
/// - Stdlib: Part of the language's standard library
/// - External: Third-party package (not in project, not stdlib)
///
/// # Arguments
///
/// * `import` - The import to classify
/// * `root` - Project root directory
/// * `current_file` - File containing the import
/// * `module_index` - Index of module names to file paths
/// * `language` - Programming language
///
/// # Returns
///
/// The classification of the dependency.
pub fn classify_import(
    import: &ImportInfo,
    root: &Path,
    current_file: &Path,
    module_index: &HashMap<String, PathBuf>,
    language: Language,
) -> DepKind {
    // 1. Try to resolve as internal first (using module_index)
    if resolve_import(import, root, current_file, module_index, language).is_some() {
        return DepKind::Internal;
    }

    // 2. If not found, classify as stdlib or external based on language
    let module = &import.module;
    match language {
        Language::Python => {
            if is_python_stdlib(module) {
                DepKind::Stdlib
            } else {
                DepKind::External
            }
        }
        Language::TypeScript | Language::JavaScript => {
            // TypeScript: relative imports that weren't resolved are still considered
            // attempts at internal imports (maybe missing files)
            if is_typescript_relative(module) {
                DepKind::Internal
            } else {
                DepKind::External
            }
        }
        Language::Go => {
            if is_go_stdlib(module) {
                DepKind::Stdlib
            } else {
                DepKind::External
            }
        }
        Language::Rust => {
            if is_rust_stdlib(module) {
                DepKind::Stdlib
            } else {
                DepKind::External
            }
        }
        Language::Java => {
            if is_java_stdlib(module) {
                DepKind::Stdlib
            } else {
                DepKind::External
            }
        }
        _ => DepKind::External,
    }
}

/// Check if Python import is stdlib.
///
/// Uses a comprehensive list of Python 3.11+ stdlib modules.
/// Handles dotted imports by checking the base module.
pub fn is_python_stdlib(module_name: &str) -> bool {
    // Comprehensive Python stdlib modules list (3.11+)
    const PYTHON_STDLIB: &[&str] = &[
        // Core
        "abc",
        "aifc",
        "argparse",
        "array",
        "ast",
        "asyncio",
        "atexit",
        "base64",
        "bdb",
        "binascii",
        "bisect",
        "builtins",
        "bz2",
        "calendar",
        "cgi",
        "cgitb",
        "chunk",
        "cmath",
        "cmd",
        "code",
        "codecs",
        "codeop",
        "collections",
        "colorsys",
        "compileall",
        "concurrent",
        "configparser",
        "contextlib",
        "contextvars",
        "copy",
        "copyreg",
        "cProfile",
        "csv",
        "ctypes",
        "curses",
        "dataclasses",
        "datetime",
        "dbm",
        "decimal",
        "difflib",
        "dis",
        "distutils",
        "doctest",
        "email",
        "encodings",
        "enum",
        "errno",
        "faulthandler",
        "fcntl",
        "filecmp",
        "fileinput",
        "fnmatch",
        "fractions",
        "ftplib",
        "functools",
        "gc",
        "getopt",
        "getpass",
        "gettext",
        "glob",
        "graphlib",
        "grp",
        "gzip",
        "hashlib",
        "heapq",
        "hmac",
        "html",
        "http",
        "idlelib",
        "imaplib",
        "imghdr",
        "importlib",
        "inspect",
        "io",
        "ipaddress",
        "itertools",
        "json",
        "keyword",
        "lib2to3",
        "linecache",
        "locale",
        "logging",
        "lzma",
        "mailbox",
        "mailcap",
        "marshal",
        "math",
        "mimetypes",
        "mmap",
        "modulefinder",
        "multiprocessing",
        "netrc",
        "nis",
        "nntplib",
        "numbers",
        "operator",
        "optparse",
        "os",
        "pathlib",
        "pdb",
        "pickle",
        "pickletools",
        "pipes",
        "pkgutil",
        "platform",
        "plistlib",
        "poplib",
        "posix",
        "posixpath",
        "pprint",
        "profile",
        "pstats",
        "pty",
        "pwd",
        "py_compile",
        "pyclbr",
        "pydoc",
        "queue",
        "quopri",
        "random",
        "re",
        "readline",
        "reprlib",
        "resource",
        "rlcompleter",
        "runpy",
        "sched",
        "secrets",
        "select",
        "selectors",
        "shelve",
        "shlex",
        "shutil",
        "signal",
        "site",
        "smtpd",
        "smtplib",
        "sndhdr",
        "socket",
        "socketserver",
        "sqlite3",
        "ssl",
        "stat",
        "statistics",
        "string",
        "stringprep",
        "struct",
        "subprocess",
        "sunau",
        "symtable",
        "sys",
        "sysconfig",
        "syslog",
        "tabnanny",
        "tarfile",
        "telnetlib",
        "tempfile",
        "termios",
        "test",
        "textwrap",
        "threading",
        "time",
        "timeit",
        "tkinter",
        "token",
        "tokenize",
        "tomllib",
        "trace",
        "traceback",
        "tracemalloc",
        "tty",
        "turtle",
        "turtledemo",
        "types",
        "typing",
        "unicodedata",
        "unittest",
        "urllib",
        "uu",
        "uuid",
        "venv",
        "warnings",
        "wave",
        "weakref",
        "webbrowser",
        "winreg",
        "winsound",
        "wsgiref",
        "xdrlib",
        "xml",
        "xmlrpc",
        "zipapp",
        "zipfile",
        "zipimport",
        "zlib",
        "zoneinfo",
        // Common typing modules
        "_typeshed",
        "typing_extensions",
        // Private/internal modules commonly seen
        "_thread",
        "_collections",
        "_abc",
        "_io",
        "_weakref",
        "__future__",
    ];

    // Get the base module name (first component before any dots)
    let base = module_name.split('.').next().unwrap_or(module_name);
    PYTHON_STDLIB.contains(&base)
}

/// Check if TypeScript/JavaScript import is a relative import.
///
/// Relative imports start with "./" or "../" and are internal.
/// Non-relative imports (like "express", "@types/node") are external.
pub fn is_typescript_relative(import_path: &str) -> bool {
    import_path.starts_with("./") || import_path.starts_with("../")
}

/// Check if TypeScript/JavaScript import is external (node_modules).
///
/// External imports don't start with . or / (relative paths).
/// Examples: "lodash", "@types/node", "express"
pub fn is_typescript_external(import_path: &str) -> bool {
    !import_path.starts_with('.') && !import_path.starts_with('/')
}

/// Check if Go import is stdlib.
///
/// Go stdlib packages don't contain dots in the base segment.
/// Examples: "fmt", "net/http", "encoding/json" are stdlib.
/// Examples: "github.com/gin-gonic/gin" is external.
pub fn is_go_stdlib(import_path: &str) -> bool {
    // Go stdlib: single-segment or known prefixes without dots
    // External packages always have dots (domain names)
    const GO_STDLIB_PREFIXES: &[&str] = &[
        "archive",
        "bufio",
        "builtin",
        "bytes",
        "cmp",
        "compress",
        "container",
        "context",
        "crypto",
        "database",
        "debug",
        "embed",
        "encoding",
        "errors",
        "expvar",
        "flag",
        "fmt",
        "go",
        "hash",
        "html",
        "image",
        "index",
        "internal",
        "io",
        "iter",
        "log",
        "maps",
        "math",
        "mime",
        "net",
        "os",
        "path",
        "plugin",
        "reflect",
        "regexp",
        "runtime",
        "slices",
        "sort",
        "strconv",
        "strings",
        "structs",
        "sync",
        "syscall",
        "testing",
        "text",
        "time",
        "unicode",
        "unsafe",
    ];

    let base = import_path.split('/').next().unwrap_or(import_path);

    // If the base contains a dot, it's likely a domain (external)
    if base.contains('.') {
        return false;
    }

    GO_STDLIB_PREFIXES.contains(&base)
}

/// Check if Rust import is stdlib.
///
/// Rust stdlib includes std::, core::, and alloc:: prefixes.
pub fn is_rust_stdlib(import_path: &str) -> bool {
    import_path.starts_with("std::")
        || import_path.starts_with("core::")
        || import_path.starts_with("alloc::")
        || import_path == "std"
        || import_path == "core"
        || import_path == "alloc"
}

/// Check if Rust import is internal (crate::, self::, super::).
pub fn is_rust_internal(import_path: &str) -> bool {
    import_path.starts_with("crate::")
        || import_path.starts_with("self::")
        || import_path.starts_with("super::")
}

/// Read the Go module path from a go.mod file in the given root directory.
///
/// Parses the `module` directive from go.mod. For example, given:
/// ```text
/// module github.com/spf13/cobra
///
/// go 1.15
/// ```
/// Returns `Some("github.com/spf13/cobra")`.
///
/// Returns `None` if go.mod doesn't exist or doesn't contain a module directive.
fn read_go_module_path(root: &Path) -> Option<String> {
    let go_mod_path = root.join("go.mod");
    let content = std::fs::read_to_string(go_mod_path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            let module_path = rest.trim();
            if !module_path.is_empty() {
                return Some(module_path.to_string());
            }
        }
    }
    None
}

/// Collect Go files grouped by their package directory (relative to root).
///
/// Returns a map from relative directory path to list of files in that directory.
/// Files at the root level use an empty string key.
fn group_go_files_by_package(root: &Path, files: &[PathBuf]) -> HashMap<String, Vec<PathBuf>> {
    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for file_path in files {
        if let Ok(relative) = file_path.strip_prefix(root) {
            let pkg_dir = relative
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            groups
                .entry(pkg_dir)
                .or_default()
                .push(file_path.clone());
        }
    }
    groups
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Convert a path to a Python module name (e.g., "src/utils" -> "src.utils")
fn path_to_module_name(path: &Path) -> String {
    path.to_string_lossy()
        .replace(['/', '\\'], ".")
        .trim_start_matches('.')
        .to_string()
}

/// Make a path relative to root, or return the path as-is if not under root
fn make_relative_path(path: &Path, root: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| path.to_path_buf())
}

/// Normalize a path (resolve . and ..)
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            c => result.push(c),
        }
    }
    result
}

/// Detect the dominant language in a directory by counting file extensions
fn detect_dominant_language(root: &Path) -> TldrResult<Language> {
    let mut counts: HashMap<Language, usize> = HashMap::new();

    // Simple extension-based detection
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() {
                if let Some(lang) = Language::from_path(&path) {
                    *counts.entry(lang).or_insert(0) += 1;
                }
            } else if path.is_dir() {
                // Check subdirectories one level deep
                if let Ok(sub_entries) = std::fs::read_dir(&path) {
                    for sub_entry in sub_entries.filter_map(|e| e.ok()) {
                        let sub_path = sub_entry.path();
                        if sub_path.is_file() {
                            if let Some(lang) = Language::from_path(&sub_path) {
                                *counts.entry(lang).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Return the most common language, defaulting to Python
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang)
        .ok_or_else(|| crate::error::TldrError::UnsupportedLanguage("unknown".to_string()))
}

/// Check if an error is recoverable (e.g., parse error in one file)
fn is_recoverable_error(err: &crate::error::TldrError) -> bool {
    matches!(err, crate::error::TldrError::ParseError { .. })
}


// =============================================================================
// Output Formatting (Phase 6)
// =============================================================================

/// Large graph warning threshold (S7-R43)
const LARGE_GRAPH_THRESHOLD: usize = 500;

/// Format dependency report as human-readable text.
///
/// Output format follows spec section 1.6:
/// ```text
/// Dependency Analysis: src/
/// Language: Python
///
/// Internal Dependencies (24 edges, 12 files):
///   src/auth.py
///     -> src/utils.py
///     -> src/db.py
///
/// External Packages (8):
///   jwt (1 import)
///   ...
///
/// Circular Dependencies Found: 1
///   [CYCLE] src/a.py -> src/b.py -> src/c.py -> src/a.py
///
/// Stats:
///   Max depth: 4
///   Leaf files: 3 (no outgoing deps)
///   Root files: 2 (no incoming deps)
/// ```
pub fn format_deps_text(report: &DepsReport) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Dependency Analysis: {}\n",
        report.root.display()
    ));
    output.push_str(&format!(
        "Language: {}\n\n",
        capitalize_first(&report.language)
    ));

    // Internal dependencies
    if !report.internal_dependencies.is_empty() {
        output.push_str(&format!(
            "Internal Dependencies ({} edges, {} files):\n",
            report.stats.total_internal_deps, report.stats.total_files
        ));
        for (file, deps) in &report.internal_dependencies {
            if !deps.is_empty() {
                output.push_str(&format!("  {}\n", file.display()));
                for dep in deps {
                    output.push_str(&format!("    -> {}\n", dep.display()));
                }
            }
        }
        output.push('\n');
    }

    // External packages
    if !report.external_dependencies.is_empty() {
        // Count imports per package
        let mut package_counts: std::collections::HashMap<&String, usize> =
            std::collections::HashMap::new();
        for deps in report.external_dependencies.values() {
            for dep in deps {
                *package_counts.entry(dep).or_insert(0) += 1;
            }
        }

        output.push_str(&format!(
            "External Packages ({}):\n",
            package_counts.len()
        ));
        // Sort by count descending, then alphabetically
        let mut packages: Vec<_> = package_counts.iter().collect();
        packages.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (pkg, count) in packages {
            let plural = if *count == 1 { "import" } else { "imports" };
            output.push_str(&format!("  {} ({} {})\n", pkg, count, plural));
        }
        output.push('\n');
    }

    // Circular dependencies
    if !report.circular_dependencies.is_empty() {
        output.push_str(&format!(
            "Circular Dependencies Found: {}\n",
            report.circular_dependencies.len()
        ));
        for cycle in &report.circular_dependencies {
            let cycle_str: Vec<String> =
                cycle.path.iter().map(|p| p.display().to_string()).collect();
            // Show cycle with closing loop
            let first = cycle
                .path
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            output.push_str(&format!("  [CYCLE] {} -> {}\n", cycle_str.join(" -> "), first));
        }
        output.push('\n');
    } else {
        output.push_str("No circular dependencies found.\n\n");
    }

    // Stats
    output.push_str("Stats:\n");
    output.push_str(&format!("  Max depth: {}\n", report.stats.max_depth));
    output.push_str(&format!(
        "  Leaf files: {} (no outgoing deps)\n",
        report.stats.leaf_files
    ));
    output.push_str(&format!(
        "  Root files: {} (no incoming deps)\n",
        report.stats.root_files
    ));

    output
}

/// Format dependency report as DOT graph for graphviz.
///
/// Risk mitigations:
/// - S7-R42: All node identifiers are quoted (paths may have special chars)
/// - S7-R43: Warns on large graphs (>500 nodes) via stderr
///
/// Output format follows spec section 1.6:
/// ```dot
/// digraph deps {
///   rankdir=LR;
///   node [shape=box];
///
///   // Nodes
///   "src/auth.py" [label="auth.py"];
///   "src/utils.py" [label="utils.py"];
///
///   // Edges
///   "src/auth.py" -> "src/utils.py";
///
///   // Cycles highlighted in red
///   "src/a.py" -> "src/b.py" [color=red];
/// }
/// ```
pub fn format_deps_dot(report: &DepsReport) -> String {
    let mut output = String::with_capacity(1024);

    // Count nodes for large graph warning (S7-R43)
    let mut nodes: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (file, deps) in &report.internal_dependencies {
        nodes.insert(file.display().to_string());
        for dep in deps {
            nodes.insert(dep.display().to_string());
        }
    }

    if nodes.len() > LARGE_GRAPH_THRESHOLD {
        eprintln!(
            "Warning: Large graph with {} nodes. Consider using --collapse-packages or filtering.",
            nodes.len()
        );
    }

    output.push_str("digraph deps {\n");
    output.push_str("  rankdir=LR;\n");
    output.push_str("  node [shape=box, fontname=\"Helvetica\"];\n");
    output.push_str("  edge [fontname=\"Helvetica\", fontsize=10];\n\n");

    // Output nodes with labels (S7-R42: quote all identifiers)
    output.push_str("  // Nodes\n");
    for node in &nodes {
        let label = std::path::Path::new(node)
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| node.into());
        // Escape quotes in node identifier and label
        let escaped_node = escape_dot_string(node);
        let escaped_label = escape_dot_string(&label);
        output.push_str(&format!(
            "  \"{}\" [label=\"{}\"];\n",
            escaped_node, escaped_label
        ));
    }
    output.push('\n');

    // Collect cycle edges for highlighting
    let mut cycle_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for cycle in &report.circular_dependencies {
        for i in 0..cycle.path.len() {
            let from = cycle.path[i].display().to_string();
            let to = cycle.path[(i + 1) % cycle.path.len()].display().to_string();
            cycle_edges.insert((from, to));
        }
    }

    // Output edges (S7-R42: quote all identifiers)
    output.push_str("  // Edges\n");
    for (file, deps) in &report.internal_dependencies {
        let from = file.display().to_string();
        let escaped_from = escape_dot_string(&from);
        for dep in deps {
            let to = dep.display().to_string();
            let escaped_to = escape_dot_string(&to);
            if cycle_edges.contains(&(from.clone(), to.clone())) {
                // Highlight cycle edges in red (spec)
                output.push_str(&format!(
                    "  \"{}\" -> \"{}\" [color=red, penwidth=2];\n",
                    escaped_from, escaped_to
                ));
            } else {
                output.push_str(&format!("  \"{}\" -> \"{}\";\n", escaped_from, escaped_to));
            }
        }
    }

    output.push_str("}\n");
    output
}

/// Escape special characters in DOT strings.
fn escape_dot_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deps_report_default() {
        let report = DepsReport::default();
        assert!(report.root.as_os_str().is_empty());
        assert!(report.language.is_empty());
        assert!(report.internal_dependencies.is_empty());
        assert!(report.external_dependencies.is_empty());
        assert!(report.circular_dependencies.is_empty());
        assert_eq!(report.stats.total_files, 0);
    }

    #[test]
    fn test_dep_node_hash_eq_by_path() {
        let node1 = DepNode::with_name(
            PathBuf::from("src/auth.py"),
            "auth".to_string(),
            DepKind::Internal,
        );
        let node2 = DepNode::with_name(
            PathBuf::from("src/auth.py"),
            "different_name".to_string(), // Different name, same path
            DepKind::External,             // Different kind, same path
        );
        let node3 = DepNode::with_name(
            PathBuf::from("src/utils.py"),
            "auth".to_string(), // Same name, different path
            DepKind::Internal,
        );

        // node1 and node2 should be equal (same path)
        assert_eq!(node1, node2);

        // node1 and node3 should not be equal (different path)
        assert_ne!(node1, node3);

        // Test hash consistency with equality
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(node1.clone());
        assert!(set.contains(&node2)); // Should find node2 via node1's path
        assert!(!set.contains(&node3)); // Should not find node3
    }

    #[test]
    fn test_dep_cycle_canonical() {
        // Cycle: A -> B -> C -> A
        let cycle1 = DepCycle::new(vec![
            PathBuf::from("a.py"),
            PathBuf::from("b.py"),
            PathBuf::from("c.py"),
        ]);

        // Same cycle starting from B: B -> C -> A -> B
        let cycle2 = DepCycle::new(vec![
            PathBuf::from("b.py"),
            PathBuf::from("c.py"),
            PathBuf::from("a.py"),
        ]);

        // Same cycle starting from C: C -> A -> B -> C
        let cycle3 = DepCycle::new(vec![
            PathBuf::from("c.py"),
            PathBuf::from("a.py"),
            PathBuf::from("b.py"),
        ]);

        // All should have the same canonical form
        let c1 = cycle1.canonical();
        let c2 = cycle2.canonical();
        let c3 = cycle3.canonical();

        assert_eq!(c1.path, c2.path);
        assert_eq!(c2.path, c3.path);

        // Canonical form should start with 'a.py' (lexicographically smallest)
        assert_eq!(c1.path[0], PathBuf::from("a.py"));
    }

    #[test]
    fn test_dep_cycle_eq_hash() {
        let cycle1 = DepCycle::new(vec![
            PathBuf::from("a.py"),
            PathBuf::from("b.py"),
            PathBuf::from("c.py"),
        ]);

        let cycle2 = DepCycle::new(vec![
            PathBuf::from("b.py"),
            PathBuf::from("c.py"),
            PathBuf::from("a.py"),
        ]);

        // Same cycle, different starting points - should be equal
        assert_eq!(cycle1, cycle2);

        // Should work in HashSet
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(cycle1);
        assert!(set.contains(&cycle2));
        assert_eq!(set.len(), 1); // Only one unique cycle
    }

    #[test]
    fn test_dep_kind_default() {
        assert_eq!(DepKind::default(), DepKind::Internal);
    }

    #[test]
    fn test_dep_stats_default() {
        let stats = DepStats::default();
        assert_eq!(stats.total_files, 0);
        assert_eq!(stats.total_internal_deps, 0);
        assert_eq!(stats.total_external_deps, 0);
        assert_eq!(stats.max_depth, 0);
        assert_eq!(stats.cycles_found, 0);
        assert_eq!(stats.leaf_files, 0);
        assert_eq!(stats.root_files, 0);
    }

    #[test]
    fn test_deps_options_builders() {
        let opts = DepsOptions::with_external();
        assert!(opts.include_external);

        let opts = DepsOptions::cycles_only();
        assert!(opts.show_cycles_only);

        let opts = DepsOptions::default()
            .with_max_cycle_length(5)
            .with_max_depth(3);
        assert_eq!(opts.max_cycle_length, Some(5));
        assert_eq!(opts.max_depth, Some(3));
    }

    #[test]
    fn test_dep_edge_constructors() {
        let edge = DepEdge::new(PathBuf::from("a.py"), PathBuf::from("b.py"));
        assert_eq!(edge.from, PathBuf::from("a.py"));
        assert_eq!(edge.to, PathBuf::from("b.py"));
        assert!(edge.line.is_none());
        assert!(edge.import_text.is_none());

        let edge = DepEdge::with_line(PathBuf::from("a.py"), PathBuf::from("b.py"), 10);
        assert_eq!(edge.line, Some(10));
        assert!(edge.import_text.is_none());

        let edge = DepEdge::with_details(
            PathBuf::from("a.py"),
            PathBuf::from("b.py"),
            10,
            "from b import func".to_string(),
        );
        assert_eq!(edge.line, Some(10));
        assert_eq!(edge.import_text, Some("from b import func".to_string()));
    }

    #[test]
    fn test_deps_report_serialization() {
        let mut report = DepsReport {
            root: PathBuf::from("src"),
            language: "python".to_string(),
            stats: DepStats {
                total_files: 2,
                total_internal_deps: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        report
            .internal_dependencies
            .insert(PathBuf::from("a.py"), vec![PathBuf::from("b.py")]);

        // Should serialize without errors
        let json = serde_json::to_string(&report).expect("serialization failed");
        assert!(json.contains("\"root\":\"src\""));
        assert!(json.contains("\"language\":\"python\""));

        // Should deserialize back
        let parsed: DepsReport = serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(parsed.root, PathBuf::from("src"));
        assert_eq!(parsed.language, "python");
    }

    #[test]
    fn test_btreemap_deterministic_order() {
        // Verify BTreeMap produces deterministic JSON output
        let mut deps1 = BTreeMap::new();
        deps1.insert(PathBuf::from("z.py"), vec![PathBuf::from("a.py")]);
        deps1.insert(PathBuf::from("a.py"), vec![PathBuf::from("b.py")]);
        deps1.insert(PathBuf::from("m.py"), vec![PathBuf::from("c.py")]);

        let mut deps2 = BTreeMap::new();
        // Insert in different order
        deps2.insert(PathBuf::from("m.py"), vec![PathBuf::from("c.py")]);
        deps2.insert(PathBuf::from("a.py"), vec![PathBuf::from("b.py")]);
        deps2.insert(PathBuf::from("z.py"), vec![PathBuf::from("a.py")]);

        let json1 = serde_json::to_string(&deps1).unwrap();
        let json2 = serde_json::to_string(&deps2).unwrap();

        // BTreeMap ensures same JSON regardless of insertion order
        assert_eq!(json1, json2);

        // Verify alphabetical order in JSON
        let a_pos = json1.find("a.py").unwrap();
        let m_pos = json1.find("m.py").unwrap();
        let z_pos = json1.find("z.py").unwrap();
        assert!(a_pos < m_pos);
        assert!(m_pos < z_pos);
    }

    // =========================================================================
    // C / C++ import resolver tests
    // =========================================================================

    #[test]
    fn test_build_module_index_c_header_files() {
        let root = PathBuf::from("/project");
        let files = vec![
            PathBuf::from("/project/src/utils.h"),
            PathBuf::from("/project/src/utils.c"),
            PathBuf::from("/project/include/config.h"),
            PathBuf::from("/project/src/net/socket.h"),
        ];
        let index = build_module_index(&root, &files, Language::C);

        // Header files should be indexed by their relative path
        assert!(index.contains_key("src/utils.h"));
        assert_eq!(index["src/utils.h"], PathBuf::from("/project/src/utils.h"));

        // Also indexed by filename only
        assert!(index.contains_key("utils.h"));

        // Nested headers
        assert!(index.contains_key("include/config.h"));
        assert!(index.contains_key("config.h"));
        assert!(index.contains_key("src/net/socket.h"));
        assert!(index.contains_key("net/socket.h"));
        assert!(index.contains_key("socket.h"));
    }

    #[test]
    fn test_build_module_index_cpp_header_files() {
        let root = PathBuf::from("/project");
        let files = vec![
            PathBuf::from("/project/include/widget.hpp"),
            PathBuf::from("/project/src/widget.cpp"),
        ];
        let index = build_module_index(&root, &files, Language::Cpp);

        assert!(index.contains_key("include/widget.hpp"));
        assert!(index.contains_key("widget.hpp"));
    }

    #[test]
    fn test_resolve_c_local_include() {
        let mut index = HashMap::new();
        index.insert(
            "src/utils.h".to_string(),
            PathBuf::from("/project/src/utils.h"),
        );
        index.insert(
            "utils.h".to_string(),
            PathBuf::from("/project/src/utils.h"),
        );

        let import = ImportInfo {
            module: "utils.h".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_c_cpp_import(
            &import,
            Path::new("/project"),
            Path::new("/project/src/main.c"),
            &index,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap(), PathBuf::from("/project/src/utils.h"));
    }

    #[test]
    fn test_resolve_c_system_include_returns_none() {
        let index = HashMap::new();
        let import = ImportInfo {
            module: "stdio.h".to_string(),
            names: Vec::new(),
            is_from: true,
            alias: None,
        };
        let result = resolve_c_cpp_import(
            &import,
            Path::new("/project"),
            Path::new("/project/src/main.c"),
            &index,
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_c_relative_path_include() {
        let mut index = HashMap::new();
        index.insert(
            "net/socket.h".to_string(),
            PathBuf::from("/project/src/net/socket.h"),
        );
        index.insert(
            "src/net/socket.h".to_string(),
            PathBuf::from("/project/src/net/socket.h"),
        );

        let import = ImportInfo {
            module: "net/socket.h".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_c_cpp_import(
            &import,
            Path::new("/project"),
            Path::new("/project/src/main.c"),
            &index,
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/project/src/net/socket.h")
        );
    }

    // =========================================================================
    // Ruby import resolver tests
    // =========================================================================

    #[test]
    fn test_build_module_index_ruby() {
        let root = PathBuf::from("/project");
        let files = vec![
            PathBuf::from("/project/lib/devise/models.rb"),
            PathBuf::from("/project/lib/utils.rb"),
            PathBuf::from("/project/app/models/user.rb"),
        ];
        let index = build_module_index(&root, &files, Language::Ruby);

        assert!(index.contains_key("devise/models"));
        assert_eq!(
            index["devise/models"],
            PathBuf::from("/project/lib/devise/models.rb")
        );
        assert!(index.contains_key("lib/devise/models"));
        assert!(index.contains_key("utils"));
    }

    #[test]
    fn test_resolve_ruby_require() {
        let mut index = HashMap::new();
        index.insert(
            "devise/models".to_string(),
            PathBuf::from("/project/lib/devise/models.rb"),
        );

        let import = ImportInfo {
            module: "devise/models".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_ruby_import(
            &import,
            Path::new("/project"),
            Path::new("/project/app/main.rb"),
            &index,
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/project/lib/devise/models.rb")
        );
    }

    #[test]
    fn test_resolve_ruby_require_relative() {
        let mut index = HashMap::new();
        index.insert(
            "utils".to_string(),
            PathBuf::from("/project/lib/utils.rb"),
        );

        let import = ImportInfo {
            module: "utils".to_string(),
            names: Vec::new(),
            is_from: true,
            alias: None,
        };
        let result = resolve_ruby_import(
            &import,
            Path::new("/project"),
            Path::new("/project/lib/main.rb"),
            &index,
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap(), PathBuf::from("/project/lib/utils.rb"));
    }

    // =========================================================================
    // C# import resolver tests
    // =========================================================================

    #[test]
    fn test_build_module_index_csharp() {
        let root = PathBuf::from("/project");
        let files = vec![
            PathBuf::from("/project/Newtonsoft/Json/JsonConvert.cs"),
            PathBuf::from("/project/MyApp/Models/User.cs"),
            PathBuf::from("/project/MyApp/Services/AuthService.cs"),
        ];
        let index = build_module_index(&root, &files, Language::CSharp);

        assert!(index.contains_key("Newtonsoft.Json.JsonConvert"));
        assert!(index.contains_key("MyApp.Models.User"));
        assert!(index.contains_key("MyApp.Services.AuthService"));
        assert!(index.contains_key("Newtonsoft.Json"));
        assert!(index.contains_key("MyApp.Models"));
    }

    #[test]
    fn test_resolve_csharp_using() {
        let mut index = HashMap::new();
        index.insert(
            "MyApp.Models".to_string(),
            PathBuf::from("/project/MyApp/Models/User.cs"),
        );

        let import = ImportInfo {
            module: "MyApp.Models".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_csharp_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Program.cs"),
            &index,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_csharp_system_namespace_returns_none() {
        let index = HashMap::new();
        let import = ImportInfo {
            module: "System.Collections.Generic".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_csharp_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Program.cs"),
            &index,
        );
        assert!(result.is_none());
    }

    // =========================================================================
    // Scala import resolver tests
    // =========================================================================

    #[test]
    fn test_build_module_index_scala() {
        let root = PathBuf::from("/project");
        let files = vec![
            PathBuf::from("/project/cats/Functor.scala"),
            PathBuf::from("/project/myapp/models/User.scala"),
            PathBuf::from("/project/myapp/services/Auth.scala"),
        ];
        let index = build_module_index(&root, &files, Language::Scala);

        assert!(index.contains_key("cats.Functor"));
        assert!(index.contains_key("myapp.models.User"));
        assert!(index.contains_key("myapp.services.Auth"));
        assert!(index.contains_key("myapp.models"));
        assert!(index.contains_key("myapp.services"));
    }

    #[test]
    fn test_resolve_scala_simple_import() {
        let mut index = HashMap::new();
        index.insert(
            "cats.Functor".to_string(),
            PathBuf::from("/project/cats/Functor.scala"),
        );

        let import = ImportInfo {
            module: "cats.Functor".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_scala_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Main.scala"),
            &index,
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            PathBuf::from("/project/cats/Functor.scala")
        );
    }

    #[test]
    fn test_resolve_scala_wildcard_import() {
        let mut index = HashMap::new();
        index.insert(
            "myapp.models".to_string(),
            PathBuf::from("/project/myapp/models/User.scala"),
        );

        let import = ImportInfo {
            module: "myapp.models".to_string(),
            names: vec!["*".to_string()],
            is_from: true,
            alias: None,
        };
        let result = resolve_scala_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Main.scala"),
            &index,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_scala_stdlib_returns_none() {
        let index = HashMap::new();
        let import = ImportInfo {
            module: "scala.util.Try".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_scala_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Main.scala"),
            &index,
        );
        assert!(result.is_none());
    }

    // =========================================================================
    // resolve_import integration tests for new languages
    // =========================================================================

    #[test]
    fn test_resolve_import_dispatches_c() {
        let mut index = HashMap::new();
        index.insert(
            "utils.h".to_string(),
            PathBuf::from("/project/src/utils.h"),
        );

        let import = ImportInfo {
            module: "utils.h".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_import(
            &import,
            Path::new("/project"),
            Path::new("/project/src/main.c"),
            &index,
            Language::C,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_import_dispatches_cpp() {
        let mut index = HashMap::new();
        index.insert(
            "widget.hpp".to_string(),
            PathBuf::from("/project/include/widget.hpp"),
        );

        let import = ImportInfo {
            module: "widget.hpp".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_import(
            &import,
            Path::new("/project"),
            Path::new("/project/src/main.cpp"),
            &index,
            Language::Cpp,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_import_dispatches_ruby() {
        let mut index = HashMap::new();
        index.insert(
            "utils".to_string(),
            PathBuf::from("/project/lib/utils.rb"),
        );

        let import = ImportInfo {
            module: "utils".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_import(
            &import,
            Path::new("/project"),
            Path::new("/project/app/main.rb"),
            &index,
            Language::Ruby,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_import_dispatches_csharp() {
        let mut index = HashMap::new();
        index.insert(
            "MyApp.Models".to_string(),
            PathBuf::from("/project/MyApp/Models/User.cs"),
        );

        let import = ImportInfo {
            module: "MyApp.Models".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Program.cs"),
            &index,
            Language::CSharp,
        );
        assert!(result.is_some());
    }

    #[test]
    fn test_resolve_import_dispatches_scala() {
        let mut index = HashMap::new();
        index.insert(
            "cats.Functor".to_string(),
            PathBuf::from("/project/cats/Functor.scala"),
        );

        let import = ImportInfo {
            module: "cats.Functor".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };
        let result = resolve_import(
            &import,
            Path::new("/project"),
            Path::new("/project/Main.scala"),
            &index,
            Language::Scala,
        );
        assert!(result.is_some());
    }
}
