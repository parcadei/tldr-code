//! Cohesion command - LCOM4 (Lack of Cohesion of Methods) analysis for Python classes.
//!
//! LCOM4 measures class cohesion by counting connected components in the method-field graph:
//! - LCOM4 = 1: All methods are connected (cohesive class)
//! - LCOM4 > 1: Methods form disconnected groups (split candidate)
//!
//! # Algorithm
//!
//! 1. Parse class, extract methods and field accesses (`self.x`)
//! 2. Build bipartite graph: methods <-> fields they access
//! 3. Add edges for intra-class method calls (`self.method()`)
//! 4. Count connected components via union-find with path compression
//!
//! # TIGER Mitigations
//!
//! - **T06**: Union-find with path compression AND union by rank
//! - **E01**: `--timeout` flag (default 30s)
//! - **E04**: `MAX_METHODS_PER_CLASS` and `MAX_FIELDS_PER_CLASS` limits
//! - **E05**: `MAX_ITERATIONS` for union-find operations
//!
//! # Example
//!
//! ```bash
//! tldr cohesion src/models.py
//! tldr cohesion src/models.py --min-methods 3 --include-dunder
//! tldr cohesion src/ --format text
//! ```

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::{Args, ValueEnum};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use tldr_core::walker::walk_project;
use tree_sitter::{Node, Parser, Tree};
use tree_sitter_python::LANGUAGE as PYTHON_LANGUAGE;

use tldr_core::quality::cohesion as core_cohesion;
use tldr_core::types::Language;

use crate::output::{common_path_prefix, strip_prefix_display, OutputFormat as GlobalOutputFormat};

use super::error::{PatternsError, PatternsResult};
use super::types::{
    ClassCohesion, CohesionReport, CohesionSummary, CohesionVerdict, ComponentInfo,
};
use super::validation::{
    read_file_safe, validate_directory_path, validate_file_path, validate_file_path_in_project,
    MAX_CLASSES_PER_FILE, MAX_DIRECTORY_FILES, MAX_FIELDS_PER_CLASS, MAX_METHODS_PER_CLASS,
};

// =============================================================================
// Constants (TIGER/ELEPHANT Mitigations)
// =============================================================================

/// Maximum union-find iterations to prevent infinite loops (E05)
const MAX_UNION_FIND_ITERATIONS: usize = 10_000;

/// Default timeout in seconds (E01)
const DEFAULT_TIMEOUT_SECS: u64 = 30;

// =============================================================================
// Output Format
// =============================================================================

/// Output format for cohesion command
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// JSON output (default)
    #[default]
    Json,
    /// Human-readable text output
    Text,
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Compute LCOM4 (Lack of Cohesion of Methods) metric for Python classes.
///
/// LCOM4 measures class cohesion by counting connected components in the
/// method-field bipartite graph. A cohesive class has LCOM4 = 1, while
/// a class with LCOM4 > 1 is a candidate for splitting.
///
/// # Example
///
/// ```bash
/// tldr cohesion src/models.py
/// tldr cohesion src/models.py --min-methods 3
/// tldr cohesion src/ --format text
/// ```
#[derive(Debug, Args)]
pub struct CohesionArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Minimum number of instance methods for a class to be included in analysis.
    /// Classes with fewer methods are filtered from results. For Rust and Go,
    /// only instance methods (with self/receiver) are counted, not associated
    /// functions like new() or default().
    #[arg(long, default_value = "1")]
    pub min_methods: u32,

    /// Include dunder methods (__init__, __str__, etc.) in analysis
    #[arg(long)]
    pub include_dunder: bool,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output-format",
        alias = "output",
        short = 'o',
        hide = true,
        value_enum,
        default_value = "json"
    )]
    pub output_format: OutputFormat,

    /// Analysis timeout in seconds
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_SECS)]
    pub timeout: u64,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,

    /// Language filter (auto-detected if omitted)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl CohesionArgs {
    /// Run the cohesion analysis command
    pub fn run(&self, global_format: GlobalOutputFormat) -> Result<()> {
        let start = Instant::now();
        let timeout = Duration::from_secs(self.timeout);

        // Validate path
        let canonical_path = if let Some(ref root) = self.project_root {
            validate_file_path_in_project(&self.path, root)?
        } else {
            validate_file_path(&self.path)?
        };

        // Analyze based on path type
        let report = if canonical_path.is_dir() {
            analyze_directory(&canonical_path, self, start, timeout)?
        } else {
            analyze_single_file(&canonical_path, self)?
        };

        // Resolve format: global -f flag takes priority over hidden --output-format
        let use_text = matches!(global_format, GlobalOutputFormat::Text)
            || matches!(self.output_format, OutputFormat::Text);

        // Output based on format
        if use_text {
            let text = format_cohesion_text(&report);
            println!("{}", text);
        } else {
            let json = serde_json::to_string_pretty(&report)?;
            println!("{}", json);
        }

        Ok(())
    }
}

// =============================================================================
// Union-Find with Path Compression (TIGER-06)
// =============================================================================

/// Union-Find (Disjoint Set Union) data structure with path compression and union by rank.
///
/// This implementation uses both optimizations to achieve near-O(1) amortized time per operation:
/// - **Path compression**: Flatten tree during `find` operations
/// - **Union by rank**: Attach smaller tree under root of larger tree
///
/// # TIGER-06 Mitigation
///
/// Path compression prevents worst-case O(n) find operations.
/// Union by rank keeps trees balanced.
#[derive(Debug, Clone)]
pub struct UnionFind {
    /// Parent pointers (index -> parent index)
    parent: Vec<usize>,
    /// Rank for union by rank optimization
    rank: Vec<usize>,
    /// Iteration counter to prevent infinite loops (E05)
    iterations: usize,
    /// Maximum allowed iterations
    max_iterations: usize,
}

impl UnionFind {
    /// Create a new union-find structure with n elements
    pub fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
            iterations: 0,
            max_iterations: MAX_UNION_FIND_ITERATIONS,
        }
    }

    /// Find the root of the set containing x, with path compression.
    ///
    /// Returns None if max iterations exceeded.
    pub fn find(&mut self, x: usize) -> Option<usize> {
        if x >= self.parent.len() {
            return None;
        }

        // Find root
        let mut root = x;
        while self.parent[root] != root {
            self.iterations += 1;
            if self.iterations > self.max_iterations {
                return None; // Exceeded iteration limit
            }
            root = self.parent[root];
        }

        // Path compression: point all nodes on path directly to root
        let mut current = x;
        while self.parent[current] != root {
            self.iterations += 1;
            if self.iterations > self.max_iterations {
                return None;
            }
            let next = self.parent[current];
            self.parent[current] = root;
            current = next;
        }

        Some(root)
    }

    /// Union the sets containing x and y, using union by rank.
    ///
    /// Returns true if a union was performed, false if already in same set or error.
    pub fn union(&mut self, x: usize, y: usize) -> bool {
        let root_x = match self.find(x) {
            Some(r) => r,
            None => return false,
        };
        let root_y = match self.find(y) {
            Some(r) => r,
            None => return false,
        };

        if root_x == root_y {
            return false; // Already in same set
        }

        // Union by rank: attach smaller tree under root of larger tree
        match self.rank[root_x].cmp(&self.rank[root_y]) {
            std::cmp::Ordering::Less => {
                self.parent[root_x] = root_y;
            }
            std::cmp::Ordering::Greater => {
                self.parent[root_y] = root_x;
            }
            std::cmp::Ordering::Equal => {
                self.parent[root_y] = root_x;
                self.rank[root_x] += 1;
            }
        }

        true
    }

    /// Count the number of unique connected components.
    ///
    /// Only counts components for the first `method_count` elements (ignoring fields
    /// that are only connected to a single method).
    pub fn count_components(&mut self, method_count: usize) -> usize {
        let mut roots = HashSet::new();
        for i in 0..method_count.min(self.parent.len()) {
            if let Some(root) = self.find(i) {
                roots.insert(root);
            }
        }
        roots.len()
    }

    /// Get components as groups of indices.
    pub fn get_components(&mut self) -> HashMap<usize, Vec<usize>> {
        let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..self.parent.len() {
            if let Some(root) = self.find(i) {
                components.entry(root).or_default().push(i);
            }
        }
        components
    }

    /// Check if iteration limit was exceeded
    pub fn limit_exceeded(&self) -> bool {
        self.iterations > self.max_iterations
    }
}

// =============================================================================
// Method Analysis
// =============================================================================

/// Analysis result for a single method
#[derive(Debug, Clone)]
struct MethodAnalysis {
    /// Method name
    name: String,
    /// Fields accessed by this method (self.x)
    field_accesses: Vec<String>,
    /// Other methods called (self.method())
    method_calls: Vec<String>,
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Analyze a single file for class cohesion.
///
/// For Python files, uses the CLI's own Python-specific implementation.
/// For all other supported languages (Java, TypeScript, Go, Rust, etc.),
/// delegates to the core library's multi-language analyzer.
fn analyze_single_file(path: &Path, args: &CohesionArgs) -> PatternsResult<CohesionReport> {
    let lang = Language::from_path(path);

    // For non-Python languages, delegate to the core multi-language analyzer
    if lang != Some(Language::Python) && lang.is_some() {
        return analyze_single_file_core(path, args);
    }

    // Python: use the CLI's existing Python-specific implementation
    let source = read_file_safe(path)?;
    let tree = parse_python(&source, path)?;
    let classes = analyze_file_ast(&tree, &source, path, args)?;

    let summary = compute_summary(&classes);

    Ok(CohesionReport { classes, summary })
}

/// Analyze a single non-Python file using the core library.
fn analyze_single_file_core(path: &Path, args: &CohesionArgs) -> PatternsResult<CohesionReport> {
    let threshold = 2;
    let core_report = core_cohesion::analyze_cohesion(path, None, threshold).map_err(|e| {
        PatternsError::ParseError {
            file: path.to_path_buf(),
            message: format!("Core cohesion analysis failed: {}", e),
        }
    })?;

    // Convert core types to CLI types
    let classes: Vec<ClassCohesion> = core_report
        .classes
        .into_iter()
        .filter(|c| c.method_count >= args.min_methods as usize)
        .map(|c| ClassCohesion {
            class_name: c.name,
            file_path: c.file.display().to_string(),
            line: c.line as u32,
            lcom4: c.lcom4 as u32,
            method_count: c.method_count as u32,
            field_count: c.field_count as u32,
            verdict: match c.verdict {
                core_cohesion::CohesionVerdict::Cohesive => CohesionVerdict::Cohesive,
                core_cohesion::CohesionVerdict::SplitCandidate => CohesionVerdict::SplitCandidate,
            },
            split_suggestion: c.split_suggestion,
            components: c
                .components
                .into_iter()
                .map(|comp| ComponentInfo {
                    methods: comp.methods,
                    fields: comp.fields,
                })
                .collect(),
        })
        .collect();

    let summary = compute_summary(&classes);
    Ok(CohesionReport { classes, summary })
}

/// Analyze a directory of source files for class cohesion.
///
/// Supports Python, Java, TypeScript, JavaScript, Go, Rust, and other
/// languages supported by the core library.
fn analyze_directory(
    dir: &Path,
    args: &CohesionArgs,
    start: Instant,
    timeout: Duration,
) -> PatternsResult<CohesionReport> {
    validate_directory_path(dir)?;

    let mut all_classes = Vec::new();
    let mut file_count = 0u32;

    for entry in walk_project(dir) {
        // Check timeout
        if start.elapsed() > timeout {
            return Err(PatternsError::Timeout {
                timeout_secs: args.timeout,
            });
        }

        // Check file limit
        if file_count >= MAX_DIRECTORY_FILES {
            return Err(PatternsError::TooManyFiles {
                count: file_count,
                max_files: MAX_DIRECTORY_FILES,
            });
        }

        let path = entry.path();

        // Analyze files with recognized language extensions
        if path.is_file() && Language::from_path(path).is_some() {
            file_count += 1;

            // Skip test files unless explicitly included
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename.starts_with("test_") || filename.ends_with("_test.py") {
                continue;
            }

            // Analyze file, collecting errors but continuing
            match analyze_single_file(path, args) {
                Ok(report) => {
                    all_classes.extend(report.classes);
                }
                Err(_) => {
                    // Skip files with parse errors
                    continue;
                }
            }
        }
    }

    let summary = compute_summary(&all_classes);

    Ok(CohesionReport {
        classes: all_classes,
        summary,
    })
}

/// Parse Python source code with tree-sitter.
fn parse_python(source: &str, file: &Path) -> PatternsResult<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&PYTHON_LANGUAGE.into())
        .map_err(|e| PatternsError::ParseError {
            file: file.to_path_buf(),
            message: format!("Failed to set Python language: {}", e),
        })?;

    parser
        .parse(source, None)
        .ok_or_else(|| PatternsError::ParseError {
            file: file.to_path_buf(),
            message: "Parsing returned None".to_string(),
        })
}

/// Analyze all classes in a parsed Python file.
fn analyze_file_ast(
    tree: &Tree,
    source: &str,
    file: &Path,
    args: &CohesionArgs,
) -> PatternsResult<Vec<ClassCohesion>> {
    let root = tree.root_node();
    let source_bytes = source.as_bytes();
    let mut results = Vec::new();
    let mut class_count = 0;

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "class_definition" {
            class_count += 1;
            if class_count > MAX_CLASSES_PER_FILE {
                break; // Limit exceeded
            }

            if let Some(cohesion) = analyze_class(child, source_bytes, file, args)? {
                results.push(cohesion);
            }
        }
    }

    Ok(results)
}

/// Analyze a single class for LCOM4 cohesion.
fn analyze_class(
    class_node: Node,
    source: &[u8],
    file: &Path,
    args: &CohesionArgs,
) -> PatternsResult<Option<ClassCohesion>> {
    // Get class name
    let class_name = class_node
        .child_by_field_name("name")
        .map(|n| get_node_text(n, source))
        .unwrap_or("<unknown>")
        .to_string();

    let line = class_node.start_position().row as u32 + 1;

    // Get class body
    let body = match class_node.child_by_field_name("body") {
        Some(b) => b,
        None => return Ok(None),
    };

    // Extract methods
    let methods = extract_methods(body, source, args.include_dunder)?;

    // Filter by min_methods threshold (use all methods for threshold)
    let all_methods = extract_methods(body, source, true)?;
    if all_methods.len() < args.min_methods as usize {
        return Ok(None);
    }

    // Check method limit (E04)
    if methods.len() > MAX_METHODS_PER_CLASS {
        return Ok(Some(ClassCohesion {
            class_name,
            file_path: file.display().to_string(),
            line,
            lcom4: 0,
            method_count: methods.len() as u32,
            field_count: 0,
            verdict: CohesionVerdict::Cohesive,
            split_suggestion: Some("Class exceeds MAX_METHODS_PER_CLASS limit".to_string()),
            components: vec![],
        }));
    }

    // Collect all unique fields
    let mut all_fields: HashSet<String> = HashSet::new();
    let method_names: HashSet<&str> = methods.iter().map(|m| m.name.as_str()).collect();

    for method in &methods {
        for field in &method.field_accesses {
            // Don't count method names as fields
            if !method_names.contains(field.as_str()) {
                all_fields.insert(field.clone());
            }
        }
    }

    // Check field limit (E04)
    if all_fields.len() > MAX_FIELDS_PER_CLASS {
        return Ok(Some(ClassCohesion {
            class_name,
            file_path: file.display().to_string(),
            line,
            lcom4: 0,
            method_count: methods.len() as u32,
            field_count: all_fields.len() as u32,
            verdict: CohesionVerdict::Cohesive,
            split_suggestion: Some("Class exceeds MAX_FIELDS_PER_CLASS limit".to_string()),
            components: vec![],
        }));
    }

    let fields: Vec<String> = all_fields.into_iter().collect();

    // Compute LCOM4
    let (lcom4, components) = compute_lcom4(&methods, &fields, &method_names);

    // Determine verdict
    let verdict = CohesionVerdict::from_lcom4(lcom4);

    // Generate split suggestion if needed
    let split_suggestion = if lcom4 > 1 {
        Some(generate_split_suggestion(&class_name, &components))
    } else {
        None
    };

    Ok(Some(ClassCohesion {
        class_name,
        file_path: file.display().to_string(),
        line,
        lcom4,
        method_count: methods.len() as u32,
        field_count: fields.len() as u32,
        verdict,
        split_suggestion,
        components,
    }))
}

/// Extract methods from a class body.
fn extract_methods(
    body: Node,
    source: &[u8],
    include_dunder: bool,
) -> PatternsResult<Vec<MethodAnalysis>> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        // Handle both sync and async function definitions
        if child.kind() == "function_definition" || child.kind() == "async_function_definition" {
            // Get method name
            let name = child
                .child_by_field_name("name")
                .map(|n| get_node_text(n, source))
                .unwrap_or("")
                .to_string();

            // Skip static methods and class methods
            if is_static_or_classmethod(&child, source) {
                continue;
            }

            // Filter dunder methods
            if !include_dunder && is_dunder(&name) {
                continue;
            }

            // Extract field accesses (self.x)
            let field_accesses = extract_field_accesses(child, source);

            // Extract method calls (self.method())
            let method_calls = extract_method_calls(child, source);

            methods.push(MethodAnalysis {
                name,
                field_accesses,
                method_calls,
            });
        }
    }

    Ok(methods)
}

/// Check if a method is a dunder method (__xxx__)
fn is_dunder(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__")
}

/// Check if a method is decorated with @staticmethod or @classmethod
fn is_static_or_classmethod(node: &Node, source: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = get_node_text(child, source);
            if text.contains("staticmethod") || text.contains("classmethod") {
                return true;
            }
        }
    }
    false
}

/// Extract field accesses (self.x) from a method.
fn extract_field_accesses(method: Node, source: &[u8]) -> Vec<String> {
    let mut fields = Vec::new();
    let self_name = get_self_param_name(method, source);

    extract_field_accesses_recursive(method, source, &self_name, &mut fields);

    fields.sort();
    fields.dedup();
    fields
}

fn extract_field_accesses_recursive(
    node: Node,
    source: &[u8],
    self_name: &str,
    fields: &mut Vec<String>,
) {
    // Check if this is a self.x attribute access
    if node.kind() == "attribute" {
        if let Some(obj) = node.child_by_field_name("object") {
            if obj.kind() == "identifier" && get_node_text(obj, source) == self_name {
                if let Some(attr) = node.child_by_field_name("attribute") {
                    let attr_name = get_node_text(attr, source);
                    fields.push(attr_name.to_string());
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_field_accesses_recursive(child, source, self_name, fields);
    }
}

/// Extract method calls (self.method()) from a method.
fn extract_method_calls(method: Node, source: &[u8]) -> Vec<String> {
    let mut calls = Vec::new();
    let self_name = get_self_param_name(method, source);

    extract_method_calls_recursive(method, source, &self_name, &mut calls);

    calls.sort();
    calls.dedup();
    calls
}

fn extract_method_calls_recursive(
    node: Node,
    source: &[u8],
    self_name: &str,
    calls: &mut Vec<String>,
) {
    // Check if this is a self.method() call
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            if func.kind() == "attribute" {
                if let Some(obj) = func.child_by_field_name("object") {
                    if obj.kind() == "identifier" && get_node_text(obj, source) == self_name {
                        if let Some(attr) = func.child_by_field_name("attribute") {
                            let method_name = get_node_text(attr, source);
                            calls.push(method_name.to_string());
                        }
                    }
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_method_calls_recursive(child, source, self_name, calls);
    }
}

/// Get the name of the self parameter (usually "self" but could be different)
fn get_self_param_name(method: Node, source: &[u8]) -> String {
    if let Some(params) = method.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for child in params.children(&mut cursor) {
            if child.kind() == "identifier" {
                return get_node_text(child, source).to_string();
            }
        }
    }
    "self".to_string()
}

/// Compute LCOM4 using union-find.
///
/// # Returns
/// (lcom4_value, connected_components)
fn compute_lcom4(
    methods: &[MethodAnalysis],
    fields: &[String],
    method_names: &HashSet<&str>,
) -> (u32, Vec<ComponentInfo>) {
    if methods.is_empty() {
        return (0, vec![]);
    }

    // Create index mappings
    let method_idx: HashMap<&str, usize> = methods
        .iter()
        .enumerate()
        .map(|(i, m)| (m.name.as_str(), i))
        .collect();

    let field_idx: HashMap<&str, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| (f.as_str(), methods.len() + i))
        .collect();

    // Initialize union-find
    let mut uf = UnionFind::new(methods.len() + fields.len());

    // Connect methods to fields they access
    for (i, method) in methods.iter().enumerate() {
        for field in &method.field_accesses {
            if let Some(&fi) = field_idx.get(field.as_str()) {
                uf.union(i, fi);
            }
        }
    }

    // Connect methods that call each other
    for (i, method) in methods.iter().enumerate() {
        for called in &method.method_calls {
            if method_names.contains(called.as_str()) {
                if let Some(&ci) = method_idx.get(called.as_str()) {
                    uf.union(i, ci);
                }
            }
        }
    }

    // Check if limit was exceeded
    if uf.limit_exceeded() {
        return (
            0,
            vec![ComponentInfo {
                methods: vec!["<analysis incomplete>".to_string()],
                fields: vec![],
            }],
        );
    }

    // Build component infos
    let raw_components = uf.get_components();
    let mut component_infos: Vec<ComponentInfo> = Vec::new();

    for (_, members) in raw_components {
        let mut ci = ComponentInfo {
            methods: Vec::new(),
            fields: Vec::new(),
        };

        for member_idx in members {
            if member_idx < methods.len() {
                ci.methods.push(methods[member_idx].name.clone());
            } else {
                let field_pos = member_idx - methods.len();
                if field_pos < fields.len() {
                    ci.fields.push(fields[field_pos].clone());
                }
            }
        }

        // Only include components that have at least one method
        if !ci.methods.is_empty() {
            ci.methods.sort();
            ci.fields.sort();
            component_infos.push(ci);
        }
    }

    // Sort components by first method name for deterministic output
    component_infos.sort_by(|a, b| a.methods.first().cmp(&b.methods.first()));

    let lcom4 = component_infos.len() as u32;
    (lcom4.max(1), component_infos) // LCOM4 is at least 1 if there are methods
}

/// Generate a split suggestion for a class with LCOM4 > 1.
fn generate_split_suggestion(class_name: &str, components: &[ComponentInfo]) -> String {
    if components.is_empty() {
        return format!("Consider splitting {} into multiple classes", class_name);
    }

    let parts: Vec<String> = components
        .iter()
        .map(|c| {
            let methods_str = c.methods.join(", ");
            format!("[{}]", methods_str)
        })
        .collect();

    format!(
        "Consider splitting {} into {} classes: {}",
        class_name,
        components.len(),
        parts.join(" + ")
    )
}

/// Compute summary statistics for a set of class cohesion results.
fn compute_summary(classes: &[ClassCohesion]) -> CohesionSummary {
    let total = classes.len() as u32;
    if total == 0 {
        return CohesionSummary::default();
    }

    let cohesive = classes
        .iter()
        .filter(|c| c.verdict == CohesionVerdict::Cohesive)
        .count() as u32;

    let split_candidates = total - cohesive;

    let avg_lcom4 = classes.iter().map(|c| c.lcom4 as f64).sum::<f64>() / total as f64;

    CohesionSummary {
        total_classes: total,
        cohesive,
        split_candidates,
        avg_lcom4: (avg_lcom4 * 100.0).round() / 100.0, // Round to 2 decimal places
    }
}

/// Get text content of a node.
fn get_node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    let start = node.start_byte();
    let end = node.end_byte();
    if end <= source.len() {
        std::str::from_utf8(&source[start..end]).unwrap_or("")
    } else {
        ""
    }
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format a cohesion report as human-readable text.
///
/// Shows split candidate classes sorted worst-first (highest LCOM4), with
/// color-coded severity, path stripping, component details, and split suggestions.
/// Top 30 entries shown by default with overflow message.
///
/// ```text
/// Cohesion Analysis (LCOM4)
///
/// LCOM4  Methods  Fields  Class                         File
///     4        8       6  UserManager                   models/user.py:42
///     |-- Component 1: create, update [db, cache]
///     |-- Component 2: send_email [mailer]
///     `-- Suggestion: Split into 4 focused classes
///     3        6       4  OrderProcessor                services/order.py:15
///     |-- Component 1: process, submit [queue]
///     `-- Suggestion: Split into 3 focused classes
///
/// Summary: 47 classes, 12 split candidates (25.5%), avg LCOM4: 1.82
/// ```
pub fn format_cohesion_text(report: &CohesionReport) -> String {
    let mut output = String::new();

    let s = &report.summary;
    output.push_str(&format!(
        "Cohesion Analysis (LCOM4) ({} classes, {} split candidates)\n\n",
        s.total_classes, s.split_candidates
    ));

    // Filter to split candidates only (LCOM4 > 1) and sort worst-first
    let mut candidates: Vec<&ClassCohesion> = report
        .classes
        .iter()
        .filter(|c| c.verdict == CohesionVerdict::SplitCandidate)
        .collect();
    candidates.sort_by(|a, b| b.lcom4.cmp(&a.lcom4));

    if candidates.is_empty() {
        output.push_str("  No split candidates found.\n\n");
        output.push_str(&format_cohesion_summary(s));
        return output;
    }

    // Compute common path prefix for relative display
    let paths: Vec<&Path> = candidates
        .iter()
        .filter_map(|c| Path::new(c.file_path.as_str()).parent())
        .collect();
    let prefix = if paths.is_empty() {
        std::path::PathBuf::new()
    } else {
        common_path_prefix(&paths)
    };

    // Header
    output.push_str(&format!(
        " {:>5}  {:>7}  {:>6}  {:<28}  {}\n",
        "LCOM4", "Methods", "Fields", "Class", "File"
    ));

    // Show top 30
    let limit = candidates.len().min(30);
    for class in candidates.iter().take(limit) {
        let rel = strip_prefix_display(Path::new(&class.file_path), &prefix);
        let lcom4_str = format_lcom4_colored(class.lcom4);

        // Truncate class name to 28 chars
        let name = if class.class_name.len() > 28 {
            format!("{}...", &class.class_name[..25])
        } else {
            class.class_name.clone()
        };

        output.push_str(&format!(
            " {:>5}  {:>7}  {:>6}  {:<28}  {}:{}\n",
            lcom4_str, class.method_count, class.field_count, name, rel, class.line
        ));

        // Show component details for split candidates
        if !class.components.is_empty() {
            let comp_count = class.components.len();
            for (i, comp) in class.components.iter().enumerate() {
                let is_last = i == comp_count - 1 && class.split_suggestion.is_none();
                let connector = if is_last { "`--" } else { "|--" };
                let methods_str = comp.methods.join(", ");
                let fields_str = if comp.fields.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", comp.fields.join(", "))
                };
                output.push_str(&format!(
                    "     {}  Component {}: {}{}\n",
                    connector,
                    i + 1,
                    methods_str,
                    fields_str
                ));
            }
        }

        // Show split suggestion
        if let Some(ref suggestion) = class.split_suggestion {
            output.push_str(&format!("     `--  Suggestion: {}\n", suggestion));
        }
    }

    if candidates.len() > limit {
        output.push_str(&format!(
            "\n  ... and {} more split candidates\n",
            candidates.len() - limit
        ));
    }

    output.push('\n');
    output.push_str(&format_cohesion_summary(s));

    output
}

/// Format LCOM4 value with color coding based on severity.
fn format_lcom4_colored(lcom4: u32) -> String {
    if lcom4 >= 4 {
        format!("{}", lcom4).red().bold().to_string()
    } else if lcom4 >= 2 {
        format!("{}", lcom4).yellow().to_string()
    } else {
        format!("{}", lcom4).green().to_string()
    }
}

/// Format the cohesion summary line.
fn format_cohesion_summary(s: &CohesionSummary) -> String {
    let pct = if s.total_classes > 0 {
        (s.split_candidates as f64 / s.total_classes as f64) * 100.0
    } else {
        0.0
    };
    format!(
        "Summary: {} classes, {} split candidates ({:.1}%), avg LCOM4: {:.2}\n",
        s.total_classes, s.split_candidates, pct, s.avg_lcom4
    )
}

// =============================================================================
// Public Entry Point
// =============================================================================

/// Run cohesion analysis (for programmatic use).
pub fn run(args: CohesionArgs) -> Result<CohesionReport> {
    let start = Instant::now();
    let timeout = Duration::from_secs(args.timeout);

    // Validate path
    let canonical_path = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(&args.path, root)?
    } else {
        validate_file_path(&args.path)?
    };

    // Analyze based on path type
    let report = if canonical_path.is_dir() {
        analyze_directory(&canonical_path, &args, start, timeout)?
    } else {
        analyze_single_file(&canonical_path, &args)?
    };

    Ok(report)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union_find_basic() {
        let mut uf = UnionFind::new(5);

        // Initially all separate
        assert_eq!(uf.find(0), Some(0));
        assert_eq!(uf.find(1), Some(1));

        // Union 0 and 1
        assert!(uf.union(0, 1));
        assert_eq!(uf.find(0), uf.find(1));

        // Union 2 and 3
        assert!(uf.union(2, 3));
        assert_eq!(uf.find(2), uf.find(3));

        // Different components
        assert_ne!(uf.find(0), uf.find(2));

        // Union the two components
        assert!(uf.union(1, 3));
        assert_eq!(uf.find(0), uf.find(3));
    }

    #[test]
    fn test_union_find_path_compression() {
        let mut uf = UnionFind::new(10);

        // Create a chain: 0 -> 1 -> 2 -> 3 -> 4
        for i in 0..4 {
            uf.union(i, i + 1);
        }

        // After find with path compression, all should point to root
        let root = uf.find(0).unwrap();
        for i in 0..5 {
            assert_eq!(uf.find(i), Some(root));
        }
    }

    #[test]
    fn test_union_find_count_components() {
        let mut uf = UnionFind::new(6);

        // Create two components: {0, 1, 2} and {3, 4, 5}
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);
        uf.union(4, 5);

        assert_eq!(uf.count_components(6), 2);
    }

    #[test]
    fn test_is_dunder() {
        assert!(is_dunder("__init__"));
        assert!(is_dunder("__str__"));
        assert!(is_dunder("__eq__"));
        assert!(!is_dunder("_private"));
        assert!(!is_dunder("__private"));
        assert!(!is_dunder("public__"));
        assert!(!is_dunder("normal"));
    }

    #[test]
    fn test_compute_summary() {
        let classes = vec![
            ClassCohesion {
                class_name: "A".to_string(),
                file_path: "test.py".to_string(),
                line: 1,
                lcom4: 1,
                method_count: 3,
                field_count: 2,
                verdict: CohesionVerdict::Cohesive,
                split_suggestion: None,
                components: vec![],
            },
            ClassCohesion {
                class_name: "B".to_string(),
                file_path: "test.py".to_string(),
                line: 10,
                lcom4: 2,
                method_count: 4,
                field_count: 3,
                verdict: CohesionVerdict::SplitCandidate,
                split_suggestion: Some("Split B".to_string()),
                components: vec![],
            },
        ];

        let summary = compute_summary(&classes);
        assert_eq!(summary.total_classes, 2);
        assert_eq!(summary.cohesive, 1);
        assert_eq!(summary.split_candidates, 1);
        assert!((summary.avg_lcom4 - 1.5).abs() < 0.01);
    }

    #[test]
    fn test_generate_split_suggestion() {
        let components = vec![
            ComponentInfo {
                methods: vec!["method_a".to_string(), "method_b".to_string()],
                fields: vec!["field_x".to_string()],
            },
            ComponentInfo {
                methods: vec!["method_c".to_string()],
                fields: vec!["field_y".to_string()],
            },
        ];

        let suggestion = generate_split_suggestion("MyClass", &components);
        assert!(suggestion.contains("MyClass"));
        assert!(suggestion.contains("2 classes"));
        assert!(suggestion.contains("method_a"));
        assert!(suggestion.contains("method_c"));
    }

    // =========================================================================
    // format_cohesion_text tests
    // =========================================================================

    /// Helper to build a ClassCohesion for tests.
    fn make_class(
        name: &str,
        location: (&str, u32),
        lcom4: u32,
        methods: u32,
        fields: u32,
        components: Vec<ComponentInfo>,
        suggestion: Option<&str>,
    ) -> ClassCohesion {
        let (file, line) = location;
        ClassCohesion {
            class_name: name.to_string(),
            file_path: file.to_string(),
            line,
            lcom4,
            method_count: methods,
            field_count: fields,
            verdict: CohesionVerdict::from_lcom4(lcom4),
            split_suggestion: suggestion.map(|s| s.to_string()),
            components,
        }
    }

    #[test]
    fn test_format_cohesion_text_sorts_worst_first() {
        let report = CohesionReport {
            classes: vec![
                make_class("Low", ("src/a.py", 1), 2, 3, 2, vec![], None),
                make_class("High", ("src/b.py", 5), 5, 8, 6, vec![], None),
                make_class("Mid", ("src/c.py", 10), 3, 5, 4, vec![], None),
            ],
            summary: CohesionSummary {
                total_classes: 3,
                cohesive: 0,
                split_candidates: 3,
                avg_lcom4: 3.33,
            },
        };
        let text = format_cohesion_text(&report);
        // "High" (LCOM4=5) should appear before "Mid" (3) before "Low" (2)
        let high_pos = text.find("High").expect("High not found");
        let mid_pos = text.find("Mid").expect("Mid not found");
        let low_pos = text.find("Low").expect("Low not found");
        assert!(
            high_pos < mid_pos,
            "High (LCOM4=5) should appear before Mid (LCOM4=3)"
        );
        assert!(
            mid_pos < low_pos,
            "Mid (LCOM4=3) should appear before Low (LCOM4=2)"
        );
    }

    #[test]
    fn test_format_cohesion_text_filters_cohesive_classes() {
        let report = CohesionReport {
            classes: vec![
                make_class("Cohesive", ("src/a.py", 1), 1, 3, 2, vec![], None),
                make_class("NeedsSplit", ("src/b.py", 5), 3, 6, 4, vec![], None),
            ],
            summary: CohesionSummary {
                total_classes: 2,
                cohesive: 1,
                split_candidates: 1,
                avg_lcom4: 2.0,
            },
        };
        let text = format_cohesion_text(&report);
        // Cohesive class (LCOM4=1) should NOT appear in the table rows
        // but NeedsSplit (LCOM4=3) should appear
        assert!(
            !text.contains("Cohesive"),
            "Cohesive classes should be filtered out"
        );
        assert!(
            text.contains("NeedsSplit"),
            "Split candidates should appear"
        );
    }

    #[test]
    fn test_format_cohesion_text_limits_to_30() {
        // Create 35 split candidates
        let classes: Vec<ClassCohesion> = (0..35)
            .map(|i| {
                make_class(
                    &format!("Class{}", i),
                    (&format!("src/mod{}.py", i), i + 1),
                    2,
                    4,
                    3,
                    vec![],
                    None,
                )
            })
            .collect();
        let report = CohesionReport {
            classes,
            summary: CohesionSummary {
                total_classes: 35,
                cohesive: 0,
                split_candidates: 35,
                avg_lcom4: 2.0,
            },
        };
        let text = format_cohesion_text(&report);
        assert!(
            text.contains("and 5 more"),
            "Should show overflow message for remaining 5 classes"
        );
    }

    #[test]
    fn test_format_cohesion_text_strips_common_path_prefix() {
        let report = CohesionReport {
            classes: vec![
                make_class("A", ("src/models/user.py", 1), 3, 5, 4, vec![], None),
                make_class("B", ("src/models/order.py", 10), 2, 4, 3, vec![], None),
            ],
            summary: CohesionSummary {
                total_classes: 2,
                cohesive: 0,
                split_candidates: 2,
                avg_lcom4: 2.5,
            },
        };
        let text = format_cohesion_text(&report);
        // The common prefix "src/models/" should be stripped, showing just filenames
        assert!(
            text.contains("user.py"),
            "Should display stripped path: user.py"
        );
        assert!(
            text.contains("order.py"),
            "Should display stripped path: order.py"
        );
        // Full path should not appear
        assert!(
            !text.contains("src/models/user.py"),
            "Full path should be stripped"
        );
    }

    #[test]
    fn test_format_cohesion_text_has_header() {
        let report = CohesionReport {
            classes: vec![make_class("A", ("src/a.py", 1), 2, 3, 2, vec![], None)],
            summary: CohesionSummary {
                total_classes: 1,
                cohesive: 0,
                split_candidates: 1,
                avg_lcom4: 2.0,
            },
        };
        let text = format_cohesion_text(&report);
        assert!(
            text.contains("Cohesion Analysis"),
            "Should have title header"
        );
        assert!(
            text.contains("LCOM4") && text.contains("Methods") && text.contains("Fields"),
            "Should have column headers"
        );
        assert!(
            text.contains("Class") && text.contains("File"),
            "Should have Class and File columns"
        );
    }

    #[test]
    fn test_format_cohesion_text_summary_line() {
        let report = CohesionReport {
            classes: vec![],
            summary: CohesionSummary {
                total_classes: 47,
                cohesive: 35,
                split_candidates: 12,
                avg_lcom4: 1.82,
            },
        };
        let text = format_cohesion_text(&report);
        assert!(
            text.contains("47 classes"),
            "Summary should show total classes"
        );
        assert!(
            text.contains("12 split candidates"),
            "Summary should show split candidate count"
        );
        assert!(text.contains("1.82"), "Summary should show avg LCOM4");
    }

    #[test]
    fn test_format_cohesion_text_shows_components() {
        let components = vec![
            ComponentInfo {
                methods: vec!["create".to_string(), "update".to_string()],
                fields: vec!["db".to_string(), "cache".to_string()],
            },
            ComponentInfo {
                methods: vec!["send_email".to_string()],
                fields: vec!["mailer".to_string()],
            },
        ];
        let report = CohesionReport {
            classes: vec![make_class(
                "UserManager",
                ("src/user.py", 1),
                2,
                3,
                3,
                components,
                Some("Split into 2 focused classes"),
            )],
            summary: CohesionSummary {
                total_classes: 1,
                cohesive: 0,
                split_candidates: 1,
                avg_lcom4: 2.0,
            },
        };
        let text = format_cohesion_text(&report);
        // Should show component info
        assert!(text.contains("Component 1"), "Should show Component 1");
        assert!(
            text.contains("create") && text.contains("update"),
            "Should show methods in component"
        );
        assert!(
            text.contains("db") && text.contains("cache"),
            "Should show fields in component"
        );
        assert!(text.contains("Component 2"), "Should show Component 2");
        assert!(
            text.contains("send_email"),
            "Should show methods in component 2"
        );
        // Should show suggestion
        assert!(
            text.contains("Split into 2 focused classes"),
            "Should show split suggestion"
        );
    }

    #[test]
    fn test_format_cohesion_text_empty_report() {
        let report = CohesionReport {
            classes: vec![],
            summary: CohesionSummary {
                total_classes: 0,
                cohesive: 0,
                split_candidates: 0,
                avg_lcom4: 0.0,
            },
        };
        let text = format_cohesion_text(&report);
        assert!(
            text.contains("No split candidates"),
            "Empty report should show 'No split candidates' message"
        );
    }

    #[test]
    fn test_format_cohesion_text_all_cohesive() {
        let report = CohesionReport {
            classes: vec![
                make_class("Good1", ("src/a.py", 1), 1, 5, 3, vec![], None),
                make_class("Good2", ("src/b.py", 10), 1, 4, 2, vec![], None),
            ],
            summary: CohesionSummary {
                total_classes: 2,
                cohesive: 2,
                split_candidates: 0,
                avg_lcom4: 1.0,
            },
        };
        let text = format_cohesion_text(&report);
        // All classes are cohesive, so no table rows should appear
        assert!(
            text.contains("No split candidates"),
            "All-cohesive report should show 'No split candidates'"
        );
    }

    #[test]
    fn test_cohesion_args_lang_flag() {
        // Verify CohesionArgs has a lang field of type Option<Language>
        let args = CohesionArgs {
            path: PathBuf::from("src/"),
            min_methods: 2,
            include_dunder: false,
            output_format: OutputFormat::Json,
            timeout: 30,
            project_root: None,
            lang: Some(Language::Rust),
        };
        assert_eq!(args.lang, Some(Language::Rust));

        // Also test None case (auto-detect)
        let args_auto = CohesionArgs {
            path: PathBuf::from("src/"),
            min_methods: 2,
            include_dunder: false,
            output_format: OutputFormat::Json,
            timeout: 30,
            project_root: None,
            lang: None,
        };
        assert_eq!(args_auto.lang, None);
    }
}
