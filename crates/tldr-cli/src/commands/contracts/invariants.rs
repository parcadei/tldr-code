//! Invariants command - Daikon-lite invariant inference from test traces.
//!
//! Infers likely invariants from analyzing test files for function call patterns.
//! This is a simplified static analysis approach that extracts invariants from
//! observed argument patterns in test assertions.
//!
//! # TIGER/ELEPHANT Mitigations Addressed
//! - TIGER-06: Test path sanitization -> validate_file_path checks
//! - E08: Parse test files before analysis -> tree-sitter validation
//! - E12: Consistent ordering -> sort test functions alphabetically
//!
//! # Invariant Types
//!
//! | Kind | Detection Rule | Example |
//! |------|---------------|---------|
//! | Type | All values same type | `x: int` |
//! | NonNull | No None values observed | `x is not None` |
//! | NonNegative | All numeric values >= 0 | `x >= 0` |
//! | Positive | All numeric values > 0 | `x > 0` |
//! | Range | Track min/max observed | `0 <= x <= 100` |
//! | Relation | p1 < p2 for all observations | `start < end` |
//!
//! # Simplified Implementation Note
//!
//! This implementation uses static analysis of test files to infer invariants,
//! rather than actual runtime tracing. It parses function calls from test
//! assertions and infers invariants from the argument patterns observed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use tldr_core::walker::walk_project;
use tree_sitter::{Node, Parser};
use tree_sitter_python::LANGUAGE as PYTHON_LANGUAGE;

use crate::output::{OutputFormat, OutputWriter};

use super::error::{ContractsError, ContractsResult};
use super::types::{
    Confidence, FunctionInvariants, Invariant, InvariantKind, InvariantsReport, InvariantsSummary,
    OutputFormat as ContractsOutputFormat,
};
use super::validation::read_file_safe;

// =============================================================================
// Resource Limits
// =============================================================================

/// Maximum depth for AST traversal (TIGER-08 mitigation)
const MAX_AST_DEPTH: usize = 100;

// =============================================================================
// CLI Arguments
// =============================================================================

/// Infer invariants from test execution traces (Daikon-lite).
///
/// Analyzes test files to extract function call patterns and infers
/// likely invariants such as type constraints, numeric bounds, and
/// ordering relations between parameters.
///
/// # Example
///
/// ```bash
/// tldr invariants src/module.py --from-tests tests/
/// tldr invariants src/math.py --from-tests tests/test_math.py --min-obs 5
/// tldr invariants src/api.py --from-tests tests/ --function process_data
/// ```
#[derive(Debug, Args)]
pub struct InvariantsArgs {
    /// Source file containing functions to analyze
    pub file: PathBuf,

    /// Test file or directory for tracing
    #[arg(long = "from-tests", short = 't')]
    pub from_tests: PathBuf,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output-format",
        short = 'o',
        hide = true,
        default_value = "json"
    )]
    pub output_format: ContractsOutputFormat,

    /// Filter to specific function
    #[arg(long)]
    pub function: Option<String>,

    /// Minimum observations required to report an invariant
    #[arg(long, default_value = "1")]
    pub min_obs: u32,

    /// Language override (auto-detected if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<String>,
}

impl InvariantsArgs {
    /// Run the invariants command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate source file exists
        if !self.file.exists() {
            return Err(ContractsError::FileNotFound {
                path: self.file.clone(),
            }
            .into());
        }

        // Validate test path exists
        if !self.from_tests.exists() {
            return Err(ContractsError::TestPathNotFound {
                path: self.from_tests.clone(),
            }
            .into());
        }

        writer.progress(&format!(
            "Inferring invariants for {} from {}...",
            self.file.display(),
            self.from_tests.display()
        ));

        // Run inference
        let report = run_invariants(
            &self.file,
            &self.from_tests,
            self.function.as_deref(),
            self.min_obs,
        )?;

        // Output based on format
        let use_text = matches!(self.output_format, ContractsOutputFormat::Text)
            || matches!(format, OutputFormat::Text);

        if use_text {
            let text = format_invariants_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Observation of a function call from a test
#[derive(Debug, Clone)]
struct Observation {
    /// Function name being called
    function_name: String,
    /// Argument values as JSON
    args: Vec<ObservedValue>,
    /// Expected return value (if available from assertion)
    return_value: Option<ObservedValue>,
}

/// An observed value from a test
#[derive(Debug, Clone)]
enum ObservedValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    None,
    List(Vec<ObservedValue>),
    Other(String), // Unparseable value represented as string
}

impl ObservedValue {
    fn type_name(&self) -> &'static str {
        match self {
            ObservedValue::Int(_) => "int",
            ObservedValue::Float(_) => "float",
            ObservedValue::String(s) => {
                let _ = s.len();
                "str"
            }
            ObservedValue::Bool(b) => {
                let _ = *b;
                "bool"
            }
            ObservedValue::None => "NoneType",
            ObservedValue::List(items) => {
                let _ = items.len();
                "list"
            }
            ObservedValue::Other(text) => {
                let _ = text.len();
                "unknown"
            }
        }
    }

    fn is_none(&self) -> bool {
        matches!(self, ObservedValue::None)
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            ObservedValue::Int(i) => Some(*i as f64),
            ObservedValue::Float(f) => Some(*f),
            _ => None,
        }
    }
}

/// Run invariant inference on source file using test observations.
///
/// Note: `_source_path` is currently unused in this simplified static analysis
/// implementation. It is kept in the API for future runtime tracing support.
pub fn run_invariants(
    _source_path: &Path,
    test_path: &Path,
    function_filter: Option<&str>,
    min_obs: u32,
) -> ContractsResult<InvariantsReport> {
    // Collect observations from test files
    let observations = collect_observations(test_path, function_filter)?;

    // Group observations by function
    let mut by_function: HashMap<String, Vec<Observation>> = HashMap::new();
    for obs in observations {
        by_function
            .entry(obs.function_name.clone())
            .or_default()
            .push(obs);
    }

    // Infer invariants for each function
    let mut functions = Vec::new();
    let mut total_observations = 0u32;
    let mut total_invariants = 0u32;
    let mut by_kind: HashMap<String, u32> = HashMap::new();

    for (func_name, obs_list) in by_function.iter() {
        let obs_count = obs_list.len() as u32;
        total_observations += obs_count;

        if obs_count < min_obs {
            continue;
        }

        let (preconditions, postconditions) = infer_invariants_for_function(obs_list);

        // Filter by min_obs
        let preconditions: Vec<_> = preconditions
            .into_iter()
            .filter(|inv| inv.observations >= min_obs)
            .collect();
        let postconditions: Vec<_> = postconditions
            .into_iter()
            .filter(|inv| inv.observations >= min_obs)
            .collect();

        // Count by kind
        for inv in preconditions.iter().chain(postconditions.iter()) {
            let kind_str = inv.kind.to_string();
            *by_kind.entry(kind_str).or_default() += 1;
            total_invariants += 1;
        }

        functions.push(FunctionInvariants {
            function_name: func_name.clone(),
            preconditions,
            postconditions,
            observation_count: obs_count,
        });
    }

    // Sort functions alphabetically for consistent output (E12)
    functions.sort_by(|a, b| a.function_name.cmp(&b.function_name));

    Ok(InvariantsReport {
        functions,
        summary: InvariantsSummary {
            total_observations,
            total_invariants,
            by_kind,
        },
    })
}

/// Collect observations from test files.
fn collect_observations(
    test_path: &Path,
    function_filter: Option<&str>,
) -> ContractsResult<Vec<Observation>> {
    let mut observations = Vec::new();

    if test_path.is_file() {
        let file_obs = extract_observations_from_file(test_path, function_filter)?;
        observations.extend(file_obs);
    } else {
        // Directory: scan all test_*.py files
        for entry in walk_project(test_path)
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        {
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if file_name.starts_with("test_") && file_name.ends_with(".py") {
                match extract_observations_from_file(path, function_filter) {
                    Ok(file_obs) => observations.extend(file_obs),
                    Err(_) => continue, // Skip files that fail to parse
                }
            }
        }
    }

    Ok(observations)
}

/// Extract observations from a single test file.
fn extract_observations_from_file(
    path: &Path,
    function_filter: Option<&str>,
) -> ContractsResult<Vec<Observation>> {
    let source = read_file_safe(path)?;

    let mut parser = Parser::new();
    parser
        .set_language(&PYTHON_LANGUAGE.into())
        .map_err(|e| ContractsError::ParseError {
            file: path.to_path_buf(),
            message: e.to_string(),
        })?;

    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| ContractsError::ParseError {
            file: path.to_path_buf(),
            message: "Failed to parse file".to_string(),
        })?;

    let root = tree.root_node();
    // Note: AST depth checking is done during recursive traversal via MAX_AST_DEPTH guard

    let mut observations = Vec::new();
    let mut current_test_function = String::new();

    // Walk the AST looking for test functions and assertions
    extract_observations_recursive(
        &root,
        &source,
        &mut observations,
        &mut current_test_function,
        function_filter,
        0,
    );

    Ok(observations)
}

/// Recursively extract observations from AST nodes.
fn extract_observations_recursive(
    node: &Node,
    source: &str,
    observations: &mut Vec<Observation>,
    current_test_function: &mut String,
    function_filter: Option<&str>,
    depth: usize,
) {
    if depth > MAX_AST_DEPTH {
        return;
    }

    match node.kind() {
        "function_definition" => {
            // Check if this is a test function
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source);
                if name.starts_with("test_") {
                    *current_test_function = name;
                }
            }
        }
        "assert_statement" => {
            // Extract observations from assert statements
            if !current_test_function.is_empty() {
                if let Some(obs) = extract_observation_from_assert(node, source, function_filter) {
                    observations.push(obs);
                }
            }
        }
        "call" => {
            // Also look at standalone calls in test functions
            if !current_test_function.is_empty() {
                if let Some(obs) = extract_observation_from_call(node, source, function_filter) {
                    observations.push(obs);
                }
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_observations_recursive(
            &child,
            source,
            observations,
            current_test_function,
            function_filter,
            depth + 1,
        );
    }
}

/// Extract an observation from an assert statement.
fn extract_observation_from_assert(
    node: &Node,
    source: &str,
    function_filter: Option<&str>,
) -> Option<Observation> {
    // Look for patterns like: assert func(args) == expected
    // or: assert func(args)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "comparison_operator" {
            // assert func(args) == expected
            if let Some(call_node) = find_call_in_subtree(&child) {
                return extract_observation_from_call_with_expected(
                    &call_node,
                    &child,
                    source,
                    function_filter,
                );
            }
        } else if child.kind() == "call" {
            // assert func(args)
            return extract_observation_from_call(&child, source, function_filter);
        }
    }
    None
}

/// Find a call node in a subtree.
fn find_call_in_subtree<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "call" {
        return Some(*node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(call) = find_call_in_subtree(&child) {
            return Some(call);
        }
    }
    None
}

/// Extract observation from a call node with expected value.
fn extract_observation_from_call_with_expected(
    call_node: &Node,
    comparison_node: &Node,
    source: &str,
    function_filter: Option<&str>,
) -> Option<Observation> {
    let function_name = extract_function_name(call_node, source)?;

    // Apply function filter
    if let Some(filter) = function_filter {
        if function_name != filter {
            return None;
        }
    }

    let args = extract_call_arguments(call_node, source);
    let return_value = extract_expected_value(comparison_node, call_node, source);

    Some(Observation {
        function_name,
        args,
        return_value,
    })
}

/// Extract observation from a call node.
fn extract_observation_from_call(
    call_node: &Node,
    source: &str,
    function_filter: Option<&str>,
) -> Option<Observation> {
    let function_name = extract_function_name(call_node, source)?;

    // Apply function filter
    if let Some(filter) = function_filter {
        if function_name != filter {
            return None;
        }
    }

    let args = extract_call_arguments(call_node, source);

    Some(Observation {
        function_name,
        args,
        return_value: None,
    })
}

/// Extract function name from a call node.
fn extract_function_name(call_node: &Node, source: &str) -> Option<String> {
    let func_node = call_node.child_by_field_name("function")?;

    match func_node.kind() {
        "identifier" => Some(node_text(func_node, source)),
        "attribute" => {
            // For method calls like obj.method(), extract just the method name
            func_node
                .child_by_field_name("attribute")
                .map(|n| node_text(n, source))
        }
        _ => None,
    }
}

/// Extract arguments from a call node.
fn extract_call_arguments(call_node: &Node, source: &str) -> Vec<ObservedValue> {
    let mut args = Vec::new();

    if let Some(args_node) = call_node.child_by_field_name("arguments") {
        let mut cursor = args_node.walk();
        for child in args_node.children(&mut cursor) {
            if child.kind() != "(" && child.kind() != ")" && child.kind() != "," {
                // Skip keyword arguments for now
                if child.kind() != "keyword_argument" {
                    args.push(parse_value(&child, source));
                }
            }
        }
    }

    args
}

/// Extract expected value from a comparison expression.
fn extract_expected_value(
    comparison_node: &Node,
    call_node: &Node,
    source: &str,
) -> Option<ObservedValue> {
    // Find the value that's being compared to (not the call itself)
    let mut cursor = comparison_node.walk();
    for child in comparison_node.children(&mut cursor) {
        // Skip the call node and operators
        if child.id() != call_node.id()
            && child.kind() != "=="
            && child.kind() != "!="
            && child.kind() != "comparison_operator"
        {
            return Some(parse_value(&child, source));
        }
    }
    None
}

/// Parse a value from an AST node.
fn parse_value(node: &Node, source: &str) -> ObservedValue {
    let text = node_text(*node, source);

    match node.kind() {
        "integer" => text
            .parse::<i64>()
            .map(ObservedValue::Int)
            .unwrap_or(ObservedValue::Other(text)),
        "float" => text
            .parse::<f64>()
            .map(ObservedValue::Float)
            .unwrap_or(ObservedValue::Other(text)),
        "string" | "concatenated_string" => {
            // Remove quotes
            let trimmed = text
                .trim_start_matches(['"', '\''])
                .trim_end_matches(['"', '\'']);
            ObservedValue::String(trimmed.to_string())
        }
        "true" => ObservedValue::Bool(true),
        "false" => ObservedValue::Bool(false),
        "none" => ObservedValue::None,
        "list" => {
            let mut items = Vec::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "[" && child.kind() != "]" && child.kind() != "," {
                    items.push(parse_value(&child, source));
                }
            }
            ObservedValue::List(items)
        }
        "unary_operator" => {
            // Handle negative numbers like -5
            if text.starts_with('-') {
                if let Ok(i) = text.parse::<i64>() {
                    return ObservedValue::Int(i);
                }
                if let Ok(f) = text.parse::<f64>() {
                    return ObservedValue::Float(f);
                }
            }
            ObservedValue::Other(text)
        }
        _ => ObservedValue::Other(text),
    }
}

/// Get text content of an AST node.
fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

// =============================================================================
// Invariant Inference
// =============================================================================

/// Infer invariants from a list of observations for a function.
fn infer_invariants_for_function(observations: &[Observation]) -> (Vec<Invariant>, Vec<Invariant>) {
    let n = observations.len() as u32;
    if n == 0 {
        return (Vec::new(), Vec::new());
    }

    let confidence = confidence_from_observations(n);
    let mut preconditions = Vec::new();
    let mut postconditions = Vec::new();

    // Collect all argument positions
    let max_args = observations.iter().map(|o| o.args.len()).max().unwrap_or(0);

    // Infer invariants for each argument position
    for arg_idx in 0..max_args {
        let values: Vec<_> = observations
            .iter()
            .filter_map(|o| o.args.get(arg_idx))
            .collect();

        if values.is_empty() {
            continue;
        }

        let param_name = format!("arg{}", arg_idx);

        // Type invariant
        if let Some(inv) = infer_type_invariant(&param_name, &values, n, confidence) {
            preconditions.push(inv);
        }

        // Non-null invariant
        if let Some(inv) = infer_non_null_invariant(&param_name, &values, n, confidence) {
            preconditions.push(inv);
        }

        // Numeric invariants
        let numeric_values: Vec<f64> = values.iter().filter_map(|v| v.as_f64()).collect();
        if !numeric_values.is_empty() && numeric_values.len() == values.len() {
            // Non-negative
            if let Some(inv) =
                infer_non_negative_invariant(&param_name, &numeric_values, n, confidence)
            {
                preconditions.push(inv);
            }

            // Positive
            if let Some(inv) = infer_positive_invariant(&param_name, &numeric_values, n, confidence)
            {
                preconditions.push(inv);
            }

            // Range
            if let Some(inv) = infer_range_invariant(&param_name, &numeric_values, n, confidence) {
                preconditions.push(inv);
            }
        }
    }

    // Infer ordering relations between arguments
    for i in 0..max_args {
        for j in (i + 1)..max_args {
            if let Some(inv) = infer_relation_invariant(observations, i, j, n, confidence) {
                preconditions.push(inv);
            }
        }
    }

    // Infer postconditions from return values
    let return_values: Vec<_> = observations
        .iter()
        .filter_map(|o| o.return_value.as_ref())
        .collect();

    if !return_values.is_empty() {
        // Type invariant for result
        if let Some(inv) = infer_type_invariant("result", &return_values, n, confidence) {
            postconditions.push(inv);
        }

        // Non-null invariant for result
        if let Some(inv) = infer_non_null_invariant("result", &return_values, n, confidence) {
            postconditions.push(inv);
        }

        // Numeric invariants for result
        let numeric_results: Vec<f64> = return_values.iter().filter_map(|v| v.as_f64()).collect();
        if !numeric_results.is_empty() && numeric_results.len() == return_values.len() {
            if let Some(inv) =
                infer_non_negative_invariant("result", &numeric_results, n, confidence)
            {
                postconditions.push(inv);
            }
            if let Some(inv) = infer_positive_invariant("result", &numeric_results, n, confidence) {
                postconditions.push(inv);
            }
            if let Some(inv) = infer_range_invariant("result", &numeric_results, n, confidence) {
                postconditions.push(inv);
            }
        }
    }

    (preconditions, postconditions)
}

/// Determine confidence level based on observation count.
fn confidence_from_observations(n: u32) -> Confidence {
    if n >= 10 {
        Confidence::High
    } else if n >= 5 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

/// Infer type invariant if all values have the same type.
fn infer_type_invariant(
    variable: &str,
    values: &[&ObservedValue],
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    if values.is_empty() {
        return None;
    }

    let first_type = values[0].type_name();
    if values.iter().all(|v| v.type_name() == first_type) && first_type != "unknown" {
        Some(Invariant {
            variable: variable.to_string(),
            kind: InvariantKind::Type,
            expression: format!("{}: {}", variable, first_type),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        })
    } else {
        None
    }
}

/// Infer non-null invariant if no values are None.
fn infer_non_null_invariant(
    variable: &str,
    values: &[&ObservedValue],
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    if values.is_empty() {
        return None;
    }

    if values.iter().all(|v| !v.is_none()) {
        Some(Invariant {
            variable: variable.to_string(),
            kind: InvariantKind::NonNull,
            expression: format!("{} is not None", variable),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        })
    } else {
        None
    }
}

/// Infer non-negative invariant if all numeric values >= 0.
fn infer_non_negative_invariant(
    variable: &str,
    values: &[f64],
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    if values.is_empty() {
        return None;
    }

    // Don't emit non_negative if all values are positive (positive is stronger)
    if values.iter().all(|v| *v >= 0.0) && values.contains(&0.0) {
        Some(Invariant {
            variable: variable.to_string(),
            kind: InvariantKind::NonNegative,
            expression: format!("{} >= 0", variable),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        })
    } else {
        None
    }
}

/// Infer positive invariant if all numeric values > 0.
fn infer_positive_invariant(
    variable: &str,
    values: &[f64],
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    if values.is_empty() {
        return None;
    }

    if values.iter().all(|v| *v > 0.0) {
        Some(Invariant {
            variable: variable.to_string(),
            kind: InvariantKind::Positive,
            expression: format!("{} > 0", variable),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        })
    } else {
        None
    }
}

/// Infer range invariant from min/max values.
fn infer_range_invariant(
    variable: &str,
    values: &[f64],
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    if values.is_empty() {
        return None;
    }

    let min_val = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Only report range if min != max (otherwise it's a constant)
    if min_val < max_val {
        Some(Invariant {
            variable: variable.to_string(),
            kind: InvariantKind::Range,
            expression: format!("{} <= {} <= {}", min_val, variable, max_val),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        })
    } else {
        None
    }
}

/// Infer ordering relation between two arguments.
fn infer_relation_invariant(
    observations: &[Observation],
    idx1: usize,
    idx2: usize,
    obs_count: u32,
    confidence: Confidence,
) -> Option<Invariant> {
    let pairs: Vec<(f64, f64)> = observations
        .iter()
        .filter_map(|o| {
            let v1 = o.args.get(idx1)?.as_f64()?;
            let v2 = o.args.get(idx2)?.as_f64()?;
            Some((v1, v2))
        })
        .collect();

    if pairs.is_empty() {
        return None;
    }

    let param1 = format!("arg{}", idx1);
    let param2 = format!("arg{}", idx2);

    // Check various relations
    if pairs.iter().all(|(v1, v2)| v1 < v2) {
        return Some(Invariant {
            variable: format!("{},{}", param1, param2),
            kind: InvariantKind::Relation,
            expression: format!("{} < {}", param1, param2),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        });
    }

    if pairs.iter().all(|(v1, v2)| v1 <= v2) {
        return Some(Invariant {
            variable: format!("{},{}", param1, param2),
            kind: InvariantKind::Relation,
            expression: format!("{} <= {}", param1, param2),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        });
    }

    if pairs.iter().all(|(v1, v2)| v1 > v2) {
        return Some(Invariant {
            variable: format!("{},{}", param1, param2),
            kind: InvariantKind::Relation,
            expression: format!("{} > {}", param1, param2),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        });
    }

    if pairs.iter().all(|(v1, v2)| v1 >= v2) {
        return Some(Invariant {
            variable: format!("{},{}", param1, param2),
            kind: InvariantKind::Relation,
            expression: format!("{} >= {}", param1, param2),
            confidence,
            observations: obs_count,
            counterexample_count: 0,
        });
    }

    None
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format invariants report as human-readable text.
pub fn format_invariants_text(report: &InvariantsReport) -> String {
    let mut lines = Vec::new();

    for fi in &report.functions {
        lines.push(format!(
            "Function: {} ({} observations)",
            fi.function_name, fi.observation_count
        ));

        if !fi.preconditions.is_empty() {
            for inv in &fi.preconditions {
                lines.push(format!(
                    "  Requires: {} [{}]",
                    inv.expression, inv.confidence
                ));
            }
        }

        if !fi.postconditions.is_empty() {
            for inv in &fi.postconditions {
                lines.push(format!(
                    "  Ensures: {} [{}]",
                    inv.expression, inv.confidence
                ));
            }
        }

        if fi.preconditions.is_empty() && fi.postconditions.is_empty() {
            lines.push("  (no invariants inferred)".to_string());
        }

        lines.push(String::new());
    }

    // Summary
    lines.push(format!(
        "Summary: {} observations, {} invariants",
        report.summary.total_observations, report.summary.total_invariants
    ));

    if !report.summary.by_kind.is_empty() {
        let kinds: Vec<_> = report
            .summary
            .by_kind
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();
        lines.push(format!("By kind: {}", kinds.join(", ")));
    }

    lines.join("\n")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_files(temp: &TempDir, source: &str, test: &str) -> (PathBuf, PathBuf) {
        let src_path = temp.path().join("src.py");
        let test_path = temp.path().join("test_src.py");
        fs::write(&src_path, source).unwrap();
        fs::write(&test_path, test).unwrap();
        (src_path, test_path)
    }

    #[test]
    fn test_invariants_type_inference() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def compute(x, y): return x + y",
            r#"
from src import compute

def test_compute_ints():
    assert compute(1, 2) == 3
    assert compute(5, 10) == 15
    assert compute(0, 0) == 0
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        assert!(!report.functions.is_empty());
        let func = report
            .functions
            .iter()
            .find(|f| f.function_name == "compute");
        assert!(func.is_some());

        let func = func.unwrap();
        // Should have type invariants
        let type_invs: Vec<_> = func
            .preconditions
            .iter()
            .filter(|i| i.kind == InvariantKind::Type)
            .collect();
        assert!(!type_invs.is_empty(), "Should detect type invariants");
    }

    #[test]
    fn test_invariants_non_null() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def process(data): return data.strip()",
            r#"
from src import process

def test_process_strings():
    assert process("hello") == "hello"
    assert process("  world  ") == "world"
    assert process("test") == "test"
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        // Should detect non-null for string argument
        let func = report
            .functions
            .iter()
            .find(|f| f.function_name == "process");
        assert!(func.is_some());

        let func = func.unwrap();
        let non_null_invs: Vec<_> = func
            .preconditions
            .iter()
            .filter(|i| i.kind == InvariantKind::NonNull)
            .collect();
        assert!(
            !non_null_invs.is_empty(),
            "Should detect non-null invariant"
        );
    }

    #[test]
    fn test_invariants_numeric_bounds() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def square(x): return x * x",
            r#"
from src import square

def test_square_positive():
    assert square(1) == 1
    assert square(2) == 4
    assert square(3) == 9
    assert square(10) == 100
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        let func = report
            .functions
            .iter()
            .find(|f| f.function_name == "square");
        assert!(func.is_some());

        let func = func.unwrap();
        // Should detect positive invariant (all values > 0)
        let positive_invs: Vec<_> = func
            .preconditions
            .iter()
            .filter(|i| i.kind == InvariantKind::Positive)
            .collect();
        assert!(
            !positive_invs.is_empty(),
            "Should detect positive invariant"
        );
    }

    #[test]
    fn test_invariants_ordering_relations() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def bounded_compute(start, end): return end - start",
            r#"
from src import bounded_compute

def test_bounded_compute():
    assert bounded_compute(0, 10) == 10
    assert bounded_compute(5, 15) == 10
    assert bounded_compute(100, 200) == 100
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        let func = report
            .functions
            .iter()
            .find(|f| f.function_name == "bounded_compute");
        assert!(func.is_some());

        let func = func.unwrap();
        // Should detect arg0 < arg1 relation
        let relation_invs: Vec<_> = func
            .preconditions
            .iter()
            .filter(|i| i.kind == InvariantKind::Relation)
            .collect();
        assert!(!relation_invs.is_empty(), "Should detect ordering relation");
    }

    #[test]
    fn test_invariants_confidence_scoring() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("func.py");
        let test_path = temp.path().join("test_func.py");

        fs::write(&src_path, "def identity(x): return x").unwrap();

        // Many observations = high confidence
        let mut test_code = String::from("from func import identity\n\n");
        for i in 0..15 {
            test_code.push_str(&format!(
                "def test_identity_{}(): assert identity({}) == {}\n",
                i, i, i
            ));
        }
        fs::write(&test_path, test_code).unwrap();

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        let func = report
            .functions
            .iter()
            .find(|f| f.function_name == "identity");
        assert!(func.is_some());

        let func = func.unwrap();
        assert!(func.observation_count >= 10);

        // With 15+ observations, confidence should be High
        for inv in &func.preconditions {
            assert_eq!(
                inv.confidence,
                Confidence::High,
                "Should have high confidence with 15 observations"
            );
        }
    }

    #[test]
    fn test_invariants_min_obs_filter() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def add(a, b): return a + b",
            r#"
from src import add

def test_add(): assert add(1, 2) == 3
"#,
        );

        // With min_obs=5, single observation should be filtered out
        let report = run_invariants(&src_path, &test_path, None, 5).unwrap();

        // Should have no functions reported (or functions with empty invariants)
        for func in &report.functions {
            assert!(
                func.preconditions.is_empty(),
                "Should filter out invariants with < 5 observations"
            );
        }
    }

    #[test]
    fn test_invariants_json_output() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def add(a, b): return a + b",
            r#"
from src import add

def test_add():
    assert add(1, 2) == 3
    assert add(2, 3) == 5
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();

        // Should serialize to JSON without error
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("functions"));
        assert!(json.contains("summary"));
    }

    #[test]
    fn test_invariants_text_output() {
        let temp = TempDir::new().unwrap();
        let (src_path, test_path) = create_test_files(
            &temp,
            "def add(a, b): return a + b",
            r#"
from src import add

def test_add():
    assert add(1, 2) == 3
"#,
        );

        let report = run_invariants(&src_path, &test_path, None, 1).unwrap();
        let text = format_invariants_text(&report);

        assert!(text.contains("Function:"));
        assert!(text.contains("observations"));
    }
}
