//! Mutability Command - Variable/Parameter Mutation Tracking
//!
//! Analyzes mutability of variables, parameters, and fields in Python code.
//!
//! # Features
//!
//! - **M1 Variable Mutability**: Track variable reassignments and in-place mutations
//! - **M2 Parameter Mutations**: Detect when function parameters are mutated
//! - **M3 Field Mutability**: Analyze class field mutations (with --include-fields)
//! - **M4 Collection Mutations**: Detect collection method calls (append, update, etc.)
//! - **M5 Alias Propagation**: Track mutations through aliases (with --include-aliases)
//! - **M8 Type Constraints**: Generate immutable type suggestions (with --constraints)
//!
//! # Example
//!
//! ```bash
//! # Analyze a single file
//! tldr mutability src/utils.py
//!
//! # Analyze a specific function
//! tldr mutability src/utils.py process_data
//!
//! # Include class field analysis
//! tldr mutability src/model.py --include-fields
//!
//! # Skip collection mutation tracking
//! tldr mutability src/utils.py --no-collections
//!
//! # Generate type constraints for unmutated parameters
//! tldr mutability src/utils.py --constraints
//!
//! # Summary only mode
//! tldr mutability src/utils.py --summary
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use clap::Args;
use tree_sitter::{Node, Parser};

use super::error::{PatternsError, PatternsResult};
use super::types::{
    ClassMutability, CollectionMutation, FieldMutability, FunctionMutability, MutabilityReport,
    MutabilitySummary, OutputFormat, ParameterMutability, VariableMutability,
};
use super::validation::{read_file_safe, validate_file_path, validate_file_path_in_project};
use tldr_core::ast::parser::ParserPool;
use tldr_core::types::Language;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze mutability of variables, parameters, and fields.
#[derive(Debug, Args)]
pub struct MutabilityArgs {
    /// File to analyze
    pub file: PathBuf,

    /// Optional function name to analyze (analyzes all if not specified)
    #[arg(name = "function")]
    pub function_name: Option<String>,

    /// Include class field mutability analysis
    #[arg(long)]
    pub include_fields: bool,

    /// Include alias propagation analysis
    #[arg(long)]
    pub include_aliases: bool,

    /// Skip collection mutation tracking
    #[arg(long)]
    pub no_collections: bool,

    /// Generate type constraints for unmutated parameters
    #[arg(long)]
    pub constraints: bool,

    /// Summary only mode - only output statistics
    #[arg(long)]
    pub summary: bool,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(long = "output", short = 'o', hide = true, default_value = "json", value_enum)]
    pub output_format: OutputFormat,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,
}

impl MutabilityArgs {
    /// Run the mutability analysis command
    pub fn run(&self) -> anyhow::Result<()> {
        run(self)
    }
}

// =============================================================================
// Constants - Mutating Methods
// =============================================================================

/// Collection methods that mutate in-place
pub const MUTATING_METHODS: &[&str] = &[
    // List mutations
    "append",
    "extend",
    "insert",
    "remove",
    "pop",
    "clear",
    "sort",
    "reverse",
    // Dict mutations
    "update",
    "setdefault",
    "popitem",
    // Set mutations
    "add",
    "discard",
    "difference_update",
    "intersection_update",
    "symmetric_difference_update",
];

/// Immutable alternatives for type hint suggestions
const IMMUTABLE_ALTERNATIVES: &[(&str, &str)] = &[
    ("list", "Sequence"),
    ("List", "Sequence"),
    ("dict", "Mapping"),
    ("Dict", "Mapping"),
    ("set", "AbstractSet"),
    ("Set", "AbstractSet"),
];

// =============================================================================
// Analysis Types
// =============================================================================

/// Tracks variable assignments within a function
#[derive(Debug, Default)]
pub struct VariableTracker {
    /// Map from variable name to list of assignment line numbers
    pub assignments: HashMap<String, Vec<u32>>,
    /// Map from variable name to list of mutation line numbers (augmented assignments)
    pub mutations: HashMap<String, Vec<u32>>,
}

impl VariableTracker {
    fn new() -> Self {
        Self::default()
    }

    fn record_assignment(&mut self, name: &str, line: u32) {
        self.assignments.entry(name.to_string()).or_default().push(line);
    }

    fn record_mutation(&mut self, name: &str, line: u32) {
        self.mutations.entry(name.to_string()).or_default().push(line);
    }

    /// Build VariableMutability results
    fn to_variable_mutabilities(&self) -> Vec<VariableMutability> {
        let mut all_names: HashSet<&String> = self.assignments.keys().collect();
        all_names.extend(self.mutations.keys());

        let mut results: Vec<VariableMutability> = all_names
            .into_iter()
            .map(|name| {
                let assignments = self.assignments.get(name).map_or(0, |v| v.len()) as u32;
                let mutations = self.mutations.get(name).map_or(0, |v| v.len()) as u32;
                let mutable = assignments > 1 || mutations > 0;

                VariableMutability {
                    name: name.clone(),
                    mutable,
                    reassignments: if assignments > 0 { assignments - 1 } else { 0 },
                    mutations,
                }
            })
            .collect();

        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }
}

/// Detects parameter mutations
#[derive(Debug, Default)]
pub struct ParameterMutationDetector {
    /// Parameter names for the current function
    param_names: HashSet<String>,
    /// Map from parameter name to mutation sites
    mutation_sites: HashMap<String, Vec<u32>>,
}

impl ParameterMutationDetector {
    fn new() -> Self {
        Self::default()
    }

    fn add_parameter(&mut self, name: &str) {
        self.param_names.insert(name.to_string());
    }

    fn record_mutation(&mut self, name: &str, line: u32) {
        if self.param_names.contains(name) {
            self.mutation_sites.entry(name.to_string()).or_default().push(line);
        }
    }

    fn to_parameter_mutabilities(&self) -> Vec<ParameterMutability> {
        let mut results: Vec<ParameterMutability> = self
            .param_names
            .iter()
            .map(|name| {
                let sites = self.mutation_sites.get(name).cloned().unwrap_or_default();
                let mutated = !sites.is_empty();

                ParameterMutability {
                    name: name.clone(),
                    mutated,
                    mutation_sites: sites,
                }
            })
            .collect();

        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }
}

/// Detects collection mutations (append, extend, update, etc.)
#[derive(Debug, Default)]
pub struct CollectionMutationDetector {
    mutations: Vec<CollectionMutation>,
}

impl CollectionMutationDetector {
    fn new() -> Self {
        Self::default()
    }

    fn record_mutation(&mut self, variable: String, operation: String, line: u32) {
        self.mutations.push(CollectionMutation {
            variable,
            operation,
            line,
        });
    }

    fn into_mutations(self) -> Vec<CollectionMutation> {
        self.mutations
    }
}

/// Tracks class field mutations
#[derive(Debug, Default)]
pub struct FieldTracker {
    /// Fields initialized in __init__
    pub init_fields: HashSet<String>,
    /// Fields modified in methods (method_name -> field_names)
    pub modified_fields: HashMap<String, HashSet<String>>,
}

impl FieldTracker {
    fn new() -> Self {
        Self::default()
    }

    fn record_init_field(&mut self, name: &str) {
        self.init_fields.insert(name.to_string());
    }

    fn record_field_modification(&mut self, method_name: &str, field_name: &str) {
        self.modified_fields
            .entry(method_name.to_string())
            .or_default()
            .insert(field_name.to_string());
    }

    fn to_field_mutabilities(&self) -> Vec<FieldMutability> {
        let mut all_fields: HashSet<&String> = self.init_fields.iter().collect();
        for fields in self.modified_fields.values() {
            all_fields.extend(fields.iter());
        }

        let mut results: Vec<FieldMutability> = all_fields
            .into_iter()
            .map(|name| {
                let init_only = self.init_fields.contains(name)
                    && !self
                        .modified_fields
                        .iter()
                        .filter(|(method, _)| *method != "__init__")
                        .any(|(_, fields)| fields.contains(name));

                let mutable = !init_only;

                FieldMutability {
                    name: name.clone(),
                    mutable,
                    init_only,
                }
            })
            .collect();

        results.sort_by(|a, b| a.name.cmp(&b.name));
        results
    }
}

// =============================================================================
// Tree-sitter Multi-Language Parsing
// =============================================================================

/// Initialize tree-sitter parser for Python (legacy, used by Python-only paths)
fn get_python_parser() -> PatternsResult<Parser> {
    get_parser_for_language(Language::Python)
}

/// Initialize tree-sitter parser for a given language
fn get_parser_for_language(lang: Language) -> PatternsResult<Parser> {
    let ts_lang = ParserPool::get_ts_language(lang)
        .ok_or_else(|| PatternsError::parse_error(PathBuf::new(), format!("Unsupported language: {}", lang)))?;
    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| PatternsError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e)))?;
    Ok(parser)
}

/// Get function/method node kinds for a language
fn function_kinds_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["function_definition", "async_function_definition"],
        Language::Go => &["function_declaration", "method_declaration"],
        Language::TypeScript | Language::JavaScript => &[
            "function_declaration", "method_definition", "arrow_function",
            "generator_function_declaration",
        ],
        Language::Rust => &["function_item"],
        Language::Java => &["method_declaration", "constructor_declaration"],
        Language::C | Language::Cpp => &["function_definition"],
        Language::Ruby => &["method", "singleton_method"],
        Language::Php => &["function_definition", "method_declaration"],
        Language::Kotlin => &["function_declaration"],
        Language::Swift => &["function_declaration", "init_declaration"],
        Language::CSharp => &["method_declaration", "constructor_declaration"],
        Language::Scala => &["function_definition", "val_definition"],
        Language::Elixir => &["call"],
        Language::Lua | Language::Luau => &["function_declaration", "local_function"],
        Language::Ocaml => &["let_binding", "value_definition"],
    }
}

/// Get class/struct node kinds for a language
fn class_kinds_for_language(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Python => &["class_definition"],
        Language::Go => &["type_declaration"],
        Language::TypeScript | Language::JavaScript => &["class_declaration"],
        Language::Java => &["class_declaration"],
        Language::Rust => &["struct_item", "impl_item"],
        Language::CSharp => &["class_declaration"],
        Language::Kotlin => &["class_declaration"],
        Language::Swift => &["class_declaration"],
        Language::Php => &["class_declaration"],
        _ => &[],
    }
}

/// Get text for a node from source
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Get the line number (1-indexed) for a node
fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Extract function name from a function definition node (multi-language)
fn get_function_name(node: Node, source: &[u8]) -> Option<String> {
    // Try field-based name extraction first (Go, Rust, Java, TS, etc.)
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, source).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // Fallback: first identifier child (Python, etc.)
    for child in node.children(&mut node.walk()) {
        if child.kind() == "identifier" {
            return Some(node_text(child, source).to_string());
        }
    }
    None
}

/// Extract class name from a class definition node (multi-language)
fn get_class_name(node: Node, source: &[u8]) -> Option<String> {
    // Try field-based name extraction first
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = node_text(name_node, source).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // Fallback: first identifier child
    for child in node.children(&mut node.walk()) {
        if child.kind() == "identifier" {
            return Some(node_text(child, source).to_string());
        }
    }
    None
}

/// Extract parameter names from a function definition (multi-language)
fn extract_parameters(func_node: Node, source: &[u8]) -> Vec<String> {
    let mut params = Vec::new();

    for child in func_node.children(&mut func_node.walk()) {
        match child.kind() {
            // Python: parameters
            "parameters" => {
                for param_child in child.children(&mut child.walk()) {
                    match param_child.kind() {
                        "identifier" => {
                            params.push(node_text(param_child, source).to_string());
                        }
                        "typed_parameter" | "typed_default_parameter" | "default_parameter" => {
                            // First identifier is the parameter name
                            for inner in param_child.children(&mut param_child.walk()) {
                                if inner.kind() == "identifier" {
                                    params.push(node_text(inner, source).to_string());
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Go: parameter_list with parameter_declaration children
            "parameter_list" => {
                for param_child in child.children(&mut child.walk()) {
                    if param_child.kind() == "parameter_declaration" {
                        // Go parameter_declaration: name(s) followed by type
                        // e.g., "a int" or "a, b int"
                        if let Some(name_node) = param_child.child_by_field_name("name") {
                            params.push(node_text(name_node, source).to_string());
                        } else {
                            // Multiple names: iterate identifiers before the type
                            for inner in param_child.children(&mut param_child.walk()) {
                                if inner.kind() == "identifier" {
                                    params.push(node_text(inner, source).to_string());
                                }
                            }
                        }
                    }
                }
            }
            // Java/TS/Rust: formal_parameters, formal_parameter
            "formal_parameters" => {
                for param_child in child.children(&mut child.walk()) {
                    if param_child.kind() == "formal_parameter" || param_child.kind() == "required_parameter" || param_child.kind() == "optional_parameter" {
                        if let Some(name_node) = param_child.child_by_field_name("name") {
                            params.push(node_text(name_node, source).to_string());
                        } else {
                            for inner in param_child.children(&mut param_child.walk()) {
                                if inner.kind() == "identifier" {
                                    params.push(node_text(inner, source).to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    params
}

// =============================================================================
// Variable Assignment Tracking (M1)
// =============================================================================

/// Track variable assignments in a function
pub fn track_variable_assignments(func_node: Node, source: &[u8]) -> VariableTracker {
    let mut tracker = VariableTracker::new();
    track_assignments_recursive(func_node, source, &mut tracker, 0);
    tracker
}

fn track_assignments_recursive(
    node: Node,
    source: &[u8],
    tracker: &mut VariableTracker,
    depth: usize,
) {
    if depth > 100 {
        return; // Prevent stack overflow
    }

    match node.kind() {
        "assignment" => {
            // Handle: x = value
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "identifier" {
                    let name = node_text(left, source);
                    tracker.record_assignment(name, get_line_number(node));
                } else if left.kind() == "pattern_list" || left.kind() == "tuple_pattern" {
                    // Handle: a, b = value
                    for child in left.children(&mut left.walk()) {
                        if child.kind() == "identifier" {
                            let name = node_text(child, source);
                            tracker.record_assignment(name, get_line_number(node));
                        }
                    }
                }
            }
        }
        "augmented_assignment" => {
            // Handle: x += 1, x -= 1, etc.
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "identifier" {
                    let name = node_text(left, source);
                    tracker.record_mutation(name, get_line_number(node));
                }
            }
        }
        "for_statement" => {
            // Handle: for x in items:
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "identifier" {
                    let name = node_text(left, source);
                    tracker.record_assignment(name, get_line_number(node));
                } else if left.kind() == "pattern_list" || left.kind() == "tuple_pattern" {
                    for child in left.children(&mut left.walk()) {
                        if child.kind() == "identifier" {
                            let name = node_text(child, source);
                            tracker.record_assignment(name, get_line_number(node));
                        }
                    }
                }
            }
        }
        "with_statement" => {
            // Handle: with open(f) as x:
            for child in node.children(&mut node.walk()) {
                if child.kind() == "with_clause" {
                    for item in child.children(&mut child.walk()) {
                        if item.kind() == "with_item" {
                            // Look for 'as' clause
                            let mut saw_as = false;
                            for inner in item.children(&mut item.walk()) {
                                if inner.kind() == "as" {
                                    saw_as = true;
                                }
                                if saw_as && inner.kind() == "identifier" {
                                    let name = node_text(inner, source);
                                    tracker.record_assignment(name, get_line_number(node));
                                }
                            }
                        }
                    }
                }
            }
        }
        "named_expression" => {
            // Handle: if (x := value):
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                tracker.record_assignment(name, get_line_number(node));
            }
        }
        _ => {}
    }

    // Recurse into children
    for child in node.children(&mut node.walk()) {
        track_assignments_recursive(child, source, tracker, depth + 1);
    }
}

// =============================================================================
// Parameter Mutation Detection (M2)
// =============================================================================

fn detect_parameter_mutations(
    func_node: Node,
    source: &[u8],
    detector: &mut ParameterMutationDetector,
) {
    detect_param_mutations_recursive(func_node, source, detector, 0);
}

fn detect_param_mutations_recursive(
    node: Node,
    source: &[u8],
    detector: &mut ParameterMutationDetector,
    depth: usize,
) {
    if depth > 100 {
        return;
    }

    match node.kind() {
        "call" => {
            // Check for mutating method calls like: param.append(x)
            if let Some(func) = node.child_by_field_name("function") {
                if func.kind() == "attribute" {
                    if let (Some(obj), Some(method)) = (
                        func.child_by_field_name("object"),
                        func.child_by_field_name("attribute"),
                    ) {
                        if obj.kind() == "identifier" {
                            let obj_name = node_text(obj, source);
                            let method_name = node_text(method, source);

                            if MUTATING_METHODS.contains(&method_name) {
                                detector.record_mutation(obj_name, get_line_number(node));
                            }
                        }
                    }
                }
            }
        }
        "augmented_assignment" => {
            // Check if parameter is target of augmented assignment
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "subscript" {
                    // param[key] += value
                    if let Some(value_node) = left.child_by_field_name("value") {
                        if value_node.kind() == "identifier" {
                            let name = node_text(value_node, source);
                            detector.record_mutation(name, get_line_number(node));
                        }
                    }
                }
            }
        }
        "assignment" => {
            // Check if parameter's item is being assigned: param[key] = value
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "subscript" {
                    if let Some(value_node) = left.child_by_field_name("value") {
                        if value_node.kind() == "identifier" {
                            let name = node_text(value_node, source);
                            detector.record_mutation(name, get_line_number(node));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        detect_param_mutations_recursive(child, source, detector, depth + 1);
    }
}

// =============================================================================
// Collection Mutation Detection (M4)
// =============================================================================

fn detect_collection_mutations(
    func_node: Node,
    source: &[u8],
    detector: &mut CollectionMutationDetector,
) {
    detect_collection_mutations_recursive(func_node, source, detector, 0);
}

fn detect_collection_mutations_recursive(
    node: Node,
    source: &[u8],
    detector: &mut CollectionMutationDetector,
    depth: usize,
) {
    if depth > 100 {
        return;
    }

    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            if func.kind() == "attribute" {
                if let (Some(obj), Some(method)) = (
                    func.child_by_field_name("object"),
                    func.child_by_field_name("attribute"),
                ) {
                    let method_name = node_text(method, source);

                    if MUTATING_METHODS.contains(&method_name) {
                        let variable = extract_name_from_expr(obj, source);
                        detector.record_mutation(variable, method_name.to_string(), get_line_number(node));
                    }
                }
            }
        }
    }

    for child in node.children(&mut node.walk()) {
        detect_collection_mutations_recursive(child, source, detector, depth + 1);
    }
}

/// Extract a name from an expression (handles chained attribute access)
fn extract_name_from_expr(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, source).to_string(),
        "attribute" => {
            let mut parts = Vec::new();
            let mut current = node;

            loop {
                if let Some(attr) = current.child_by_field_name("attribute") {
                    parts.push(node_text(attr, source).to_string());
                }

                if let Some(obj) = current.child_by_field_name("object") {
                    if obj.kind() == "attribute" {
                        current = obj;
                    } else if obj.kind() == "identifier" {
                        parts.push(node_text(obj, source).to_string());
                        break;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            parts.reverse();
            parts.join(".")
        }
        _ => node_text(node, source).to_string(),
    }
}

// =============================================================================
// Class Field Analysis (M3)
// =============================================================================

/// Analyze class fields for mutability
pub fn analyze_class_fields(class_node: Node, source: &[u8]) -> FieldTracker {
    let mut tracker = FieldTracker::new();

    // Find methods in the class
    for child in class_node.children(&mut class_node.walk()) {
        if child.kind() == "block" {
            for block_child in child.children(&mut child.walk()) {
                if block_child.kind() == "function_definition" {
                    let method_name = get_function_name(block_child, source)
                        .unwrap_or_else(|| "<unknown>".to_string());

                    analyze_method_field_access(block_child, source, &method_name, &mut tracker);
                }
            }
        }
    }

    tracker
}

fn analyze_method_field_access(
    method_node: Node,
    source: &[u8],
    method_name: &str,
    tracker: &mut FieldTracker,
) {
    analyze_field_access_recursive(method_node, source, method_name, tracker, 0);
}

fn analyze_field_access_recursive(
    node: Node,
    source: &[u8],
    method_name: &str,
    tracker: &mut FieldTracker,
    depth: usize,
) {
    if depth > 100 {
        return;
    }

    match node.kind() {
        "assignment" | "augmented_assignment" => {
            // Check for self.field = value
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "attribute" {
                    if let (Some(obj), Some(attr)) = (
                        left.child_by_field_name("object"),
                        left.child_by_field_name("attribute"),
                    ) {
                        if obj.kind() == "identifier" && node_text(obj, source) == "self" {
                            let field_name = node_text(attr, source);

                            if method_name == "__init__" {
                                tracker.record_init_field(field_name);
                            } else {
                                tracker.record_field_modification(method_name, field_name);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    for child in node.children(&mut node.walk()) {
        analyze_field_access_recursive(child, source, method_name, tracker, depth + 1);
    }
}

// =============================================================================
// Function Analysis
// =============================================================================

/// Analyze a single function for mutability
fn analyze_function_mutability(
    func_node: Node,
    source: &[u8],
    no_collections: bool,
) -> FunctionMutability {
    let name = get_function_name(func_node, source).unwrap_or_else(|| "<anonymous>".to_string());

    // Track variable assignments (M1)
    let var_tracker = track_variable_assignments(func_node, source);
    let variables = var_tracker.to_variable_mutabilities();

    // Track parameter mutations (M2)
    let params = extract_parameters(func_node, source);
    let mut param_detector = ParameterMutationDetector::new();
    for param in &params {
        param_detector.add_parameter(param);
    }
    detect_parameter_mutations(func_node, source, &mut param_detector);
    let parameters = param_detector.to_parameter_mutabilities();

    // Track collection mutations (M4)
    let collection_mutations = if no_collections {
        Vec::new()
    } else {
        let mut collection_detector = CollectionMutationDetector::new();
        detect_collection_mutations(func_node, source, &mut collection_detector);
        collection_detector.into_mutations()
    };

    FunctionMutability {
        name,
        variables,
        parameters,
        collection_mutations,
    }
}

/// Analyze a class for mutability (when --include-fields is set)
fn analyze_class_mutability(class_node: Node, source: &[u8]) -> ClassMutability {
    let name = get_class_name(class_node, source).unwrap_or_else(|| "<anonymous>".to_string());
    let field_tracker = analyze_class_fields(class_node, source);
    let fields = field_tracker.to_field_mutabilities();

    ClassMutability { name, fields }
}

// =============================================================================
// Summary Computation
// =============================================================================

fn compute_summary(
    functions: &[FunctionMutability],
    classes: &[ClassMutability],
) -> MutabilitySummary {
    let functions_analyzed = functions.len() as u32;
    let classes_analyzed = classes.len() as u32;

    let mut total_variables = 0u32;
    let mut mutable_variables = 0u32;
    let mut total_parameters = 0u32;
    let mut mutated_parameters = 0u32;

    for func in functions {
        total_variables += func.variables.len() as u32;
        mutable_variables += func.variables.iter().filter(|v| v.mutable).count() as u32;

        total_parameters += func.parameters.len() as u32;
        mutated_parameters += func.parameters.iter().filter(|p| p.mutated).count() as u32;
    }

    let immutable_variables = total_variables.saturating_sub(mutable_variables);

    let immutable_pct = if total_variables > 0 {
        (immutable_variables as f64 / total_variables as f64) * 100.0
    } else {
        0.0
    };

    let unmutated_pct = if total_parameters > 0 {
        let unmutated = total_parameters.saturating_sub(mutated_parameters);
        (unmutated as f64 / total_parameters as f64) * 100.0
    } else {
        0.0
    };

    let mut fields_analyzed = 0u32;
    let mut mutable_fields = 0u32;

    for class in classes {
        fields_analyzed += class.fields.len() as u32;
        mutable_fields += class.fields.iter().filter(|f| f.mutable).count() as u32;
    }

    MutabilitySummary {
        functions_analyzed,
        classes_analyzed,
        total_variables,
        mutable_variables,
        immutable_variables,
        immutable_pct,
        parameters_analyzed: total_parameters,
        mutated_parameters,
        unmutated_pct,
        fields_analyzed,
        mutable_fields,
        constraints_generated: 0, // Not tracking constraints in current impl
    }
}

// =============================================================================
// File Analysis
// =============================================================================

/// Analyze mutability for a single file
pub fn analyze_mutability_file(
    path: &std::path::Path,
    args: &MutabilityArgs,
) -> PatternsResult<MutabilityReport> {
    let start_time = Instant::now();

    // Validate path
    let canonical = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(path, root)?
    } else {
        validate_file_path(path)?
    };

    // Detect language from file extension (default to Python for backward compat)
    let lang = Language::from_path(&canonical).unwrap_or(Language::Python);

    // Read source
    let source = read_file_safe(&canonical)?;
    let source_bytes = source.as_bytes();

    // Parse with tree-sitter (multi-language)
    let mut parser = get_parser_for_language(lang)?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| PatternsError::parse_error(&canonical, "Failed to parse file"))?;

    let root = tree.root_node();
    let func_kinds = function_kinds_for_language(lang);
    let class_kinds = class_kinds_for_language(lang);

    // Collect all function and class names
    let mut function_nodes: Vec<Node> = Vec::new();
    let mut class_nodes: Vec<Node> = Vec::new();

    collect_definitions(root, &mut function_nodes, &mut class_nodes, func_kinds, class_kinds);

    // If function specified, filter to only that function
    if let Some(ref target_func) = args.function_name {
        let mut found = false;
        function_nodes.retain(|node| {
            if let Some(name) = get_function_name(*node, source_bytes) {
                if &name == target_func {
                    found = true;
                    return true;
                }
            }
            false
        });

        if !found {
            return Err(PatternsError::function_not_found(target_func, &canonical));
        }
    }

    // Analyze functions
    let functions: Vec<FunctionMutability> = function_nodes
        .iter()
        .map(|node| analyze_function_mutability(*node, source_bytes, args.no_collections))
        .collect();

    // Analyze classes (if --include-fields)
    let classes: Vec<ClassMutability> = if args.include_fields {
        class_nodes
            .iter()
            .map(|node| analyze_class_mutability(*node, source_bytes))
            .collect()
    } else {
        Vec::new()
    };

    // Compute summary
    let summary = compute_summary(&functions, &classes);

    let elapsed = start_time.elapsed();

    // Detect language from file path
    let language = Language::from_path(&canonical)
        .unwrap_or(Language::Python);
    let language_str = serde_json::to_value(&language)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "python".to_string());

    Ok(MutabilityReport {
        file: canonical.to_string_lossy().to_string(),
        language: language_str,
        functions,
        classes,
        summary,
        analysis_time_ms: elapsed.as_millis() as u64,
    })
}

/// Collect function and class definition nodes from the AST
fn collect_definitions<'a>(
    node: Node<'a>,
    functions: &mut Vec<Node<'a>>,
    classes: &mut Vec<Node<'a>>,
    func_kinds: &[&str],
    class_kinds: &[&str],
) {
    let kind = node.kind();

    if func_kinds.contains(&kind) {
        functions.push(node);
    } else if class_kinds.contains(&kind) {
        classes.push(node);
        // Also collect methods inside the class body
        for child in node.children(&mut node.walk()) {
            // Python uses "block", other languages use "class_body", "declaration_list", etc.
            let child_kind = child.kind();
            if child_kind == "block" || child_kind == "class_body" || child_kind == "declaration_list" || child_kind == "body" {
                for block_child in child.children(&mut child.walk()) {
                    if func_kinds.contains(&block_child.kind()) {
                        functions.push(block_child);
                    }
                }
            }
        }
    }

    // Recurse into children (but not into class blocks, handled above)
    if !class_kinds.contains(&kind) {
        for child in node.children(&mut node.walk()) {
            collect_definitions(child, functions, classes, func_kinds, class_kinds);
        }
    }
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format a mutability report as human-readable text
pub fn format_mutability_text(report: &MutabilityReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!("File: {}", report.file));
    lines.push(format!("Language: {}", report.language));
    lines.push(String::new());

    for func in &report.functions {
        lines.push(format!("Function: {}", func.name));

        if !func.variables.is_empty() {
            lines.push("  Variables:".to_string());
            for var in &func.variables {
                let status = if var.mutable { "mutable" } else { "immutable" };
                lines.push(format!(
                    "    {} ({}) - {} reassignments, {} mutations",
                    var.name, status, var.reassignments, var.mutations
                ));
            }
        }

        if !func.parameters.is_empty() {
            lines.push("  Parameters:".to_string());
            for param in &func.parameters {
                let status = if param.mutated { "mutated" } else { "unmutated" };
                if param.mutated {
                    lines.push(format!(
                        "    {} ({}) at lines {:?}",
                        param.name, status, param.mutation_sites
                    ));
                } else {
                    lines.push(format!("    {} ({})", param.name, status));
                }
            }
        }

        if !func.collection_mutations.is_empty() {
            lines.push("  Collection Mutations:".to_string());
            for cm in &func.collection_mutations {
                lines.push(format!("    {}.{}() at line {}", cm.variable, cm.operation, cm.line));
            }
        }

        lines.push(String::new());
    }

    if !report.classes.is_empty() {
        lines.push("Classes:".to_string());
        for class in &report.classes {
            lines.push(format!("  Class: {}", class.name));
            for field in &class.fields {
                let status = if field.mutable {
                    "mutable"
                } else if field.init_only {
                    "init-only"
                } else {
                    "immutable"
                };
                lines.push(format!("    {} ({})", field.name, status));
            }
        }
        lines.push(String::new());
    }

    lines.push("Summary:".to_string());
    lines.push(format!("  Functions analyzed: {}", report.summary.functions_analyzed));
    lines.push(format!("  Classes analyzed: {}", report.summary.classes_analyzed));
    lines.push(format!(
        "  Variables: {}/{} immutable ({:.1}%)",
        report.summary.immutable_variables,
        report.summary.total_variables,
        report.summary.immutable_pct
    ));
    lines.push(format!(
        "  Parameters: {}/{} unmutated ({:.1}%)",
        report.summary.parameters_analyzed - report.summary.mutated_parameters,
        report.summary.parameters_analyzed,
        report.summary.unmutated_pct
    ));

    if report.summary.fields_analyzed > 0 {
        lines.push(format!(
            "  Fields: {}/{} mutable",
            report.summary.mutable_fields, report.summary.fields_analyzed
        ));
    }

    lines.push(format!("  Analysis time: {}ms", report.analysis_time_ms));

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

/// Execute the mutability command
pub fn run(args: &MutabilityArgs) -> anyhow::Result<()> {
    // Only Python is supported currently
    let report = analyze_mutability_file(&args.file, args)?;

    match args.output_format {
        OutputFormat::Json => {
            if args.summary {
                // Summary-only mode
                let summary_output = serde_json::json!({
                    "file": report.file,
                    "language": report.language,
                    "summary": report.summary,
                    "analysis_time_ms": report.analysis_time_ms,
                });
                let json = serde_json::to_string_pretty(&summary_output)?;
                println!("{}", json);
            } else {
                let json = serde_json::to_string_pretty(&report)?;
                println!("{}", json);
            }
        }
        OutputFormat::Text => {
            println!("{}", format_mutability_text(&report));
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_mutating_methods_constant() {
        assert!(MUTATING_METHODS.contains(&"append"));
        assert!(MUTATING_METHODS.contains(&"extend"));
        assert!(MUTATING_METHODS.contains(&"update"));
        assert!(MUTATING_METHODS.contains(&"pop"));
        assert!(MUTATING_METHODS.contains(&"clear"));
    }

    #[test]
    fn test_variable_tracker_basic() {
        let mut tracker = VariableTracker::new();
        tracker.record_assignment("x", 1);
        tracker.record_assignment("x", 5);
        tracker.record_mutation("y", 10);

        let vars = tracker.to_variable_mutabilities();

        let x = vars.iter().find(|v| v.name == "x").unwrap();
        assert!(x.mutable);
        assert_eq!(x.reassignments, 1); // 2 assignments - 1 = 1 reassignment
        assert_eq!(x.mutations, 0);

        let y = vars.iter().find(|v| v.name == "y").unwrap();
        assert!(y.mutable);
        assert_eq!(y.mutations, 1);
    }

    #[test]
    fn test_parameter_mutation_detector() {
        let mut detector = ParameterMutationDetector::new();
        detector.add_parameter("items");
        detector.add_parameter("config");
        detector.record_mutation("items", 5);
        detector.record_mutation("items", 10);

        let params = detector.to_parameter_mutabilities();

        let items = params.iter().find(|p| p.name == "items").unwrap();
        assert!(items.mutated);
        assert_eq!(items.mutation_sites, vec![5, 10]);

        let config = params.iter().find(|p| p.name == "config").unwrap();
        assert!(!config.mutated);
    }

    #[test]
    fn test_analyze_immutable_function() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("immut.py");
        std::fs::write(
            &file_path,
            r#"
def immutable_vars(x, y):
    a = x + 1
    b = y + 2
    result = a + b
    return result
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("immutable_vars".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        assert_eq!(report.functions.len(), 1);
        let func = &report.functions[0];
        assert_eq!(func.name, "immutable_vars");

        // Variables a, b, result should be immutable (single assignment)
        for var in &func.variables {
            if var.name != "x" && var.name != "y" {
                assert!(!var.mutable, "Variable {} should be immutable", var.name);
            }
        }
    }

    #[test]
    fn test_analyze_mutable_function() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("mut.py");
        std::fs::write(
            &file_path,
            r#"
def mutable_vars(x):
    count = 0
    count = count + 1
    count += 1
    return count
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("mutable_vars".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        let func = &report.functions[0];
        let count_var = func.variables.iter().find(|v| v.name == "count").unwrap();
        assert!(count_var.mutable, "count should be mutable");
        assert!(count_var.reassignments >= 1, "count should have reassignments");
    }

    #[test]
    fn test_analyze_parameter_mutation() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("param_mut.py");
        std::fs::write(
            &file_path,
            r#"
def mutate_parameter(items):
    items.append("new")
    items.extend([1, 2, 3])
    return items
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("mutate_parameter".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        let func = &report.functions[0];
        let items_param = func.parameters.iter().find(|p| p.name == "items").unwrap();
        assert!(items_param.mutated, "items parameter should be marked as mutated");
    }

    #[test]
    fn test_collection_mutations() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("coll.py");
        std::fs::write(
            &file_path,
            r#"
def collection_mutations():
    data = []
    data.append(1)
    data.extend([2, 3])
    mapping = {}
    mapping.update({'key': 'value'})
    return data, mapping
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("collection_mutations".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        let func = &report.functions[0];
        assert!(
            !func.collection_mutations.is_empty(),
            "Should detect collection mutations"
        );

        let ops: Vec<&str> = func.collection_mutations.iter().map(|cm| cm.operation.as_str()).collect();
        assert!(ops.contains(&"append"));
        assert!(ops.contains(&"extend"));
        assert!(ops.contains(&"update"));
    }

    #[test]
    fn test_no_collections_flag() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("coll.py");
        std::fs::write(
            &file_path,
            r#"
def collection_mutations():
    data = []
    data.append(1)
    return data
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("collection_mutations".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: true, // Skip collection tracking
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        let func = &report.functions[0];
        assert!(
            func.collection_mutations.is_empty(),
            "With --no-collections, should not track collection mutations"
        );
    }

    #[test]
    fn test_summary_percentages() {
        let functions = vec![
            FunctionMutability {
                name: "f1".to_string(),
                variables: vec![
                    VariableMutability {
                        name: "a".to_string(),
                        mutable: false,
                        reassignments: 0,
                        mutations: 0,
                    },
                    VariableMutability {
                        name: "b".to_string(),
                        mutable: true,
                        reassignments: 1,
                        mutations: 0,
                    },
                ],
                parameters: vec![
                    ParameterMutability {
                        name: "x".to_string(),
                        mutated: false,
                        mutation_sites: vec![],
                    },
                    ParameterMutability {
                        name: "y".to_string(),
                        mutated: true,
                        mutation_sites: vec![5],
                    },
                ],
                collection_mutations: vec![],
            },
        ];

        let summary = compute_summary(&functions, &[]);

        assert_eq!(summary.total_variables, 2);
        assert_eq!(summary.mutable_variables, 1);
        assert_eq!(summary.immutable_variables, 1);
        assert!((summary.immutable_pct - 50.0).abs() < 0.1);

        assert_eq!(summary.parameters_analyzed, 2);
        assert_eq!(summary.mutated_parameters, 1);
        assert!((summary.unmutated_pct - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_function_not_found_error() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("test.py");
        std::fs::write(&file_path, "def existing(): pass").unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("nonexistent".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let result = analyze_mutability_file(&file_path, &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_analyze_go_function_mutability() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("counter.go");
        std::fs::write(
            &file_path,
            r#"package main

func Add(a int, b int) int {
	return a + b
}
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: None,
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        // Should find Go function
        assert!(
            !report.functions.is_empty(),
            "Should find at least one Go function"
        );
        let add_fn = report.functions.iter().find(|f| f.name == "Add");
        assert!(add_fn.is_some(), "Should find Go function 'Add'");
    }

    #[test]
    fn test_analyze_go_specific_function() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("math.go");
        std::fs::write(
            &file_path,
            r#"package main

func Multiply(x int, y int) int {
	result := x * y
	return result
}

func Divide(x int, y int) int {
	return x / y
}
"#,
        )
        .unwrap();

        let args = MutabilityArgs {
            file: file_path.clone(),
            function_name: Some("Multiply".to_string()),
            include_fields: false,
            include_aliases: false,
            no_collections: false,
            constraints: false,
            summary: false,
            output_format: OutputFormat::Json,
            project_root: None,
        };

        let report = analyze_mutability_file(&file_path, &args).unwrap();

        assert_eq!(report.functions.len(), 1, "Should find only 'Multiply'");
        assert_eq!(report.functions[0].name, "Multiply");
    }
}
