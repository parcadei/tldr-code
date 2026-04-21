//! Cohesion Analyzer for Health Command
//!
//! This module provides class cohesion analysis using the LCOM4 metric.
//! It creates a NEW implementation (not reusing debt.rs LCOM4 which returns f64).
//!
//! # LCOM4 Algorithm
//!
//! LCOM4 (Lack of Cohesion of Methods 4) measures class cohesion by counting
//! connected components in the method-field graph:
//!
//! 1. For each class, build a graph where nodes = methods
//! 2. Add edges between methods that share at least one field access
//! 3. Count connected components using Union-Find
//! 4. LCOM4 = component count (usize, NOT normalized!)
//!
//! # Interpretation
//!
//! - LCOM4 = 1: Fully cohesive (all methods share fields, single responsibility)
//! - LCOM4 > 1: Multiple responsibilities, candidate for splitting
//! - LCOM4 = 0: Degenerate case (no methods)
//!
//! # Multi-Language Support
//!
//! - Python: class with def methods
//! - TypeScript/JavaScript: class with methods
//! - Java: class/interface/enum with methods
//! - Go: struct with receiver methods
//! - Rust: struct with impl block methods
//! - Ruby: class with def methods, @instance_variable field access
//! - C#: class/struct/interface with methods, this.field access
//! - Scala: class/object/trait with def methods, this.field access
//! - PHP: class/interface/trait with function methods, $this->field access
//!
//! # References
//!
//! - Chidamber & Kemerer, "A Metrics Suite for Object Oriented Design"
//! - Health spec section 4.2

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::ast::parser::parse;
use crate::error::TldrError;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Information about a connected component in the method-field graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    /// Methods in this component
    pub methods: Vec<String>,
    /// Fields accessed by methods in this component
    pub fields: Vec<String>,
}

/// Verdict for class cohesion
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CohesionVerdict {
    /// Class is cohesive (LCOM4 <= threshold)
    Cohesive,
    /// Class should be considered for splitting (LCOM4 > threshold)
    SplitCandidate,
}

/// Cohesion analysis for a single class
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassCohesion {
    /// Class/struct name
    pub name: String,
    /// File path containing the class
    pub file: PathBuf,
    /// Line number where the class starts
    pub line: usize,
    /// Number of methods (excluding dunders)
    pub method_count: usize,
    /// Number of unique fields accessed
    pub field_count: usize,
    /// LCOM4 value: raw connected component count (NOT normalized!)
    /// - 0: no methods (degenerate)
    /// - 1: fully cohesive
    /// - >1: multiple responsibilities
    pub lcom4: usize,
    /// Connected components with their methods and fields
    pub components: Vec<ComponentInfo>,
    /// Cohesion verdict based on threshold
    pub verdict: CohesionVerdict,
    /// Optional suggestion for splitting
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_suggestion: Option<String>,
}

/// Summary statistics for cohesion analysis
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CohesionSummary {
    /// Total number of classes analyzed
    pub total_classes: usize,
    /// Number of cohesive classes (LCOM4 <= threshold)
    pub cohesive: usize,
    /// Number of split candidates (LCOM4 > threshold)
    pub split_candidates: usize,
    /// Average LCOM4 across all classes (None if no classes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_lcom4: Option<f64>,
}

/// Complete cohesion analysis report
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CohesionReport {
    /// Number of classes analyzed
    pub classes_analyzed: usize,
    /// Average LCOM4 across all classes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_lcom4: Option<f64>,
    /// Number of classes with low cohesion (LCOM4 > threshold)
    pub low_cohesion_count: usize,
    /// All classes with cohesion data (sorted by LCOM4 descending)
    pub classes: Vec<ClassCohesion>,
    /// Summary statistics
    pub summary: CohesionSummary,
}

/// Options for cohesion analysis
#[derive(Debug, Clone)]
pub struct CohesionOptions {
    /// Include dunder methods in analysis (default: false)
    pub include_dunder: bool,
    /// Threshold for low cohesion detection (default: 2)
    /// Classes with LCOM4 > threshold are flagged as SplitCandidate
    pub low_cohesion_threshold: usize,
}

impl Default for CohesionOptions {
    fn default() -> Self {
        Self {
            include_dunder: false,
            low_cohesion_threshold: 2,
        }
    }
}

// =============================================================================
// Union-Find Data Structure
// =============================================================================

/// Union-Find data structure for LCOM4 connected component calculation.
/// Uses iterative path compression to avoid stack overflow.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    /// Find root with iterative path compression
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        // Find root
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression
        let mut node = x;
        while self.parent[node] != root {
            let next = self.parent[node];
            self.parent[node] = root;
            node = next;
        }
        root
    }

    /// Union by rank
    fn union(&mut self, x: usize, y: usize) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx != ry {
            if self.rank[rx] < self.rank[ry] {
                self.parent[rx] = ry;
            } else if self.rank[rx] > self.rank[ry] {
                self.parent[ry] = rx;
            } else {
                self.parent[ry] = rx;
                self.rank[rx] += 1;
            }
        }
    }

    /// Count connected components
    fn count_components(&mut self) -> usize {
        let n = self.parent.len();
        if n == 0 {
            return 0;
        }
        (0..n)
            .map(|i| self.find(i))
            .collect::<HashSet<_>>()
            .len()
    }

    /// Get component ID for each node (after all unions)
    fn get_components(&mut self) -> Vec<usize> {
        let n = self.parent.len();
        (0..n).map(|i| self.find(i)).collect()
    }
}

// =============================================================================
// Internal Types for Class Extraction
// =============================================================================

/// Method information for LCOM4 calculation
struct MethodInfo {
    name: String,
    start_byte: usize,
    end_byte: usize,
}

/// Class information for LCOM4 calculation
struct ClassInfo {
    name: String,
    line: usize,
    methods: Vec<MethodInfo>,
}

// =============================================================================
// Main API
// =============================================================================

/// Analyze class cohesion using LCOM4 metric
///
/// Scans all supported files in the given path, extracts classes, and computes
/// LCOM4 (connected component count) for each class.
///
/// # Arguments
/// * `path` - Directory or file to analyze
/// * `language` - Optional language filter (auto-detect if None)
/// * `threshold` - LCOM4 threshold for low cohesion (default: 2)
///
/// # Returns
/// * `Ok(CohesionReport)` - Report with cohesion metrics per class
/// * `Err(TldrError)` - On file system errors
///
/// # Behavior
/// - LCOM4 = 1 means cohesive (all methods share fields)
/// - LCOM4 > 1 indicates potential for splitting
/// - Dunder methods (__init__, __str__, etc.) excluded by default
/// - Empty classes return LCOM4 = 0 (degenerate case)
///
/// # Example
/// ```ignore
/// use tldr_core::quality::cohesion::analyze_cohesion;
/// use std::path::Path;
///
/// let report = analyze_cohesion(Path::new("src/"), None, 2)?;
/// for class in &report.classes {
///     if class.lcom4 > 2 {
///         println!("{}: LCOM4={} - consider splitting", class.name, class.lcom4);
///     }
/// }
/// ```
pub fn analyze_cohesion(
    path: &Path,
    language: Option<Language>,
    threshold: usize,
) -> TldrResult<CohesionReport> {
    let options = CohesionOptions {
        include_dunder: false,
        low_cohesion_threshold: threshold,
    };

    analyze_cohesion_with_options(path, language, options)
}

/// Analyze class cohesion with full options
pub fn analyze_cohesion_with_options(
    path: &Path,
    language: Option<Language>,
    options: CohesionOptions,
) -> TldrResult<CohesionReport> {
    // Collect files to analyze
    let file_paths: Vec<PathBuf> = if path.is_file() {
        vec![path.to_path_buf()]
    } else {
        WalkDir::new(path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let detected = Language::from_path(e.path());
                match (detected, language) {
                    (Some(d), Some(l)) => d == l,
                    (Some(_), None) => true,
                    _ => false,
                }
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    // Analyze each file and collect class cohesion data
    let mut all_classes: Vec<ClassCohesion> = Vec::new();

    for file_path in &file_paths {
        if let Ok(classes) = analyze_file_cohesion(file_path, &options) {
            all_classes.extend(classes);
        }
        // Skip files that fail to parse (graceful degradation)
    }

    // Sort by LCOM4 descending (worst cohesion first)
    all_classes.sort_by(|a, b| b.lcom4.cmp(&a.lcom4));

    // Calculate summary statistics
    let total_classes = all_classes.len();
    let total_lcom4: usize = all_classes.iter().map(|c| c.lcom4).sum();
    let avg_lcom4 = if total_classes > 0 {
        Some(total_lcom4 as f64 / total_classes as f64)
    } else {
        None
    };
    let low_cohesion_count = all_classes
        .iter()
        .filter(|c| c.lcom4 > options.low_cohesion_threshold)
        .count();
    let cohesive_count = all_classes
        .iter()
        .filter(|c| c.verdict == CohesionVerdict::Cohesive)
        .count();

    let summary = CohesionSummary {
        total_classes,
        cohesive: cohesive_count,
        split_candidates: low_cohesion_count,
        avg_lcom4,
    };

    Ok(CohesionReport {
        classes_analyzed: total_classes,
        avg_lcom4,
        low_cohesion_count,
        classes: all_classes,
        summary,
    })
}

/// Analyze cohesion for all classes in a single file
fn analyze_file_cohesion(
    file_path: &Path,
    options: &CohesionOptions,
) -> TldrResult<Vec<ClassCohesion>> {
    let source = std::fs::read_to_string(file_path)?;
    let language = Language::from_path(file_path)
        .ok_or_else(|| TldrError::UnsupportedLanguage(
            file_path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string()
        ))?;

    // Parse the file using the global parser pool
    let tree = parse(&source, language)?;
    let root = tree.root_node();

    // Extract classes based on language
    let class_infos = extract_classes(root, &source, language);

    // Compute LCOM4 for each class
    let mut results = Vec::new();
    for class_info in class_infos {
        let cohesion = compute_class_cohesion(
            &class_info,
            &source,
            file_path,
            options,
        );
        results.push(cohesion);
    }

    Ok(results)
}

/// Extract classes from the AST based on language
fn extract_classes(
    root: tree_sitter::Node,
    source: &str,
    language: Language,
) -> Vec<ClassInfo> {
    match language {
        Language::Python => extract_python_classes(root, source),
        Language::TypeScript | Language::JavaScript => extract_typescript_classes(root, source),
        Language::Go => extract_go_structs(root, source),
        Language::Rust => extract_rust_structs(root, source),
        Language::Java => extract_java_classes(root, source),
        Language::Ruby => extract_ruby_classes(root, source),
        Language::CSharp => extract_csharp_classes(root, source),
        Language::Scala => extract_scala_classes(root, source),
        Language::Php => extract_php_classes(root, source),
        _ => vec![], // Unsupported language
    }
}

// =============================================================================
// Python Class Extraction
// =============================================================================

/// Extract Python classes with their methods
fn extract_python_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_python_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_python_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                if let Some(class_info) = extract_python_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into class body for nested classes (T16 mitigation)
                if let Some(body) = child.child_by_field_name("body") {
                    extract_python_classes_recursive(body, source, classes);
                }
            }
            "decorated_definition" => {
                // Handle decorated classes
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "class_definition" {
                        if let Some(class_info) = extract_python_class_info(&def, source) {
                            classes.push(class_info);
                        }
                        // Recurse into class body for nested classes
                        if let Some(body) = def.child_by_field_name("body") {
                            extract_python_classes_recursive(body, source, classes);
                        }
                    }
                }
            }
            _ => {
                // Recurse into other nodes (module level)
                extract_python_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_python_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_python_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

fn extract_python_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(method) = extract_python_method(&child, source) {
                    methods.push(method);
                }
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(method) = extract_python_method(&def, source) {
                            methods.push(method);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    methods
}

fn extract_python_method(node: &tree_sitter::Node, source: &str) -> Option<MethodInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();

    Some(MethodInfo {
        name,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

// =============================================================================
// TypeScript/JavaScript Class Extraction
// =============================================================================

/// Extract TypeScript/JavaScript classes with their methods
fn extract_typescript_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_typescript_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_typescript_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "class_declaration" || child.kind() == "class" {
            if let Some(class_info) = extract_typescript_class_info(&child, source) {
                classes.push(class_info);
            }
        }
        // Recurse into children
        extract_typescript_classes_recursive(child, source, classes);
    }
}

fn extract_typescript_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_typescript_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

fn extract_typescript_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        // TypeScript method_definition
        if child.kind() == "method_definition" || child.kind() == "public_field_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    // Skip constructor for cohesion analysis (similar to __init__)
                    if name != "constructor" {
                        methods.push(MethodInfo {
                            name: name.to_string(),
                            start_byte: child.start_byte(),
                            end_byte: child.end_byte(),
                        });
                    }
                }
            }
        }
    }

    methods
}

// =============================================================================
// Java Class Extraction
// =============================================================================

/// Extract Java classes, interfaces, and enums with their methods
fn extract_java_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_java_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_java_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                if let Some(class_info) = extract_java_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into class body for nested classes
                if let Some(body) = child.child_by_field_name("body") {
                    extract_java_classes_recursive(body, source, classes);
                }
            }
            _ => {
                // Recurse into other nodes (program level, etc.)
                extract_java_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_java_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_java_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

fn extract_java_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        // method_declaration is a regular method; constructor_declaration is excluded
        // (similar to how TypeScript excludes "constructor")
        if child.kind() == "method_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    methods.push(MethodInfo {
                        name: name.to_string(),
                        start_byte: child.start_byte(),
                        end_byte: child.end_byte(),
                    });
                }
            }
        }
    }

    methods
}

// =============================================================================
// Go Struct Extraction
// =============================================================================

/// Extract Go structs with their receiver methods
fn extract_go_structs(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut structs: HashMap<String, ClassInfo> = HashMap::new();

    // First pass: collect all struct declarations
    collect_go_structs(root, source, &mut structs);

    // Second pass: collect receiver methods and associate with structs
    collect_go_methods(root, source, &mut structs);

    structs.into_values().collect()
}

fn collect_go_structs(
    node: tree_sitter::Node,
    source: &str,
    structs: &mut HashMap<String, ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "type_declaration" {
            // Look for struct type specs
            let mut type_cursor = child.walk();
            for type_child in child.children(&mut type_cursor) {
                if type_child.kind() == "type_spec" {
                    if let Some(name_node) = type_child.child_by_field_name("name") {
                        if let Some(type_node) = type_child.child_by_field_name("type") {
                            if type_node.kind() == "struct_type" {
                                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                                    let line = type_child.start_position().row + 1;
                                    structs.insert(
                                        name.to_string(),
                                        ClassInfo {
                                            name: name.to_string(),
                                            line,
                                            methods: Vec::new(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        // Recurse
        collect_go_structs(child, source, structs);
    }
}

fn collect_go_methods(
    node: tree_sitter::Node,
    source: &str,
    structs: &mut HashMap<String, ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            // Extract receiver type
            if let Some(receiver) = child.child_by_field_name("receiver") {
                if let Some(struct_name) = extract_go_receiver_type(&receiver, source) {
                    // Extract method name
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(method_name) = name_node.utf8_text(source.as_bytes()) {
                            if let Some(class_info) = structs.get_mut(&struct_name) {
                                class_info.methods.push(MethodInfo {
                                    name: method_name.to_string(),
                                    start_byte: child.start_byte(),
                                    end_byte: child.end_byte(),
                                });
                            }
                        }
                    }
                }
            }
        }
        // Recurse
        collect_go_methods(child, source, structs);
    }
}

fn extract_go_receiver_type(receiver: &tree_sitter::Node, source: &str) -> Option<String> {
    // receiver is parameter_list, find the type inside
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            if let Some(type_node) = child.child_by_field_name("type") {
                // Handle pointer receiver (*Type)
                if type_node.kind() == "pointer_type" {
                    if let Some(elem) = type_node.named_child(0) {
                        return elem.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
                    }
                } else {
                    return type_node.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

// =============================================================================
// Rust Struct Extraction
// =============================================================================

/// Extract Rust structs with their impl block methods
fn extract_rust_structs(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut structs: HashMap<String, ClassInfo> = HashMap::new();

    // First pass: collect all struct declarations
    collect_rust_structs(root, source, &mut structs);

    // Second pass: collect impl block methods and associate with structs
    collect_rust_impl_methods(root, source, &mut structs);

    structs.into_values().collect()
}

fn collect_rust_structs(
    node: tree_sitter::Node,
    source: &str,
    structs: &mut HashMap<String, ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "struct_item" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    let line = child.start_position().row + 1;
                    structs.insert(
                        name.to_string(),
                        ClassInfo {
                            name: name.to_string(),
                            line,
                            methods: Vec::new(),
                        },
                    );
                }
            }
        }
        // Recurse
        collect_rust_structs(child, source, structs);
    }
}

fn collect_rust_impl_methods(
    node: tree_sitter::Node,
    source: &str,
    structs: &mut HashMap<String, ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "impl_item" {
            // Get the type being implemented
            if let Some(type_node) = child.child_by_field_name("type") {
                if let Ok(type_name) = type_node.utf8_text(source.as_bytes()) {
                    let type_name = type_name.to_string();

                    // Get the body of the impl block
                    if let Some(body) = child.child_by_field_name("body") {
                        let mut body_cursor = body.walk();
                        for body_child in body.children(&mut body_cursor) {
                            if body_child.kind() == "function_item" {
                                // Skip associated functions (no self parameter).
                                // Only include instance methods (&self, &mut self, self)
                                // for LCOM4 analysis, since associated functions like
                                // new() and default() don't access self.field and would
                                // inflate LCOM4 by forming disconnected components.
                                if !rust_function_has_self(&body_child) {
                                    continue;
                                }
                                if let Some(name_node) = body_child.child_by_field_name("name") {
                                    if let Ok(method_name) = name_node.utf8_text(source.as_bytes()) {
                                        if let Some(class_info) = structs.get_mut(&type_name) {
                                            class_info.methods.push(MethodInfo {
                                                name: method_name.to_string(),
                                                start_byte: body_child.start_byte(),
                                                end_byte: body_child.end_byte(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Recurse
        collect_rust_impl_methods(child, source, structs);
    }
}

/// Check if a Rust function_item has a self parameter (&self, &mut self, or self).
///
/// In tree-sitter-rust, instance methods have a `self_parameter` node inside
/// the `parameters` field. Associated functions (like `fn new() -> Self`)
/// have no `self_parameter`.
fn rust_function_has_self(function_node: &tree_sitter::Node) -> bool {
    if let Some(params) = function_node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param_child in params.children(&mut cursor) {
            if param_child.kind() == "self_parameter" {
                return true;
            }
        }
    }
    false
}

// =============================================================================
// Ruby Class Extraction
// =============================================================================

/// Extract Ruby classes with their methods
fn extract_ruby_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_ruby_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_ruby_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class" => {
                if let Some(class_info) = extract_ruby_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into class body for nested classes
                if let Some(body) = child.child_by_field_name("body") {
                    extract_ruby_classes_recursive(body, source, classes);
                }
            }
            _ => {
                extract_ruby_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_ruby_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_ruby_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

/// Extract methods from a Ruby class body (body_statement node).
///
/// Ruby methods are `method` nodes (instance methods) and `singleton_method`
/// nodes (class methods like `self.foo`). For LCOM4 we include both since
/// singleton methods can also access class-level instance variables.
fn extract_ruby_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "method" || child.kind() == "singleton_method" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    methods.push(MethodInfo {
                        name: name.to_string(),
                        start_byte: child.start_byte(),
                        end_byte: child.end_byte(),
                    });
                }
            }
        }
    }

    methods
}

// =============================================================================
// C# Class Extraction
// =============================================================================

/// Extract C# classes, structs, and interfaces with their methods
fn extract_csharp_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_csharp_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_csharp_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "struct_declaration" | "interface_declaration" => {
                if let Some(class_info) = extract_csharp_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into class body for nested classes
                if let Some(body) = child.child_by_field_name("body") {
                    extract_csharp_classes_recursive(body, source, classes);
                }
            }
            _ => {
                extract_csharp_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_csharp_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_csharp_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

/// Extract methods from a C# class body (declaration_list node).
///
/// Only includes `method_declaration` nodes. Constructors
/// (`constructor_declaration`) are excluded from LCOM4 analysis,
/// consistent with how Java excludes constructors.
fn extract_csharp_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    methods.push(MethodInfo {
                        name: name.to_string(),
                        start_byte: child.start_byte(),
                        end_byte: child.end_byte(),
                    });
                }
            }
        }
    }

    methods
}

// =============================================================================
// Scala Class Extraction
// =============================================================================

/// Extract Scala classes, objects, and traits with their methods
fn extract_scala_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_scala_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_scala_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" | "object_definition" | "trait_definition" => {
                if let Some(class_info) = extract_scala_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into body for nested classes
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "template_body" || inner_child.kind() == "body" {
                        extract_scala_classes_recursive(inner_child, source, classes);
                    }
                }
            }
            _ => {
                extract_scala_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_scala_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    // Scala tree-sitter may use "name" field or have identifier as a direct child
    let name = node
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(source.as_bytes()).ok().map(|s| s.to_string()))
        .or_else(|| {
            // Fallback: find first identifier child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return child.utf8_text(source.as_bytes()).ok().map(|s| s.to_string());
                }
            }
            None
        })?;

    let line = node.start_position().row + 1;
    let methods = extract_scala_methods(node, source);

    Some(ClassInfo { name, line, methods })
}

/// Extract methods from a Scala class/object/trait.
///
/// Scala methods (`function_definition` / `function_declaration`) live inside
/// a `template_body` or `body` child of the class node.
fn extract_scala_methods(node: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "template_body" || child.kind() == "body" {
            let mut body_cursor = child.walk();
            for body_child in child.children(&mut body_cursor) {
                if body_child.kind() == "function_definition"
                    || body_child.kind() == "function_declaration"
                {
                    if let Some(name_node) = body_child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                            methods.push(MethodInfo {
                                name: name.to_string(),
                                start_byte: body_child.start_byte(),
                                end_byte: body_child.end_byte(),
                            });
                        }
                    }
                }
            }
        }
    }

    methods
}

// =============================================================================
// PHP Class Extraction
// =============================================================================

/// Extract PHP classes, interfaces, and traits with their methods
fn extract_php_classes(root: tree_sitter::Node, source: &str) -> Vec<ClassInfo> {
    let mut classes = Vec::new();
    extract_php_classes_recursive(root, source, &mut classes);
    classes
}

fn extract_php_classes_recursive(
    node: tree_sitter::Node,
    source: &str,
    classes: &mut Vec<ClassInfo>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                if let Some(class_info) = extract_php_class_info(&child, source) {
                    classes.push(class_info);
                }
                // Recurse into class body for nested classes
                if let Some(body) = child.child_by_field_name("body") {
                    extract_php_classes_recursive(body, source, classes);
                }
            }
            _ => {
                extract_php_classes_recursive(child, source, classes);
            }
        }
    }
}

fn extract_php_class_info(node: &tree_sitter::Node, source: &str) -> Option<ClassInfo> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source.as_bytes()).ok()?.to_string();
    let line = node.start_position().row + 1;

    let body = node.child_by_field_name("body")?;
    let methods = extract_php_methods(&body, source);

    Some(ClassInfo { name, line, methods })
}

/// Extract methods from a PHP class body (declaration_list node).
///
/// Only includes `method_declaration` nodes. Constructors (`__construct`)
/// are included as regular methods since PHP doesn't use a separate AST
/// node type for constructors.
fn extract_php_methods(body: &tree_sitter::Node, source: &str) -> Vec<MethodInfo> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        if child.kind() == "method_declaration" {
            if let Some(name_node) = child.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    methods.push(MethodInfo {
                        name: name.to_string(),
                        start_byte: child.start_byte(),
                        end_byte: child.end_byte(),
                    });
                }
            }
        }
    }

    methods
}

// =============================================================================
// LCOM4 Computation
// =============================================================================

/// Check if a method name is a dunder method (__name__)
fn is_dunder_method(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__")
}

/// Extract self.field accesses from a method's source text (Python)
pub(crate) fn extract_self_accesses(method_source: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Regex to match self.field_name patterns
    // Handles: self.field, self.field_name, self._private_field
    let pattern = Regex::new(r"self\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    for cap in pattern.captures_iter(method_source) {
        if let Some(field) = cap.get(1) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Extract this.field accesses from a method's source text (TypeScript/JavaScript)
fn extract_this_accesses(method_source: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Regex to match this.field_name patterns
    let pattern = Regex::new(r"this\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    for cap in pattern.captures_iter(method_source) {
        if let Some(field) = cap.get(1) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Extract field accesses from Go method (receiver.field)
fn extract_go_receiver_accesses(method_source: &str, receiver_name: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Match receiver.field patterns
    let pattern_str = format!(r"{}\.([a-zA-Z_][a-zA-Z0-9_]*)", regex::escape(receiver_name));
    if let Ok(pattern) = Regex::new(&pattern_str) {
        for cap in pattern.captures_iter(method_source) {
            if let Some(field) = cap.get(1) {
                fields.insert(field.as_str().to_string());
            }
        }
    }

    // Also match common Go receiver patterns like s.field, t.field
    let short_pattern = Regex::new(r"\b([a-z])\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
    for cap in short_pattern.captures_iter(method_source) {
        if let Some(field) = cap.get(2) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Extract field accesses from Rust method (self.field)
fn extract_rust_self_accesses(method_source: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Regex to match self.field_name patterns
    let pattern = Regex::new(r"self\.([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    for cap in pattern.captures_iter(method_source) {
        if let Some(field) = cap.get(1) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Extract field accesses from Ruby method (@field instance variables)
fn extract_ruby_instance_var_accesses(method_source: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Match all @-prefixed identifiers (including @@class_vars), then filter.
    // The regex crate does not support lookbehinds, so we capture an optional
    // second '@' and skip matches where it is present (@@class_var).
    let pattern = Regex::new(r"(@?)@([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    for cap in pattern.captures_iter(method_source) {
        // If group 1 captured an '@', this is a @@class_var -- skip it.
        if cap.get(1).is_some_and(|m| !m.as_str().is_empty()) {
            continue;
        }
        if let Some(field) = cap.get(2) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Extract field accesses from PHP method ($this->field)
fn extract_php_this_accesses(method_source: &str) -> HashSet<String> {
    let mut fields = HashSet::new();

    // Regex to match $this->field_name patterns
    let pattern = Regex::new(r"\$this->([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();

    for cap in pattern.captures_iter(method_source) {
        if let Some(field) = cap.get(1) {
            fields.insert(field.as_str().to_string());
        }
    }

    fields
}

/// Compute cohesion for a single class
fn compute_class_cohesion(
    class_info: &ClassInfo,
    source: &str,
    file_path: &Path,
    options: &CohesionOptions,
) -> ClassCohesion {
    // Filter out dunder methods if not including them
    let methods: Vec<&MethodInfo> = class_info
        .methods
        .iter()
        .filter(|m| options.include_dunder || !is_dunder_method(&m.name))
        .collect();

    let method_count = methods.len();

    // Special cases (T9 mitigation):
    // - 0 methods: LCOM4 = 0 (degenerate case, can't measure)
    // - 1 method: LCOM4 = 1 (single method is trivially cohesive)
    if method_count == 0 {
        return ClassCohesion {
            name: class_info.name.clone(),
            file: file_path.to_path_buf(),
            line: class_info.line,
            method_count: 0,
            field_count: 0,
            lcom4: 0,
            components: vec![],
            verdict: CohesionVerdict::Cohesive,
            split_suggestion: None,
        };
    }

    if method_count == 1 {
        let method = methods[0];
        let method_source = &source[method.start_byte..method.end_byte];
        let fields = extract_field_accesses(method_source, file_path);
        let field_vec: Vec<String> = fields.into_iter().collect();

        return ClassCohesion {
            name: class_info.name.clone(),
            file: file_path.to_path_buf(),
            line: class_info.line,
            method_count: 1,
            field_count: field_vec.len(),
            lcom4: 1,
            components: vec![ComponentInfo {
                methods: vec![method.name.clone()],
                fields: field_vec,
            }],
            verdict: CohesionVerdict::Cohesive,
            split_suggestion: None,
        };
    }

    // Extract field accesses for each method
    let method_fields: Vec<HashSet<String>> = methods
        .iter()
        .map(|m| {
            let method_source = &source[m.start_byte..m.end_byte];
            extract_field_accesses(method_source, file_path)
        })
        .collect();

    // Collect all unique fields
    let all_fields: HashSet<String> = method_fields.iter().flatten().cloned().collect();
    let field_count = all_fields.len();

    // If no methods access any fields, each method is its own component
    if all_fields.is_empty() {
        let lcom4 = method_count;
        let components: Vec<ComponentInfo> = methods
            .iter()
            .map(|m| ComponentInfo {
                methods: vec![m.name.clone()],
                fields: vec![],
            })
            .collect();

        let verdict = if lcom4 > options.low_cohesion_threshold {
            CohesionVerdict::SplitCandidate
        } else {
            CohesionVerdict::Cohesive
        };

        let split_suggestion = if verdict == CohesionVerdict::SplitCandidate {
            Some(format!(
                "Class has {} disconnected methods with no shared state",
                method_count
            ))
        } else {
            None
        };

        return ClassCohesion {
            name: class_info.name.clone(),
            file: file_path.to_path_buf(),
            line: class_info.line,
            method_count,
            field_count: 0,
            lcom4,
            components,
            verdict,
            split_suggestion,
        };
    }

    // Build Union-Find and connect methods that share fields
    let mut uf = UnionFind::new(method_count);

    for i in 0..method_count {
        for j in (i + 1)..method_count {
            // Check if methods i and j share any fields
            if !method_fields[i].is_disjoint(&method_fields[j]) {
                uf.union(i, j);
            }
        }
    }

    // Count connected components
    let lcom4 = uf.count_components();

    // Build component info
    let component_ids = uf.get_components();
    let mut component_map: HashMap<usize, (Vec<String>, HashSet<String>)> = HashMap::new();

    for (i, &comp_id) in component_ids.iter().enumerate() {
        let entry = component_map.entry(comp_id).or_insert_with(|| (Vec::new(), HashSet::new()));
        entry.0.push(methods[i].name.clone());
        entry.1.extend(method_fields[i].iter().cloned());
    }

    let components: Vec<ComponentInfo> = component_map
        .into_values()
        .map(|(methods, fields)| ComponentInfo {
            methods,
            fields: fields.into_iter().collect(),
        })
        .collect();

    let verdict = if lcom4 > options.low_cohesion_threshold {
        CohesionVerdict::SplitCandidate
    } else {
        CohesionVerdict::Cohesive
    };

    let split_suggestion = if verdict == CohesionVerdict::SplitCandidate {
        Some(format!(
            "Consider splitting into {} classes based on {} disconnected method groups",
            lcom4, lcom4
        ))
    } else {
        None
    };

    ClassCohesion {
        name: class_info.name.clone(),
        file: file_path.to_path_buf(),
        line: class_info.line,
        method_count,
        field_count,
        lcom4,
        components,
        verdict,
        split_suggestion,
    }
}

/// Extract field accesses based on file extension/language.
///
/// Uses AST-based extraction when possible, falling back to regex for
/// languages where tree-sitter parsing fails or returns no results.
fn extract_field_accesses(method_source: &str, file_path: &Path) -> HashSet<String> {
    let lang = Language::from_path(file_path);

    match lang {
        Some(language) => {
            extract_field_accesses_ast(method_source, language, None)
        }
        None => {
            // Unknown language: try regex fallback based on extension
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            match ext {
                "py" => extract_self_accesses(method_source),
                "ts" | "tsx" | "js" | "jsx" => extract_this_accesses(method_source),
                "go" => extract_go_receiver_accesses(method_source, ""),
                "rs" => extract_rust_self_accesses(method_source),
                "rb" => extract_ruby_instance_var_accesses(method_source),
                "cs" => extract_this_accesses(method_source),
                "scala" | "sc" => extract_this_accesses(method_source),
                "php" => extract_php_this_accesses(method_source),
                _ => HashSet::new(),
            }
        }
    }
}

/// AST-based field access extraction for all 18 supported languages.
///
/// Parses the method source text with tree-sitter and walks the AST looking
/// for field/member access nodes where the object is self/this/receiver.
///
/// Falls back to regex if AST parsing fails.
///
/// # Arguments
/// * `method_source` - Source code of the method body
/// * `language` - The programming language
/// * `receiver_name` - Optional receiver name for Go (e.g., "s" in `func (s *Server)`)
pub fn extract_field_accesses_ast(
    method_source: &str,
    language: Language,
    receiver_name: Option<&str>,
) -> HashSet<String> {
    use crate::security::ast_utils::field_access_info;

    let tree = match parse(method_source, language) {
        Ok(t) => t,
        Err(_) => {
            // Fallback to regex if AST parsing fails
            return extract_field_accesses_regex(method_source, language, receiver_name);
        }
    };

    let mut fields = HashSet::new();
    let source = method_source.as_bytes();
    let patterns = field_access_info(language);

    walk_and_extract_fields(
        &tree.root_node(),
        source,
        language,
        receiver_name,
        patterns,
        &mut fields,
    );

    // If AST found nothing but regex would have found something, fallback
    if fields.is_empty() {
        let regex_fields = extract_field_accesses_regex(method_source, language, receiver_name);
        if !regex_fields.is_empty() {
            return regex_fields;
        }
    }

    fields
}

/// Walk AST nodes recursively and extract field names from field access expressions.
fn walk_and_extract_fields(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    receiver_name: Option<&str>,
    patterns: &[crate::security::ast_utils::FieldAccessPattern],
    fields: &mut HashSet<String>,
) {
    use crate::security::ast_utils::{is_in_comment, is_in_string};

    let node_kind = node.kind();

    for pattern in patterns {
        if node_kind == pattern.node_kind {
            // Skip if inside a comment or string
            if is_in_comment(node, language) || is_in_string(node, language) {
                continue;
            }

            if let Some(field_name) = extract_field_from_pattern(
                node, source, language, receiver_name, pattern,
            ) {
                fields.insert(field_name);
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_and_extract_fields(&child, source, language, receiver_name, patterns, fields);
    }
}

/// Extract a field name from a node matching a FieldAccessPattern.
///
/// Returns Some(field_name) if the node is a self/this/receiver field access,
/// None otherwise.
fn extract_field_from_pattern(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    receiver_name: Option<&str>,
    _pattern: &crate::security::ast_utils::FieldAccessPattern,
) -> Option<String> {
    match language {
        Language::Python => extract_field_with_named_receiver(
            node,
            source,
            "object",
            "attribute",
            "self",
            "call",
            "function",
        ),
        Language::TypeScript | Language::JavaScript => extract_field_with_named_receiver(
            node,
            source,
            "object",
            "property",
            "this",
            "call_expression",
            "function",
        ),
        Language::Go => extract_go_field_access(node, source, receiver_name),
        Language::Rust => extract_field_with_named_receiver(
            node,
            source,
            "value",
            "field",
            "self",
            "call_expression",
            "function",
        ),
        Language::Java => extract_field_with_named_receiver(
            node,
            source,
            "object",
            "field",
            "this",
            "method_invocation",
            "object",
        ),
        Language::CSharp => extract_field_with_positional_receiver(
            node,
            source,
            0,
            "name",
            "this",
            "invocation_expression",
            0,
        ),
        Language::Cpp => extract_field_with_named_receiver(
            node,
            source,
            "argument",
            "field",
            "this",
            "call_expression",
            "function",
        ),
        Language::C => extract_c_field_access(node, source),
        Language::Ruby => extract_ruby_instance_field(node, source),
        Language::Kotlin => extract_navigation_field_access(
            node,
            source,
            "this_expression",
            "this",
            "call_expression",
        ),
        Language::Swift => extract_navigation_field_access(
            node,
            source,
            "self_expression",
            "self",
            "call_expression",
        ),
        Language::Scala => extract_scala_this_field_access(node, source),
        Language::Php => extract_php_this_field_access(node, source),
        Language::Lua | Language::Luau => extract_lua_self_field_access(node, source),
        Language::Elixir => extract_elixir_module_attribute(node, source),
        Language::Ocaml => None,
    }
}

fn extract_field_with_named_receiver(
    node: &tree_sitter::Node,
    source: &[u8],
    receiver_field: &str,
    field_name: &str,
    expected_receiver: &str,
    call_parent_kind: &str,
    call_target_field: &str,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let receiver = node.child_by_field_name(receiver_field)?;
    if node_text(&receiver, source) != expected_receiver {
        return None;
    }
    if parent_field_matches_node(node, call_parent_kind, call_target_field) {
        return None;
    }
    Some(node_text(&node.child_by_field_name(field_name)?, source).to_string())
}

fn extract_field_with_positional_receiver(
    node: &tree_sitter::Node,
    source: &[u8],
    receiver_index: usize,
    field_name: &str,
    expected_receiver: &str,
    call_parent_kind: &str,
    call_target_index: usize,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let receiver = node.child(receiver_index)?;
    if node_text(&receiver, source) != expected_receiver {
        return None;
    }
    if parent_child_matches_node(node, call_parent_kind, call_target_index) {
        return None;
    }
    Some(node_text(&node.child_by_field_name(field_name)?, source).to_string())
}

fn extract_go_field_access(
    node: &tree_sitter::Node,
    source: &[u8],
    receiver_name: Option<&str>,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let operand = node.child_by_field_name("operand")?;
    let operand_text = node_text(&operand, source);
    if !is_go_receiver_match(operand_text, receiver_name) {
        return None;
    }
    if parent_field_matches_node(node, "call_expression", "function") {
        return None;
    }
    Some(node_text(&node.child_by_field_name("field")?, source).to_string())
}

fn is_go_receiver_match(operand_text: &str, receiver_name: Option<&str>) -> bool {
    match receiver_name {
        Some("") | None => is_single_lowercase_identifier(operand_text),
        Some(recv) => operand_text == recv,
    }
}

fn is_single_lowercase_identifier(text: &str) -> bool {
    text.len() == 1
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
}

fn extract_c_field_access(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    node.child_by_field_name("argument")?;
    Some(node_text(&node.child_by_field_name("field")?, source).to_string())
}

fn extract_ruby_instance_field(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let text = node_text(node, source);
    if text.starts_with('@') && !text.starts_with("@@") {
        return Some(text[1..].to_string());
    }
    None
}

fn extract_navigation_field_access(
    node: &tree_sitter::Node,
    source: &[u8],
    self_kind: &str,
    self_text: &str,
    call_parent_kind: &str,
) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let target = node.child(0)?;
    if target.kind() != self_kind && node_text(&target, source) != self_text {
        return None;
    }

    for i in 1..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
            if parent_child_matches_node(node, call_parent_kind, 0) {
                return None;
            }
            return Some(node_text(&child, source).to_string());
        }
        if child.kind() == "navigation_suffix" {
            if let Some(identifier) = extract_suffix_identifier(&child, source) {
                if parent_child_matches_node(node, call_parent_kind, 0) {
                    return None;
                }
                return Some(identifier);
            }
        }
    }

    None
}

fn extract_suffix_identifier(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    for i in 0..node.child_count() {
        let child = node.child(i)?;
        if child.kind() == "simple_identifier" || child.kind() == "identifier" {
            return Some(node_text(&child, source).to_string());
        }
    }
    None
}

fn extract_scala_this_field_access(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let mut identifiers = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i)?;
        match child.kind() {
            "identifier" | "type_identifier" => {
                identifiers.push(node_text(&child, source).to_string());
            }
            "this" => identifiers.push("this".to_string()),
            _ => {}
        }
    }
    if identifiers.len() >= 2 && identifiers[0] == "this" {
        return Some(identifiers[1].clone());
    }
    None
}

fn extract_php_this_field_access(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let object = node.child_by_field_name("object")?;
    if node_text(&object, source) != "$this" {
        return None;
    }
    Some(node_text(&node.child_by_field_name("name")?, source).to_string())
}

fn extract_lua_self_field_access(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let first = node.child(0)?;
    if node_text(&first, source) != "self" {
        return None;
    }

    for i in (0..node.child_count()).rev() {
        let child = node.child(i)?;
        if child.kind() == "identifier" {
            if parent_child_matches_node(node, "function_call", 0) {
                return None;
            }
            return Some(node_text(&child, source).to_string());
        }
    }
    None
}

fn extract_elixir_module_attribute(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    use crate::security::ast_utils::node_text;

    let operator = node.child(0)?;
    if node_text(&operator, source) != "@" {
        return None;
    }
    let name_node = node.child(1)?;
    if name_node.kind() == "call" {
        return Some(node_text(&name_node.child(0)?, source).to_string());
    }
    Some(node_text(&name_node, source).to_string())
}

fn parent_field_matches_node(
    node: &tree_sitter::Node,
    parent_kind: &str,
    field_name: &str,
) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != parent_kind {
        return false;
    }
    parent
        .child_by_field_name(field_name)
        .is_some_and(|child| child.id() == node.id())
}

fn parent_child_matches_node(
    node: &tree_sitter::Node,
    parent_kind: &str,
    child_index: usize,
) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != parent_kind {
        return false;
    }
    parent
        .child(child_index)
        .is_some_and(|child| child.id() == node.id())
}

/// Regex-based field access extraction (fallback when AST parsing fails).
fn extract_field_accesses_regex(
    method_source: &str,
    language: Language,
    receiver_name: Option<&str>,
) -> HashSet<String> {
    match language {
        Language::Python => extract_self_accesses(method_source),
        Language::TypeScript | Language::JavaScript => extract_this_accesses(method_source),
        Language::Go => {
            let recv = receiver_name.unwrap_or("");
            extract_go_receiver_accesses(method_source, recv)
        }
        Language::Rust => extract_rust_self_accesses(method_source),
        Language::Ruby => extract_ruby_instance_var_accesses(method_source),
        Language::CSharp => extract_this_accesses(method_source),
        Language::Scala => extract_this_accesses(method_source),
        Language::Php => extract_php_this_accesses(method_source),
        _ => HashSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union_find_single_component() {
        let mut uf = UnionFind::new(3);
        uf.union(0, 1);
        uf.union(1, 2);
        assert_eq!(uf.count_components(), 1);
    }

    #[test]
    fn test_union_find_multiple_components() {
        let mut uf = UnionFind::new(4);
        uf.union(0, 1); // Component 1: {0, 1}
        uf.union(2, 3); // Component 2: {2, 3}
        assert_eq!(uf.count_components(), 2);
    }

    #[test]
    fn test_union_find_all_separate() {
        let mut uf = UnionFind::new(4);
        // No unions
        assert_eq!(uf.count_components(), 4);
    }

    #[test]
    fn test_union_find_empty() {
        let mut uf = UnionFind::new(0);
        assert_eq!(uf.count_components(), 0);
    }

    #[test]
    fn test_is_dunder_method() {
        assert!(is_dunder_method("__init__"));
        assert!(is_dunder_method("__str__"));
        assert!(is_dunder_method("__repr__"));
        assert!(!is_dunder_method("__private"));
        assert!(!is_dunder_method("public__"));
        assert!(!is_dunder_method("regular_method"));
    }

    #[test]
    fn test_extract_self_accesses() {
        let source = r#"
def method(self):
    self.value = 1
    self._private = 2
    x = self.other
    return self.value + self.other
"#;
        let fields = extract_self_accesses(source);
        assert!(fields.contains("value"));
        assert!(fields.contains("_private"));
        assert!(fields.contains("other"));
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_extract_this_accesses() {
        let source = r#"
getValue() {
    return this.value + this.other;
}
"#;
        let fields = extract_this_accesses(source);
        assert!(fields.contains("value"));
        assert!(fields.contains("other"));
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn test_cohesion_verdict_serialization() {
        let cohesive = CohesionVerdict::Cohesive;
        let split = CohesionVerdict::SplitCandidate;

        assert_eq!(
            serde_json::to_string(&cohesive).unwrap(),
            "\"cohesive\""
        );
        assert_eq!(
            serde_json::to_string(&split).unwrap(),
            "\"split_candidate\""
        );
    }

    // =========================================================================
    // AST-based field access extraction tests (all 18 languages)
    // =========================================================================

    #[test]
    fn test_ast_python_field_access() {
        let source = "def method(self):\n    x = self.name\n    y = self.age\n    z = self.name";
        let fields = extract_field_accesses_ast(source, Language::Python, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
        assert_eq!(fields.len(), 2, "Expected 2 unique fields, got {:?}", fields);
    }

    #[test]
    fn test_ast_python_excludes_method_calls() {
        let source = "def method(self):\n    self.do_thing()\n    x = self.name";
        let fields = extract_field_accesses_ast(source, Language::Python, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        // do_thing is a method call, not a field access
        assert!(!fields.contains("do_thing"), "Should not contain method call 'do_thing': {:?}", fields);
    }

    #[test]
    fn test_ast_python_string_not_detected() {
        let source = r#"def method(self):
    x = "self.fake_field"
    y = self.real_field"#;
        let fields = extract_field_accesses_ast(source, Language::Python, None);
        assert!(fields.contains("real_field"), "Expected 'real_field' in {:?}", fields);
        assert!(!fields.contains("fake_field"), "Should not detect field in string literal: {:?}", fields);
    }

    #[test]
    fn test_ast_python_comment_not_detected() {
        let source = "def method(self):\n    # self.commented_field\n    x = self.real_field";
        let fields = extract_field_accesses_ast(source, Language::Python, None);
        assert!(fields.contains("real_field"), "Expected 'real_field' in {:?}", fields);
        assert!(!fields.contains("commented_field"), "Should not detect field in comment: {:?}", fields);
    }

    #[test]
    fn test_ast_typescript_field_access() {
        let source = "method() {\n    const x = this.name;\n    const y = this.age;\n}";
        let fields = extract_field_accesses_ast(source, Language::TypeScript, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
        assert_eq!(fields.len(), 2, "Expected 2 fields, got {:?}", fields);
    }

    #[test]
    fn test_ast_javascript_field_access() {
        let source = "function method() {\n    const x = this.value;\n    this.count = 0;\n}";
        let fields = extract_field_accesses_ast(source, Language::JavaScript, None);
        assert!(fields.contains("value"), "Expected 'value' in {:?}", fields);
        assert!(fields.contains("count"), "Expected 'count' in {:?}", fields);
    }

    #[test]
    fn test_ast_go_field_access() {
        let source = "func (s *Server) method() {\n    x := s.host\n    y := s.port\n}";
        let fields = extract_field_accesses_ast(source, Language::Go, Some("s"));
        assert!(fields.contains("host"), "Expected 'host' in {:?}", fields);
        assert!(fields.contains("port"), "Expected 'port' in {:?}", fields);
    }

    #[test]
    fn test_ast_go_single_letter_receiver_heuristic() {
        // When no explicit receiver name, match single-letter lowercase identifiers
        let source = "func method() {\n    x := s.host\n    y := s.port\n}";
        let fields = extract_field_accesses_ast(source, Language::Go, None);
        assert!(fields.contains("host"), "Expected 'host' in {:?}", fields);
        assert!(fields.contains("port"), "Expected 'port' in {:?}", fields);
    }

    #[test]
    fn test_ast_rust_field_access() {
        let source = "fn method(&self) {\n    let x = self.name;\n    let y = self.age;\n}";
        let fields = extract_field_accesses_ast(source, Language::Rust, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
        assert_eq!(fields.len(), 2, "Expected 2 fields, got {:?}", fields);
    }

    #[test]
    fn test_ast_java_field_access() {
        let source = "void method() {\n    int x = this.value;\n    this.count = 0;\n}";
        let fields = extract_field_accesses_ast(source, Language::Java, None);
        assert!(fields.contains("value"), "Expected 'value' in {:?}", fields);
        assert!(fields.contains("count"), "Expected 'count' in {:?}", fields);
    }

    #[test]
    fn test_ast_kotlin_field_access() {
        let source = "fun method() {\n    val x = this.name\n    this.age = 25\n}";
        let fields = extract_field_accesses_ast(source, Language::Kotlin, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_swift_field_access() {
        // Swift tree-sitter now works with tree-sitter 0.25.0 (ABI v15 support)
        let source = "func method() {\n    let x = self.name\n    self.age = 25\n}";
        let fields = extract_field_accesses_ast(source, Language::Swift, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_csharp_field_access() {
        let source = "void Method() {\n    var x = this.Name;\n    this.Count = 0;\n}";
        let fields = extract_field_accesses_ast(source, Language::CSharp, None);
        assert!(fields.contains("Name"), "Expected 'Name' in {:?}", fields);
        assert!(fields.contains("Count"), "Expected 'Count' in {:?}", fields);
    }

    #[test]
    fn test_ast_cpp_field_access() {
        let source = "void method() {\n    int x = this->name;\n    this->age = 25;\n}";
        let fields = extract_field_accesses_ast(source, Language::Cpp, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_c_field_access() {
        let source = "void method(struct Server* s) {\n    int x = s->host;\n    s->port = 80;\n}";
        let fields = extract_field_accesses_ast(source, Language::C, None);
        assert!(fields.contains("host"), "Expected 'host' in {:?}", fields);
        assert!(fields.contains("port"), "Expected 'port' in {:?}", fields);
    }

    #[test]
    fn test_ast_ruby_instance_variable() {
        let source = "def method\n    x = @name\n    @age = 25\nend";
        let fields = extract_field_accesses_ast(source, Language::Ruby, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_scala_field_access() {
        let source = "def method(): Unit = {\n    val x = this.name\n    this.age = 25\n}";
        let fields = extract_field_accesses_ast(source, Language::Scala, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_php_field_access() {
        let source = "<?php\nfunction method() {\n    $x = $this->name;\n    $this->age = 25;\n}";
        let fields = extract_field_accesses_ast(source, Language::Php, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_lua_field_access() {
        let source = "function MyClass:method()\n    local x = self.name\n    self.age = 25\nend";
        let fields = extract_field_accesses_ast(source, Language::Lua, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_luau_field_access() {
        let source = "function MyClass:method()\n    local x = self.name\n    self.age = 25\nend";
        let fields = extract_field_accesses_ast(source, Language::Luau, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_elixir_module_attribute() {
        let source = "defmodule MyModule do\n  @name \"test\"\n  @age 25\nend";
        let fields = extract_field_accesses_ast(source, Language::Elixir, None);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
    }

    #[test]
    fn test_ast_ocaml_no_self() {
        // OCaml is functional - no self concept, should return empty
        let source = "let method x = x.name + x.age";
        let fields = extract_field_accesses_ast(source, Language::Ocaml, None);
        // OCaml has no self/this - should return empty or record field accesses
        // For LCOM4 purposes, OCaml classes are rare, so empty is fine
        assert!(fields.is_empty(), "OCaml should return empty set for LCOM4: {:?}", fields);
    }

    #[test]
    fn test_ast_regex_fallback_on_parse_failure() {
        // Test that regex fallback works when AST parsing would fail
        // Python regex should still work even with invalid syntax wrapping
        let source = "self.name = 1\nself.age = 2";
        let fields = extract_field_accesses_regex(source, Language::Python, None);
        assert!(fields.contains("name"), "Regex fallback should find 'name': {:?}", fields);
        assert!(fields.contains("age"), "Regex fallback should find 'age': {:?}", fields);
    }

    // =========================================================================
    // Java class extraction tests
    // =========================================================================

    #[test]
    fn test_extract_java_classes_basic() {
        let source = r#"
public class MyService {
    private String name;

    public MyService(String name) {
        this.name = name;
    }

    public String getName() {
        return this.name;
    }

    public void setName(String name) {
        this.name = name;
    }
}
"#;
        let tree = parse(source, Language::Java).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Java);
        assert_eq!(classes.len(), 1, "Expected 1 class, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(classes[0].name, "MyService");
        // constructor should be excluded, leaving getName and setName
        let non_ctor_methods: Vec<_> = classes[0].methods.iter().filter(|m| m.name != "MyService").collect();
        assert_eq!(non_ctor_methods.len(), 2, "Expected 2 non-constructor methods, got {:?}", classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>());
    }

    #[test]
    fn test_extract_java_classes_multiple() {
        let source = r#"
public class First {
    public void doA() {}
}

class Second {
    public void doB() {}
}
"#;
        let tree = parse(source, Language::Java).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Java);
        assert_eq!(classes.len(), 2, "Expected 2 classes, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"First"), "Expected 'First' in {:?}", names);
        assert!(names.contains(&"Second"), "Expected 'Second' in {:?}", names);
    }

    #[test]
    fn test_extract_java_interface_and_enum() {
        let source = r#"
interface Describable {
    String describe();
}

enum Color {
    RED, GREEN, BLUE;

    public String label() {
        return this.name();
    }
}
"#;
        let tree = parse(source, Language::Java).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Java);
        // Should find at least Color enum (has a method), and possibly Describable interface
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Color"), "Expected 'Color' enum in {:?}", names);
    }

    #[test]
    fn test_extract_java_methods_exclude_constructors() {
        let source = r#"
public class Widget {
    private int size;

    public Widget() {
        this.size = 0;
    }

    public Widget(int size) {
        this.size = size;
    }

    public int getSize() {
        return this.size;
    }
}
"#;
        let tree = parse(source, Language::Java).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Java);
        assert_eq!(classes.len(), 1);
        let widget = &classes[0];
        // Constructors should be included as MethodInfo (LCOM4 filters dunders, not constructors per se,
        // but for Java we should include method_declaration only, not constructor_declaration)
        // method_declaration: getSize; constructor_declaration: Widget(), Widget(int)
        // We expect only getSize from method_declaration
        let method_names: Vec<&str> = widget.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(method_names.contains(&"getSize"), "Expected 'getSize' in {:?}", method_names);
        // Constructors are separate AST nodes (constructor_declaration) -- we don't extract them
        assert!(!method_names.contains(&"Widget"), "Constructors should not be extracted: {:?}", method_names);
    }

    // =========================================================================
    // Rust struct extraction and LCOM4 tests
    // =========================================================================

    #[test]
    fn test_extract_rust_structs_basic() {
        let source = r#"
pub struct Foo {
    bar: String,
    baz: i32,
}

impl Foo {
    pub fn get_bar(&self) -> &str {
        &self.bar
    }
    pub fn get_baz(&self) -> i32 {
        self.baz
    }
    pub fn set_bar(&mut self, val: String) {
        self.bar = val;
    }
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Rust);
        assert_eq!(classes.len(), 1, "Expected 1 struct, got {}", classes.len());
        assert_eq!(classes[0].name, "Foo");
        assert_eq!(
            classes[0].methods.len(), 3,
            "Expected 3 methods, got {}: {:?}",
            classes[0].methods.len(),
            classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_rust_lcom4_cohesive_struct() {
        // All methods access overlapping fields => LCOM4 = 1
        let source = r#"
pub struct Foo {
    bar: String,
    baz: i32,
}

impl Foo {
    pub fn get_bar(&self) -> &str {
        &self.bar
    }
    pub fn get_baz(&self) -> i32 {
        self.baz
    }
    pub fn set_bar(&mut self, val: String) {
        self.bar = val;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("foo.rs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let foo = results.iter().find(|c| c.name == "Foo").unwrap();
        assert_eq!(foo.method_count, 3, "Expected 3 methods, got {}", foo.method_count);
        assert!(foo.field_count > 0, "Expected fields to be detected, got {}", foo.field_count);
        // get_bar accesses bar, get_baz accesses baz, set_bar accesses bar
        // bar connects get_bar and set_bar; they're in one component
        // baz is only in get_baz => separate component
        // So LCOM4 should be 2 (two components: {get_bar, set_bar} and {get_baz})
        assert_eq!(foo.lcom4, 2, "Expected LCOM4=2 (two components), got {}", foo.lcom4);
    }

    #[test]
    fn test_rust_lcom4_fully_cohesive() {
        // All methods share the same field => LCOM4 = 1
        let source = r#"
pub struct Counter {
    count: i32,
}

impl Counter {
    pub fn increment(&mut self) {
        self.count += 1;
    }
    pub fn decrement(&mut self) {
        self.count -= 1;
    }
    pub fn get(&self) -> i32 {
        self.count
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("counter.rs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let counter = results.iter().find(|c| c.name == "Counter").unwrap();
        assert_eq!(counter.method_count, 3);
        assert_eq!(counter.field_count, 1, "Expected 1 field (count), got {}", counter.field_count);
        assert_eq!(counter.lcom4, 1, "Fully cohesive class should have LCOM4=1, got {}", counter.lcom4);
    }

    #[test]
    fn test_rust_multiple_structs() {
        let source = r#"
pub struct Alpha {
    x: i32,
}

impl Alpha {
    pub fn get_x(&self) -> i32 {
        self.x
    }
}

pub struct Beta {
    y: String,
}

impl Beta {
    pub fn get_y(&self) -> &str {
        &self.y
    }
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Rust);
        assert_eq!(classes.len(), 2, "Expected 2 structs, got {}", classes.len());
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Alpha"), "Expected 'Alpha' in {:?}", names);
        assert!(names.contains(&"Beta"), "Expected 'Beta' in {:?}", names);
    }

    #[test]
    fn test_rust_struct_with_multiple_impl_blocks() {
        let source = r#"
pub struct MyType {
    a: i32,
    b: String,
}

impl MyType {
    pub fn get_a(&self) -> i32 {
        self.a
    }
}

impl MyType {
    pub fn get_b(&self) -> &str {
        &self.b
    }
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Rust);
        assert_eq!(classes.len(), 1, "Expected 1 struct (merged impl blocks), got {}", classes.len());
        let my_type = &classes[0];
        assert_eq!(
            my_type.methods.len(), 2,
            "Expected 2 methods from merged impl blocks, got {}: {:?}",
            my_type.methods.len(),
            my_type.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_rust_trait_impl_methods_included() {
        // impl Default for X should count as methods of X
        let source = r#"
pub struct Config {
    name: String,
    count: i32,
}

impl Config {
    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: String::new(),
            count: 0,
        }
    }
}
"#;
        let tree = parse(source, Language::Rust).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Rust);
        assert_eq!(classes.len(), 1, "Expected 1 struct");
        let config = &classes[0];
        let method_names: Vec<&str> = config.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(
            method_names.contains(&"get_name"),
            "Expected 'get_name' in methods: {:?}", method_names
        );
        // default() from impl Default is a static/associated function (no self parameter),
        // so it should NOT be included in instance methods for LCOM4 analysis.
        assert!(
            !method_names.contains(&"default"),
            "Static 'default()' should be excluded from instance methods: {:?}", method_names
        );
        assert_eq!(
            config.methods.len(), 1,
            "Expected 1 instance method (get_name only, default() excluded), got {}: {:?}",
            config.methods.len(), method_names
        );
    }

    #[test]
    fn test_rust_static_method_not_inflating_lcom4() {
        // Static methods (no self parameter) shouldn't inflate LCOM4
        // because they don't participate in field sharing
        let source = r#"
pub struct Builder {
    name: String,
    count: i32,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            count: 0,
        }
    }
    pub fn get_name(&self) -> &str {
        &self.name
    }
    pub fn get_count(&self) -> i32 {
        self.count
    }
    pub fn inc(&mut self) {
        self.count += 1;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("builder.rs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        let builder = results.iter().find(|c| c.name == "Builder").unwrap();
        // new() is a static method (no self parameter), it should be excluded from LCOM4.
        // Only instance methods (with &self or &mut self) participate in field sharing.
        // After fix: method_count should be 3 (get_name, get_count, inc)
        // LCOM4 should be 2: {get_name} accesses name, {get_count, inc} access count
        assert_eq!(
            builder.method_count, 3,
            "Expected 3 instance methods (excluding static new()), got {}",
            builder.method_count
        );
        assert_eq!(
            builder.lcom4, 2,
            "Expected LCOM4=2 (two components: {{get_name}} and {{get_count, inc}}), got {}",
            builder.lcom4
        );
    }

    #[test]
    fn test_rust_field_accesses_detected_in_methods() {
        // Verify that field accesses are detected when extracting from a Rust method
        let method_source = r#"pub fn process(&mut self) {
    let x = self.name;
    self.count += 1;
    self.data.push(x);
}"#;
        let fields = extract_rust_self_accesses(method_source);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("count"), "Expected 'count' in {:?}", fields);
        assert!(fields.contains("data"), "Expected 'data' in {:?}", fields);
        assert_eq!(fields.len(), 3, "Expected 3 fields, got {:?}", fields);
    }

    #[test]
    fn test_rust_analyze_file_cohesion_on_coupling_rs() {
        // coupling.rs has many structs (CouplingReport, ModuleCoupling, etc.)
        // but they are data-only structs with impl Default (a static method).
        // After the self-filtering fix, these structs have 0 instance methods,
        // which is correct since they have no self-accessing methods.
        let coupling_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/quality/coupling.rs");
        if coupling_path.exists() {
            let options = CohesionOptions::default();
            let results = analyze_file_cohesion(&coupling_path, &options).unwrap();
            // The structs should be found, even though they have 0 instance methods
            let names: Vec<&str> = results.iter().map(|c| c.name.as_str()).collect();
            assert!(
                results.len() >= 3,
                "Expected at least 3 structs in coupling.rs, got {}: {:?}",
                results.len(),
                names
            );
            // All should have 0 instance methods (only default() which is static)
            for r in &results {
                assert_eq!(
                    r.method_count, 0,
                    "Struct {} should have 0 instance methods (default() is static), got {}",
                    r.name, r.method_count
                );
            }
        }
    }

    #[test]
    fn test_rust_analyze_file_cohesion_on_real_file() {
        // Test with a more realistic Rust file that has pub visibility, derives, etc.
        let source = r#"
use std::collections::HashMap;

/// A report structure
#[derive(Debug, Clone)]
pub struct Report {
    pub title: String,
    pub items: Vec<String>,
    pub metadata: HashMap<String, String>,
}

impl Report {
    pub fn new(title: String) -> Self {
        Self {
            title,
            items: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    pub fn add_item(&mut self, item: String) {
        self.items.push(item);
    }

    pub fn get_title(&self) -> &str {
        &self.title
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn set_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("report.rs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected structs to be found in realistic Rust file");
        let report = results.iter().find(|c| c.name == "Report")
            .expect("Expected 'Report' struct to be found");
        // new() is a static method (no self), add_item uses self.items,
        // get_title uses self.title, item_count uses self.items,
        // set_metadata uses self.metadata
        assert!(report.method_count >= 4, "Expected at least 4 methods, got {}", report.method_count);
        assert!(report.field_count >= 2, "Expected at least 2 fields, got {}", report.field_count);
    }

    // =========================================================================
    // Ruby class extraction tests
    // =========================================================================

    #[test]
    fn test_extract_ruby_classes_basic() {
        let source = r#"
class Dog
  def initialize(name, age)
    @name = name
    @age = age
  end

  def bark
    @name
  end

  def age
    @age
  end
end
"#;
        let tree = parse(source, Language::Ruby).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Ruby);
        assert_eq!(classes.len(), 1, "Expected 1 class, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(classes[0].name, "Dog");
        // initialize, bark, age = 3 methods
        assert_eq!(
            classes[0].methods.len(), 3,
            "Expected 3 methods, got {:?}",
            classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_ruby_classes_with_inheritance() {
        let source = r#"
class Animal
  def speak
    @sound
  end
end

class Cat < Animal
  def purr
    @purr_volume
  end
end
"#;
        let tree = parse(source, Language::Ruby).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Ruby);
        assert_eq!(classes.len(), 2, "Expected 2 classes, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Animal"), "Expected 'Animal' in {:?}", names);
        assert!(names.contains(&"Cat"), "Expected 'Cat' in {:?}", names);
    }

    #[test]
    fn test_ruby_lcom4_cohesive_class() {
        // All methods access the same field => LCOM4 = 1
        let source = r#"
class Counter
  def initialize
    @count = 0
  end

  def increment
    @count += 1
  end

  def decrement
    @count -= 1
  end

  def value
    @count
  end
end
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("counter.rb");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let counter = results.iter().find(|c| c.name == "Counter").unwrap();
        assert_eq!(counter.method_count, 4, "Expected 4 methods, got {}", counter.method_count);
        assert_eq!(counter.lcom4, 1, "Fully cohesive class should have LCOM4=1, got {}", counter.lcom4);
    }

    #[test]
    fn test_ruby_lcom4_split_candidate() {
        // Two groups of methods accessing different fields => LCOM4 = 2
        let source = r#"
class Mixed
  def get_name
    @name
  end

  def set_name(n)
    @name = n
  end

  def get_age
    @age
  end

  def set_age(a)
    @age = a
  end
end
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("mixed.rb");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let mixed = results.iter().find(|c| c.name == "Mixed").unwrap();
        assert_eq!(mixed.method_count, 4, "Expected 4 methods, got {}", mixed.method_count);
        // name group: {get_name, set_name}, age group: {get_age, set_age}
        assert_eq!(mixed.lcom4, 2, "Expected LCOM4=2 (two components), got {}", mixed.lcom4);
    }

    // =========================================================================
    // C# class extraction tests
    // =========================================================================

    #[test]
    fn test_extract_csharp_classes_basic() {
        let source = r#"
class UserService {
    private string name;

    public string GetName() {
        return this.name;
    }

    public void SetName(string n) {
        this.name = n;
    }
}
"#;
        let tree = parse(source, Language::CSharp).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::CSharp);
        assert_eq!(classes.len(), 1, "Expected 1 class, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(classes[0].name, "UserService");
        assert_eq!(
            classes[0].methods.len(), 2,
            "Expected 2 methods, got {:?}",
            classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_csharp_classes_multiple() {
        let source = r#"
class First {
    public void DoA() {}
}

class Second {
    public void DoB() {}
}
"#;
        let tree = parse(source, Language::CSharp).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::CSharp);
        assert_eq!(classes.len(), 2, "Expected 2 classes, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"First"), "Expected 'First' in {:?}", names);
        assert!(names.contains(&"Second"), "Expected 'Second' in {:?}", names);
    }

    #[test]
    fn test_csharp_lcom4_cohesive_class() {
        let source = r#"
class Counter {
    private int count;

    public void Increment() {
        this.count += 1;
    }

    public void Decrement() {
        this.count -= 1;
    }

    public int GetValue() {
        return this.count;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("counter.cs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let counter = results.iter().find(|c| c.name == "Counter").unwrap();
        assert_eq!(counter.method_count, 3, "Expected 3 methods, got {}", counter.method_count);
        assert_eq!(counter.lcom4, 1, "Fully cohesive class should have LCOM4=1, got {}", counter.lcom4);
    }

    #[test]
    fn test_csharp_lcom4_split_candidate() {
        let source = r#"
class Mixed {
    private string name;
    private int age;

    public string GetName() {
        return this.name;
    }

    public void SetName(string n) {
        this.name = n;
    }

    public int GetAge() {
        return this.age;
    }

    public void SetAge(int a) {
        this.age = a;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("mixed.cs");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let mixed = results.iter().find(|c| c.name == "Mixed").unwrap();
        assert_eq!(mixed.method_count, 4, "Expected 4 methods, got {}", mixed.method_count);
        assert_eq!(mixed.lcom4, 2, "Expected LCOM4=2 (two components), got {}", mixed.lcom4);
    }

    // =========================================================================
    // Scala class extraction tests
    // =========================================================================

    #[test]
    fn test_extract_scala_classes_basic() {
        let source = r#"
class UserService {
  def getName(): String = {
    this.name
  }

  def setName(n: String): Unit = {
    this.name = n
  }
}
"#;
        let tree = parse(source, Language::Scala).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Scala);
        assert_eq!(classes.len(), 1, "Expected 1 class, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(classes[0].name, "UserService");
        assert_eq!(
            classes[0].methods.len(), 2,
            "Expected 2 methods, got {:?}",
            classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_scala_object_and_trait() {
        let source = r#"
object Config {
  def getValue(): String = {
    this.value
  }
}

trait Describable {
  def describe(): String
}
"#;
        let tree = parse(source, Language::Scala).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Scala);
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Config"), "Expected 'Config' object in {:?}", names);
        assert!(names.contains(&"Describable"), "Expected 'Describable' trait in {:?}", names);
    }

    #[test]
    fn test_scala_lcom4_cohesive_class() {
        let source = r#"
class Counter {
  def increment(): Unit = {
    this.count += 1
  }

  def decrement(): Unit = {
    this.count -= 1
  }

  def getValue(): Int = {
    this.count
  }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("counter.scala");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let counter = results.iter().find(|c| c.name == "Counter").unwrap();
        assert_eq!(counter.method_count, 3, "Expected 3 methods, got {}", counter.method_count);
        assert_eq!(counter.lcom4, 1, "Fully cohesive class should have LCOM4=1, got {}", counter.lcom4);
    }

    #[test]
    fn test_scala_lcom4_split_candidate() {
        let source = r#"
class Mixed {
  def getName(): String = {
    this.name
  }

  def setName(n: String): Unit = {
    this.name = n
  }

  def getAge(): Int = {
    this.age
  }

  def setAge(a: Int): Unit = {
    this.age = a
  }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("mixed.scala");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let mixed = results.iter().find(|c| c.name == "Mixed").unwrap();
        assert_eq!(mixed.method_count, 4, "Expected 4 methods, got {}", mixed.method_count);
        assert_eq!(mixed.lcom4, 2, "Expected LCOM4=2 (two components), got {}", mixed.lcom4);
    }

    // =========================================================================
    // PHP class extraction tests
    // =========================================================================

    #[test]
    fn test_extract_php_classes_basic() {
        let source = r#"<?php
class UserService {
    private $name;

    public function getName() {
        return $this->name;
    }

    public function setName($n) {
        $this->name = $n;
    }
}
"#;
        let tree = parse(source, Language::Php).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Php);
        assert_eq!(classes.len(), 1, "Expected 1 class, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        assert_eq!(classes[0].name, "UserService");
        assert_eq!(
            classes[0].methods.len(), 2,
            "Expected 2 methods, got {:?}",
            classes[0].methods.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_php_classes_multiple() {
        let source = r#"<?php
class First {
    public function doA() {}
}

class Second {
    public function doB() {}
}
"#;
        let tree = parse(source, Language::Php).unwrap();
        let classes = extract_classes(tree.root_node(), source, Language::Php);
        assert_eq!(classes.len(), 2, "Expected 2 classes, got {:?}", classes.iter().map(|c| &c.name).collect::<Vec<_>>());
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"First"), "Expected 'First' in {:?}", names);
        assert!(names.contains(&"Second"), "Expected 'Second' in {:?}", names);
    }

    #[test]
    fn test_php_lcom4_cohesive_class() {
        let source = r#"<?php
class Counter {
    private $count;

    public function increment() {
        $this->count += 1;
    }

    public function decrement() {
        $this->count -= 1;
    }

    public function getValue() {
        return $this->count;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("counter.php");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let counter = results.iter().find(|c| c.name == "Counter").unwrap();
        assert_eq!(counter.method_count, 3, "Expected 3 methods, got {}", counter.method_count);
        assert_eq!(counter.lcom4, 1, "Fully cohesive class should have LCOM4=1, got {}", counter.lcom4);
    }

    #[test]
    fn test_php_lcom4_split_candidate() {
        let source = r#"<?php
class Mixed {
    private $name;
    private $age;

    public function getName() {
        return $this->name;
    }

    public function setName($n) {
        $this->name = $n;
    }

    public function getAge() {
        return $this->age;
    }

    public function setAge($a) {
        $this->age = $a;
    }
}
"#;
        let test_dir = tempfile::tempdir().unwrap();
        let file_path = test_dir.path().join("mixed.php");
        std::fs::write(&file_path, source).unwrap();

        let options = CohesionOptions::default();
        let results = analyze_file_cohesion(&file_path, &options).unwrap();
        assert!(!results.is_empty(), "Expected at least 1 class in results");
        let mixed = results.iter().find(|c| c.name == "Mixed").unwrap();
        assert_eq!(mixed.method_count, 4, "Expected 4 methods, got {}", mixed.method_count);
        assert_eq!(mixed.lcom4, 2, "Expected LCOM4=2 (two components), got {}", mixed.lcom4);
    }

    #[test]
    fn test_extract_ruby_instance_var_no_panic() {
        // The regex must not panic (lookbehinds are unsupported in the regex crate).
        let source = "@name = 'Alice'\n@@class_var = 1\n@age = 30";
        let fields = extract_ruby_instance_var_accesses(source);
        assert!(fields.contains("name"), "Expected 'name' in {:?}", fields);
        assert!(fields.contains("age"), "Expected 'age' in {:?}", fields);
        // @@class_var should NOT produce a match for "class_var" as an instance var
        assert!(!fields.contains("class_var"), "@@class_var should not be matched as instance var, got {:?}", fields);
    }

    #[test]
    fn test_extract_ruby_instance_var_inline() {
        // Instance variable in expressions
        let source = "puts @value + @@counter";
        let fields = extract_ruby_instance_var_accesses(source);
        assert!(fields.contains("value"), "Expected 'value' in {:?}", fields);
        assert!(!fields.contains("counter"), "@@counter should not match as instance var, got {:?}", fields);
    }
}
