//! Architecture Rules Engine (Phase 3)
//!
//! This module implements `--generate-rules` and `--check-rules` for architecture validation.
//!
//! # Features
//!
//! - **Rule Generation**: Generate architecture rules from detected layer structure
//!   - L1, L2: Layer constraint rules (e.g., "LOW may not import HIGH")
//!   - C1, C2, ...: Cycle break rules from detected circular dependencies
//!
//! - **Rule Checking**: Validate code against architecture rules
//!   - Build import graph (NOT call graph - addresses A22)
//!   - Check each import edge against layer rules
//!   - Report violations with file/line information
//!
//! # Mitigations
//!
//! - A8: Missing fields for rule generation - uses RulesGenerationContext
//! - A22: Import graph vs call graph - builds separate import graph for rules checking
//!
//! # References
//!
//! - Spec: architecture-spec.md Section 1.2, 1.3
//! - Plan: architecture-phased-plan.yaml Phase 3
//! - Premortems: architecture-premortem-1.yaml (A8), architecture-premortem-2.yaml (A22)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::imports::get_imports;
use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{
    ArchRule, ArchRuleType, ArchRulesFile, ArchitectureReport, IgnoreSpec, ImportInfo, Language,
    LayerDefinition, LayerDefinitions, LayerType, RulesGenerationContext, Violation,
    ViolationInfo, ViolationReport,
};
use crate::TldrResult;

// =============================================================================
// Import Graph Types (A22)
// =============================================================================

/// An edge in the import graph (file A imports file B)
#[derive(Debug, Clone)]
pub struct ImportEdge {
    /// File containing the import statement
    pub from_file: PathBuf,
    /// File being imported
    pub to_file: PathBuf,
    /// Module name as written in the import
    pub module: String,
    /// Line number of the import statement
    pub line: u32,
}

/// Import graph for a project
///
/// This is distinct from the call graph. The call graph tracks function calls,
/// while the import graph tracks import statements at the file level.
/// Layer violations are about import dependencies, not call patterns.
#[derive(Debug, Clone, Default)]
pub struct ImportGraph {
    /// All import edges
    pub edges: Vec<ImportEdge>,
    /// Map from file to its imports (for fast lookup)
    pub file_to_imports: HashMap<PathBuf, Vec<ImportEdge>>,
    /// All files in the project
    pub files: HashSet<PathBuf>,
}

impl ImportGraph {
    /// Create a new empty import graph
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an import edge
    pub fn add_edge(&mut self, edge: ImportEdge) {
        self.files.insert(edge.from_file.clone());
        self.files.insert(edge.to_file.clone());
        self.file_to_imports
            .entry(edge.from_file.clone())
            .or_default()
            .push(edge.clone());
        self.edges.push(edge);
    }
}

// =============================================================================
// Import Graph Builder (A22)
// =============================================================================

/// Build an import graph from a project directory.
///
/// This builds a file-level import graph (which file imports which file),
/// NOT a call graph (which function calls which function).
///
/// # Arguments
///
/// * `root` - Project root directory
/// * `language` - Programming language to analyze
///
/// # Returns
///
/// An `ImportGraph` with all import edges between project files.
///
/// # A22 Mitigation
///
/// This function specifically builds an IMPORT graph, not a call graph.
/// Layer rules should be checked against imports, not calls.
pub fn build_import_graph(root: &Path, language: Language) -> TldrResult<ImportGraph> {
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let tree = get_file_tree(root, Some(&extensions), true, Some(&IgnoreSpec::default()))?;
    let files = collect_files(&tree, root);

    let mut graph = ImportGraph::new();

    // First, collect all files to enable import resolution
    let mut all_files: HashSet<PathBuf> = HashSet::new();
    for file_path in &files {
        all_files.insert(file_path.clone());
    }

    // Then, extract imports from each file
    for file_path in &files {
        graph.files.insert(file_path.clone());

        match get_imports(file_path, language) {
            Ok(imports) => {
                for import in imports {
                    // Try to resolve the import to a project file
                    if let Some(resolved) =
                        resolve_import(&import, file_path, root, &all_files, language)
                    {
                        graph.add_edge(ImportEdge {
                            from_file: file_path.clone(),
                            to_file: resolved,
                            module: import.module.clone(),
                            line: 1, // We don't have precise line info from get_imports
                        });
                    }
                }
            }
            Err(e) => {
                // Skip files with parse errors (non-fatal)
                if e.is_recoverable() {
                    continue;
                }
            }
        }
    }

    Ok(graph)
}

/// Resolve an import to a project file path.
///
/// Returns `None` if the import is external (third-party or stdlib).
fn resolve_import(
    import: &ImportInfo,
    from_file: &Path,
    project_root: &Path,
    all_files: &HashSet<PathBuf>,
    language: Language,
) -> Option<PathBuf> {
    match language {
        Language::Python => resolve_python_import(import, from_file, project_root, all_files),
        Language::TypeScript | Language::JavaScript => {
            resolve_ts_import(import, from_file, project_root, all_files)
        }
        Language::Go => resolve_go_import(import, all_files),
        Language::Rust => resolve_rust_import(import, from_file, project_root, all_files),
        _ => None,
    }
}

/// Resolve a Python import to a file path.
fn resolve_python_import(
    import: &ImportInfo,
    from_file: &Path,
    project_root: &Path,
    all_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Handle relative imports (leading dots)
    let (dots, module_path) = count_leading_dots(module);

    if dots > 0 {
        // Relative import
        let from_dir = from_file.parent()?;
        let mut base_dir = from_dir.to_path_buf();

        // Go up `dots - 1` directories (1 dot = current dir, 2 dots = parent, etc.)
        for _ in 0..(dots.saturating_sub(1)) {
            base_dir = base_dir.parent()?.to_path_buf();
        }

        // Convert module path to file path
        let relative_path = module_path.replace('.', "/");

        // Try module.py first, then module/__init__.py
        let candidate = base_dir.join(format!("{}.py", relative_path));
        if all_files.contains(&candidate) {
            return Some(candidate);
        }

        let candidate = base_dir.join(&relative_path).join("__init__.py");
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
    } else {
        // Absolute import
        let relative_path = module.replace('.', "/");

        // Try module.py first
        let candidate = project_root.join(format!("{}.py", relative_path));
        if all_files.contains(&candidate) {
            return Some(candidate);
        }

        // Try module/__init__.py
        let candidate = project_root.join(&relative_path).join("__init__.py");
        if all_files.contains(&candidate) {
            return Some(candidate);
        }

        // Try src/module.py pattern
        let candidate = project_root
            .join("src")
            .join(format!("{}.py", relative_path));
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    None // External import
}

/// Count leading dots in a Python import module name.
fn count_leading_dots(module: &str) -> (usize, &str) {
    let dots = module.chars().take_while(|&c| c == '.').count();
    let rest = &module[dots..];
    (dots, rest)
}

/// Resolve a TypeScript/JavaScript import to a file path.
fn resolve_ts_import(
    import: &ImportInfo,
    from_file: &Path,
    project_root: &Path,
    all_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Skip node_modules imports
    if !module.starts_with('.') && !module.starts_with('/') {
        // Could be a local alias, but likely external
        return None;
    }

    let from_dir = from_file.parent()?;
    let base_dir = if module.starts_with('.') {
        from_dir.to_path_buf()
    } else {
        project_root.to_path_buf()
    };

    let clean_path = module.trim_start_matches("./").trim_start_matches("../");
    let path_part = if module.starts_with("../") {
        let ups = module.matches("../").count();
        let mut dir = base_dir.clone();
        for _ in 0..ups {
            dir = dir.parent()?.to_path_buf();
        }
        dir.join(clean_path)
    } else {
        base_dir.join(clean_path)
    };

    // Try various extensions
    for ext in &[
        "",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        "/index.ts",
        "/index.tsx",
        "/index.js",
    ] {
        let candidate = PathBuf::from(format!("{}{}", path_part.display(), ext));
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
    }

    None
}

/// Resolve a Go import to a file path.
fn resolve_go_import(
    import: &ImportInfo,
    all_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    // Go imports are package paths; we look for the package directory
    // This is simplified - full Go import resolution requires go.mod parsing

    let module = &import.module;
    let parts: Vec<&str> = module.split('/').collect();

    // Try to find a matching directory with .go files
    if let Some(last) = parts.last() {
        // Look for files in a directory matching the last path component
        for file in all_files {
            if let Some(parent) = file.parent() {
                if parent.ends_with(last) && file.extension().map(|e| e == "go").unwrap_or(false) {
                    return Some(file.clone());
                }
            }
        }
    }

    None
}

/// Resolve a Rust import to a file path.
fn resolve_rust_import(
    import: &ImportInfo,
    from_file: &Path,
    project_root: &Path,
    all_files: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let module = &import.module;

    // Rust uses :: for paths
    let path_parts: Vec<&str> = module.split("::").collect();

    // Skip crate:: prefix or external crates
    let start_idx = if matches!(path_parts.first(), Some(&"crate") | Some(&"self")) {
        1
    } else if path_parts.first() == Some(&"super") {
        // Handle super:: prefix
        let from_dir = from_file.parent()?;
        let parent = from_dir.parent()?;
        let rest_path = path_parts[1..].join("/");
        let candidate = parent.join(format!("{}.rs", rest_path));
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
        let candidate = parent.join(&rest_path).join("mod.rs");
        if all_files.contains(&candidate) {
            return Some(candidate);
        }
        return None;
    } else {
        // Likely external crate
        return None;
    };

    // Build relative path
    let relative_path = path_parts[start_idx..].join("/");

    // Try src/relative_path.rs
    let candidate = project_root
        .join("src")
        .join(format!("{}.rs", relative_path));
    if all_files.contains(&candidate) {
        return Some(candidate);
    }

    // Try src/relative_path/mod.rs
    let candidate = project_root.join("src").join(&relative_path).join("mod.rs");
    if all_files.contains(&candidate) {
        return Some(candidate);
    }

    None
}

// =============================================================================
// Rule Generation
// =============================================================================

/// Generate architecture rules from an architecture report.
///
/// This takes the detected layers and circular dependencies and generates
/// a set of rules that can be used to validate the architecture.
///
/// # Generated Rules
///
/// - **Layer Rules (L1, L2, ...):**
///   - LOW may not import HIGH
///   - MIDDLE may not import HIGH
///   - Custom rules based on detected layer violations
///
/// - **Cycle Break Rules (C1, C2, ...):**
///   - One rule per detected circular dependency
///   - Suggests which edge to break
///
/// # Arguments
///
/// * `arch_report` - Architecture analysis report with detected layers
/// * `context` - Additional context for rule generation
///
/// # Returns
///
/// An `ArchRulesFile` that can be serialized to YAML/JSON.
pub fn generate_rules(
    arch_report: &ArchitectureReport,
    context: &RulesGenerationContext,
) -> ArchRulesFile {
    let mut rules_file = ArchRulesFile::new();

    // Set timestamp using std::time
    let now = std::time::SystemTime::now();
    let timestamp = now
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string());
    rules_file.generated_at = Some(timestamp);

    // Build layer definitions from detected layers
    rules_file.layers = build_layer_definitions(&arch_report.inferred_layers);

    // Generate layer constraint rules
    let layer_rules = generate_layer_rules(&arch_report.inferred_layers);
    for rule in layer_rules {
        rules_file.rules.push(rule);
    }

    // Generate cycle break rules from circular dependencies
    let cycle_rules = generate_cycle_rules(&arch_report.circular_dependencies, context);
    for rule in cycle_rules {
        rules_file.rules.push(rule);
    }

    rules_file
}

/// Build layer definitions from inferred layers.
fn build_layer_definitions(inferred_layers: &HashMap<PathBuf, LayerType>) -> LayerDefinitions {
    let mut high_dirs: Vec<String> = Vec::new();
    let mut middle_dirs: Vec<String> = Vec::new();
    let mut low_dirs: Vec<String> = Vec::new();

    for (dir, layer) in inferred_layers {
        let dir_str = format!("{}/", dir.display());
        match layer {
            LayerType::Entry | LayerType::DynamicDispatch => {
                high_dirs.push(dir_str);
            }
            LayerType::Service => {
                middle_dirs.push(dir_str);
            }
            LayerType::Utility => {
                low_dirs.push(dir_str);
            }
        }
    }

    // Sort for deterministic output
    high_dirs.sort();
    middle_dirs.sort();
    low_dirs.sort();

    LayerDefinitions {
        high: if high_dirs.is_empty() {
            None
        } else {
            Some(LayerDefinition::new(
                "Entry/Controller layer - handles external requests",
                high_dirs,
            ))
        },
        middle: if middle_dirs.is_empty() {
            None
        } else {
            Some(LayerDefinition::new(
                "Service/Business layer - core logic",
                middle_dirs,
            ))
        },
        low: if low_dirs.is_empty() {
            None
        } else {
            Some(LayerDefinition::new(
                "Utility/Data layer - shared utilities",
                low_dirs,
            ))
        },
    }
}

/// Generate layer constraint rules.
///
/// Standard rules:
/// - L1: LOW may not import HIGH
/// - L2: MIDDLE may not import HIGH
fn generate_layer_rules(inferred_layers: &HashMap<PathBuf, LayerType>) -> Vec<ArchRule> {
    let mut rules = Vec::new();

    // Check if we have layers to generate rules for
    let has_high = inferred_layers
        .values()
        .any(|l| matches!(l, LayerType::Entry | LayerType::DynamicDispatch));
    let has_middle = inferred_layers
        .values()
        .any(|l| matches!(l, LayerType::Service));
    let has_low = inferred_layers
        .values()
        .any(|l| matches!(l, LayerType::Utility));

    // L1: LOW may not import HIGH
    if has_low && has_high {
        rules.push(ArchRule::layer(
            "L1",
            "LOW may not import HIGH",
            vec!["LOW".to_string()],
            vec!["HIGH".to_string()],
            "Utility layers should not depend on entry layers",
        ));
    }

    // L2: MIDDLE may not import HIGH
    if has_middle && has_high {
        rules.push(ArchRule::layer(
            "L2",
            "MIDDLE may not import HIGH",
            vec!["MIDDLE".to_string()],
            vec!["HIGH".to_string()],
            "Service layers should not depend on entry layers",
        ));
    }

    rules
}

/// Generate cycle break rules from circular dependencies.
fn generate_cycle_rules(
    circular_deps: &[crate::types::CircularDep],
    _context: &RulesGenerationContext,
) -> Vec<ArchRule> {
    let mut rules = Vec::new();

    for (i, dep) in circular_deps.iter().enumerate() {
        let id = format!("C{}", i + 1);
        let constraint = format!(
            "Break cycle: {} should not import {}",
            dep.a.display(),
            dep.b.display()
        );
        let files = vec![
            dep.a.to_string_lossy().to_string(),
            dep.b.to_string_lossy().to_string(),
        ];

        rules.push(ArchRule::cycle_break(
            id,
            constraint,
            files,
            format!(
                "Circular dependency between {} and {}",
                dep.a.display(),
                dep.b.display()
            ),
        ));
    }

    rules
}

// =============================================================================
// Rule Checking
// =============================================================================

/// Check project code against architecture rules.
///
/// This builds an import graph and checks each import edge against the rules.
///
/// # Arguments
///
/// * `rules` - Architecture rules to check against
/// * `import_graph` - Import graph of the project (use `build_import_graph`)
/// * `layers` - File-to-layer mapping
///
/// # Returns
///
/// A `ViolationReport` with any violations found.
pub fn check_rules(
    rules: &ArchRulesFile,
    import_graph: &ImportGraph,
    layers: &HashMap<PathBuf, LayerType>,
) -> ViolationReport {
    let mut report = ViolationReport::new();
    report.summary.rules_checked = rules.rules.len();
    report.summary.files_scanned = import_graph.files.len();

    // Pre-compute file-to-layer mapping with canonical names
    let file_layers = compute_file_layers(layers, rules);

    // Check each import edge against layer rules
    for edge in &import_graph.edges {
        let from_layer = get_file_layer(&edge.from_file, &file_layers);
        let to_layer = get_file_layer(&edge.to_file, &file_layers);

        // Only check if both files are in known layers
        if let (Some(from), Some(to)) = (from_layer, to_layer) {
            for rule in &rules.rules {
                if let Some(violation) = check_layer_rule(rule, edge, from, to) {
                    report.add_violation(violation);
                }
            }
        }
    }

    // Check cycle break rules
    for rule in &rules.rules {
        if rule.rule_type == ArchRuleType::CycleBreak {
            check_cycle_rule(rule, import_graph, &mut report);
        }
    }

    report
}

/// Compute file-to-layer mapping with canonical layer names (HIGH, MIDDLE, LOW).
fn compute_file_layers(
    layers: &HashMap<PathBuf, LayerType>,
    rules: &ArchRulesFile,
) -> HashMap<PathBuf, String> {
    let mut file_layers: HashMap<PathBuf, String> = HashMap::new();

    for (dir, layer_type) in layers {
        let layer_name = match layer_type {
            LayerType::Entry | LayerType::DynamicDispatch => "HIGH",
            LayerType::Service => "MIDDLE",
            LayerType::Utility => "LOW",
        };

        // Mark all files in this directory as belonging to this layer
        // We use the directory itself as a prefix match
        file_layers.insert(dir.clone(), layer_name.to_string());
    }

    // Also add files based on layer definitions in rules
    if let Some(high) = &rules.layers.high {
        for dir in &high.directories {
            let dir_path = PathBuf::from(dir.trim_end_matches('/'));
            file_layers.insert(dir_path, "HIGH".to_string());
        }
    }
    if let Some(middle) = &rules.layers.middle {
        for dir in &middle.directories {
            let dir_path = PathBuf::from(dir.trim_end_matches('/'));
            file_layers.insert(dir_path, "MIDDLE".to_string());
        }
    }
    if let Some(low) = &rules.layers.low {
        for dir in &low.directories {
            let dir_path = PathBuf::from(dir.trim_end_matches('/'));
            file_layers.insert(dir_path, "LOW".to_string());
        }
    }

    file_layers
}

/// Get the layer for a file based on its directory.
fn get_file_layer<'a>(
    file: &Path,
    file_layers: &'a HashMap<PathBuf, String>,
) -> Option<&'a String> {
    // Check the file's parent directories
    let mut current = file.parent();
    while let Some(dir) = current {
        if let Some(layer) = file_layers.get(dir) {
            return Some(layer);
        }
        current = dir.parent();
    }

    // Also check if the file path starts with any known layer directory
    for (layer_dir, layer) in file_layers {
        if file.starts_with(layer_dir) {
            return Some(layer);
        }
    }

    None
}

/// Check a single import edge against a layer rule.
fn check_layer_rule(
    rule: &ArchRule,
    edge: &ImportEdge,
    from_layer: &String,
    to_layer: &String,
) -> Option<Violation> {
    if rule.rule_type != ArchRuleType::Layer {
        return None;
    }

    // Check if this import violates the rule
    // Rule: from_layers may not import to_layers
    let from_matches = rule.from_layers.iter().any(|l| l == from_layer);
    let to_matches = rule.to_layers.iter().any(|l| l == to_layer);

    if from_matches && to_matches {
        Some(Violation::direct(ViolationInfo {
            rule_id: rule.id.clone(),
            rule_constraint: rule.constraint.clone(),
            from_file: edge.from_file.clone(),
            from_line: edge.line,
            imports_file: edge.to_file.clone(),
            from_layer: from_layer.clone(),
            to_layer: to_layer.clone(),
            severity: rule.severity,
        }))
    } else {
        None
    }
}

/// Check cycle break rules against the import graph.
fn check_cycle_rule(rule: &ArchRule, import_graph: &ImportGraph, report: &mut ViolationReport) {
    if rule.files.len() < 2 {
        return;
    }

    // Check if any of the files import each other
    let files: HashSet<&str> = rule.files.iter().map(|s| s.as_str()).collect();

    for edge in &import_graph.edges {
        let from_str = edge.from_file.to_string_lossy();
        let to_str = edge.to_file.to_string_lossy();

        // Check if this edge is part of the cycle we're trying to break
        let from_in_rule = files.iter().any(|f| from_str.ends_with(*f));
        let to_in_rule = files.iter().any(|f| to_str.ends_with(*f));

        if from_in_rule && to_in_rule {
            report.add_violation(Violation::direct(ViolationInfo {
                rule_id: rule.id.clone(),
                rule_constraint: rule.constraint.clone(),
                from_file: edge.from_file.clone(),
                from_line: edge.line,
                imports_file: edge.to_file.clone(),
                from_layer: "CYCLE".to_string(),
                to_layer: "CYCLE".to_string(),
                severity: rule.severity,
            }));
        }
    }
}

/// Check for transitive violations.
///
/// A transitive violation occurs when A imports B, B imports C,
/// and the chain A -> ... -> C violates a layer rule.
///
/// # Arguments
///
/// * `rules` - Architecture rules
/// * `import_graph` - Import graph
/// * `file_layers` - File-to-layer mapping
///
/// # Returns
///
/// Vector of transitive violations found.
pub fn check_transitive_violations(
    rules: &ArchRulesFile,
    import_graph: &ImportGraph,
    layers: &HashMap<PathBuf, LayerType>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let file_layers = compute_file_layers(layers, rules);

    // Build adjacency list for BFS
    let mut adjacency: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for edge in &import_graph.edges {
        adjacency
            .entry(edge.from_file.clone())
            .or_default()
            .push(edge.to_file.clone());
    }

    // For each file in a "from" layer, find all reachable files and check for violations
    for rule in &rules.rules {
        if rule.rule_type != ArchRuleType::Layer {
            continue;
        }

        // Find all files in "from" layers
        for (dir, layer_name) in &file_layers {
            if !rule.from_layers.contains(layer_name) {
                continue;
            }

            // BFS from this directory to find transitive imports
            for start_file in import_graph.files.iter() {
                if !start_file.starts_with(dir) {
                    continue;
                }

                // BFS to find all reachable files
                let mut visited: HashSet<PathBuf> = HashSet::new();
                let mut queue: Vec<(PathBuf, Vec<PathBuf>)> =
                    vec![(start_file.clone(), vec![start_file.clone()])];

                while let Some((current, path)) = queue.pop() {
                    if visited.contains(&current) {
                        continue;
                    }
                    visited.insert(current.clone());

                    // Check if this file is in a forbidden "to" layer
                    if let Some(current_layer) = get_file_layer(&current, &file_layers) {
                        if rule.to_layers.contains(current_layer) && path.len() > 2 {
                            // Transitive violation found
                            violations.push(Violation::transitive(
                                ViolationInfo {
                                    rule_id: rule.id.clone(),
                                    rule_constraint: rule.constraint.clone(),
                                    from_file: start_file.clone(),
                                    from_line: 1,
                                    imports_file: current.clone(),
                                    from_layer: layer_name.clone(),
                                    to_layer: current_layer.clone(),
                                    severity: rule.severity,
                                },
                                path.clone(),
                            ));
                        }
                    }

                    // Continue BFS
                    if let Some(neighbors) = adjacency.get(&current) {
                        for neighbor in neighbors {
                            if !visited.contains(neighbor) {
                                let mut new_path = path.clone();
                                new_path.push(neighbor.clone());
                                queue.push((neighbor.clone(), new_path));
                            }
                        }
                    }
                }
            }
        }
    }

    violations
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CircularDep, RuleSeverity};

    // -------------------------------------------------------------------------
    // Rule Generation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn generate_rules_yaml_format() {
        let mut inferred_layers = HashMap::new();
        inferred_layers.insert(PathBuf::from("api"), LayerType::Entry);
        inferred_layers.insert(PathBuf::from("services"), LayerType::Service);
        inferred_layers.insert(PathBuf::from("utils"), LayerType::Utility);

        let arch_report = ArchitectureReport {
            entry_layer: Vec::new(),
            middle_layer: Vec::new(),
            leaf_layer: Vec::new(),
            directories: HashMap::new(),
            circular_dependencies: Vec::new(),
            inferred_layers,
        };

        let context = RulesGenerationContext::new(PathBuf::from("."));
        let rules = generate_rules(&arch_report, &context);

        // Check version
        assert_eq!(rules.version, "1.0");

        // Check timestamp exists
        assert!(rules.generated_at.is_some());

        // Check layers
        assert!(rules.layers.high.is_some());
        assert!(rules.layers.middle.is_some());
        assert!(rules.layers.low.is_some());

        // Check rules
        assert!(!rules.rules.is_empty());
    }

    #[test]
    fn generate_rules_includes_layer_constraints() {
        let mut inferred_layers = HashMap::new();
        inferred_layers.insert(PathBuf::from("api"), LayerType::Entry);
        inferred_layers.insert(PathBuf::from("services"), LayerType::Service);
        inferred_layers.insert(PathBuf::from("utils"), LayerType::Utility);

        let arch_report = ArchitectureReport {
            entry_layer: Vec::new(),
            middle_layer: Vec::new(),
            leaf_layer: Vec::new(),
            directories: HashMap::new(),
            circular_dependencies: Vec::new(),
            inferred_layers,
        };

        let context = RulesGenerationContext::new(PathBuf::from("."));
        let rules = generate_rules(&arch_report, &context);

        // Should have L1 and L2 rules
        let l1 = rules.rules.iter().find(|r| r.id == "L1");
        assert!(l1.is_some(), "Expected L1 rule");

        let l1 = l1.unwrap();
        assert_eq!(l1.from_layers, vec!["LOW"]);
        assert_eq!(l1.to_layers, vec!["HIGH"]);
        assert_eq!(l1.rule_type, ArchRuleType::Layer);

        let l2 = rules.rules.iter().find(|r| r.id == "L2");
        assert!(l2.is_some(), "Expected L2 rule");
    }

    #[test]
    fn generate_rules_includes_cycle_breaks() {
        let mut inferred_layers = HashMap::new();
        inferred_layers.insert(PathBuf::from("api"), LayerType::Entry);
        inferred_layers.insert(PathBuf::from("services"), LayerType::Service);

        let circular_deps = vec![CircularDep {
            a: PathBuf::from("services/auth.py"),
            b: PathBuf::from("api/routes.py"),
        }];

        let arch_report = ArchitectureReport {
            entry_layer: Vec::new(),
            middle_layer: Vec::new(),
            leaf_layer: Vec::new(),
            directories: HashMap::new(),
            circular_dependencies: circular_deps,
            inferred_layers,
        };

        let context = RulesGenerationContext::new(PathBuf::from("."));
        let rules = generate_rules(&arch_report, &context);

        // Should have C1 cycle break rule
        let c1 = rules.rules.iter().find(|r| r.id == "C1");
        assert!(c1.is_some(), "Expected C1 cycle break rule");

        let c1 = c1.unwrap();
        assert_eq!(c1.rule_type, ArchRuleType::CycleBreak);
        assert_eq!(c1.severity, RuleSeverity::Warn);
        assert_eq!(c1.files.len(), 2);
    }

    // -------------------------------------------------------------------------
    // Rule Checking Tests
    // -------------------------------------------------------------------------

    #[test]
    fn check_rules_detects_layer_violation() {
        // Create a rules file
        let rules = ArchRulesFile::new().with_rule(ArchRule::layer(
            "L1",
            "LOW may not import HIGH",
            vec!["LOW".to_string()],
            vec!["HIGH".to_string()],
            "Test rationale",
        ));

        // Create an import graph with a violation (utils imports api)
        let mut import_graph = ImportGraph::new();
        import_graph.add_edge(ImportEdge {
            from_file: PathBuf::from("utils/helpers.py"),
            to_file: PathBuf::from("api/routes.py"),
            module: "api.routes".to_string(),
            line: 5,
        });

        // Create layer mappings
        let mut layers = HashMap::new();
        layers.insert(PathBuf::from("api"), LayerType::Entry);
        layers.insert(PathBuf::from("utils"), LayerType::Utility);

        let report = check_rules(&rules, &import_graph, &layers);

        assert!(!report.pass, "Expected violations");
        assert!(!report.violations.is_empty());
        assert_eq!(report.violations[0].rule_id, "L1");
    }

    #[test]
    fn check_rules_detects_cycle_violation() {
        // Create a rules file with a cycle break rule
        let rules = ArchRulesFile::new().with_rule(ArchRule::cycle_break(
            "C1",
            "Break cycle",
            vec!["services/auth.py".to_string(), "api/routes.py".to_string()],
            "Test rationale",
        ));

        // Create an import graph where the cycle exists
        let mut import_graph = ImportGraph::new();
        import_graph.add_edge(ImportEdge {
            from_file: PathBuf::from("services/auth.py"),
            to_file: PathBuf::from("api/routes.py"),
            module: "api.routes".to_string(),
            line: 1,
        });
        import_graph.add_edge(ImportEdge {
            from_file: PathBuf::from("api/routes.py"),
            to_file: PathBuf::from("services/auth.py"),
            module: "services.auth".to_string(),
            line: 1,
        });

        let layers = HashMap::new();
        let report = check_rules(&rules, &import_graph, &layers);

        assert!(report.has_violations());
        let cycle_violations: Vec<_> = report
            .violations
            .iter()
            .filter(|v| v.rule_id == "C1")
            .collect();
        assert!(!cycle_violations.is_empty());
    }

    #[test]
    fn check_rules_allows_valid_dependencies() {
        // Create rules
        let rules = ArchRulesFile::new().with_rule(ArchRule::layer(
            "L1",
            "LOW may not import HIGH",
            vec!["LOW".to_string()],
            vec!["HIGH".to_string()],
            "Test",
        ));

        // Create a valid import graph (api -> services -> utils)
        let mut import_graph = ImportGraph::new();
        import_graph.add_edge(ImportEdge {
            from_file: PathBuf::from("api/routes.py"),
            to_file: PathBuf::from("services/user.py"),
            module: "services.user".to_string(),
            line: 1,
        });
        import_graph.add_edge(ImportEdge {
            from_file: PathBuf::from("services/user.py"),
            to_file: PathBuf::from("utils/db.py"),
            module: "utils.db".to_string(),
            line: 1,
        });

        let mut layers = HashMap::new();
        layers.insert(PathBuf::from("api"), LayerType::Entry);
        layers.insert(PathBuf::from("services"), LayerType::Service);
        layers.insert(PathBuf::from("utils"), LayerType::Utility);

        let report = check_rules(&rules, &import_graph, &layers);

        assert!(report.pass, "Valid dependencies should pass");
        assert!(report.violations.is_empty());
    }

    // -------------------------------------------------------------------------
    // Import Graph Tests
    // -------------------------------------------------------------------------

    #[test]
    fn import_graph_add_edge() {
        let mut graph = ImportGraph::new();

        graph.add_edge(ImportEdge {
            from_file: PathBuf::from("a.py"),
            to_file: PathBuf::from("b.py"),
            module: "b".to_string(),
            line: 1,
        });

        assert_eq!(graph.edges.len(), 1);
        assert!(graph.files.contains(&PathBuf::from("a.py")));
        assert!(graph.files.contains(&PathBuf::from("b.py")));
    }

    #[test]
    fn resolve_python_import_absolute() {
        let all_files: HashSet<PathBuf> = [
            PathBuf::from("/project/services/user.py"),
            PathBuf::from("/project/utils/db.py"),
        ]
        .into_iter()
        .collect();

        let import = ImportInfo {
            module: "services.user".to_string(),
            names: Vec::new(),
            is_from: false,
            alias: None,
        };

        let resolved = resolve_python_import(
            &import,
            &PathBuf::from("/project/api/routes.py"),
            &PathBuf::from("/project"),
            &all_files,
        );

        assert_eq!(resolved, Some(PathBuf::from("/project/services/user.py")));
    }

    #[test]
    fn resolve_python_import_relative() {
        let all_files: HashSet<PathBuf> = [
            PathBuf::from("/project/api/routes.py"),
            PathBuf::from("/project/api/utils.py"),
        ]
        .into_iter()
        .collect();

        let import = ImportInfo {
            module: ".utils".to_string(),
            names: Vec::new(),
            is_from: true,
            alias: None,
        };

        let resolved = resolve_python_import(
            &import,
            &PathBuf::from("/project/api/routes.py"),
            &PathBuf::from("/project"),
            &all_files,
        );

        assert_eq!(resolved, Some(PathBuf::from("/project/api/utils.py")));
    }
}
