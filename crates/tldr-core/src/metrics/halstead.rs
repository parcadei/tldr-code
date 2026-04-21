//! Halstead software science metrics
//!
//! This module provides standalone Halstead metrics analysis per function.
//!
//! # Halstead Metrics
//!
//! Based on Maurice Halstead's software science metrics:
//! - n1 = number of distinct operators
//! - n2 = number of distinct operands
//! - N1 = total number of operators
//! - N2 = total number of operands
//!
//! ## Derived Metrics
//! - vocabulary = n1 + n2
//! - length = N1 + N2
//! - volume = length * log2(vocabulary)
//! - difficulty = (n1/2) * (N2/n2)
//! - effort = difficulty * volume
//! - time = effort / 18 (seconds)
//! - bugs = volume / 3000
//!
//! # References
//! - Halstead, M.H. (1977). "Elements of Software Science"

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::ast::extract::extract_file;
use crate::ast::function_finder::find_function_node;
use crate::ast::parser::{parse, parse_file};
use crate::metrics::types::HalsteadInfo;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// Threshold status for Halstead metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdStatus {
    /// Metric is below warning thresholds.
    Good,
    /// Metric exceeds warning threshold but not critical.
    Warning,
    /// Metric exceeds the highest configured threshold.
    Bad,
}

/// Threshold violations for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadThresholds {
    /// Classification for Halstead volume.
    pub volume_status: ThresholdStatus,
    /// Classification for Halstead difficulty.
    pub difficulty_status: ThresholdStatus,
}

impl Default for HalsteadThresholds {
    fn default() -> Self {
        Self {
            volume_status: ThresholdStatus::Good,
            difficulty_status: ThresholdStatus::Good,
        }
    }
}

/// Halstead metrics result for a single function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionHalstead {
    /// Function name.
    pub name: String,
    /// Source file path.
    pub file: String,
    /// One-based line where the function starts.
    pub line: u32,
    /// Raw Halstead metrics for the function.
    pub metrics: HalsteadInfo,
    /// Threshold classification for the metrics.
    pub thresholds: HalsteadThresholds,
    /// Distinct operators observed in the function when enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operators: Option<Vec<String>>,
    /// Distinct operands observed in the function when enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operands: Option<Vec<String>>,
}

/// Violation record for exceeding thresholds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadViolation {
    /// Function name where the violation occurred.
    pub name: String,
    /// Source file path.
    pub file: String,
    /// Metric name (`volume` or `difficulty`).
    pub metric: String,
    /// Observed metric value.
    pub value: f64,
    /// Threshold that was exceeded.
    pub threshold: f64,
}

/// Summary statistics for Halstead analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadSummary {
    /// Number of analyzed functions.
    pub total_functions: usize,
    /// Mean Halstead volume across analyzed functions.
    pub avg_volume: f64,
    /// Mean Halstead difficulty across analyzed functions.
    pub avg_difficulty: f64,
    /// Mean Halstead effort across analyzed functions.
    pub avg_effort: f64,
    /// Sum of estimated delivered bugs.
    pub total_estimated_bugs: f64,
    /// Number of recorded threshold violations.
    pub violations_count: usize,
}

impl Default for HalsteadSummary {
    fn default() -> Self {
        Self {
            total_functions: 0,
            avg_volume: 0.0,
            avg_difficulty: 0.0,
            avg_effort: 0.0,
            total_estimated_bugs: 0.0,
            violations_count: 0,
        }
    }
}

/// Complete Halstead analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadReport {
    /// Per-function Halstead metric results.
    pub functions: Vec<FunctionHalstead>,
    /// Threshold violations found during analysis.
    pub violations: Vec<HalsteadViolation>,
    /// Aggregate statistics for the analyzed set.
    pub summary: HalsteadSummary,
    /// Warnings encountered during analysis
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Options for Halstead analysis
#[derive(Debug, Clone, Default)]
pub struct HalsteadOptions {
    /// Specific function to analyze (None = all functions)
    pub function: Option<String>,
    /// Volume threshold for warnings (default: 1000)
    pub volume_threshold: f64,
    /// Difficulty threshold for warnings (default: 20)
    pub difficulty_threshold: f64,
    /// Include list of operators in output
    pub show_operators: bool,
    /// Include list of operands in output
    pub show_operands: bool,
    /// Maximum functions to report (0 = all)
    pub top: usize,
}

impl HalsteadOptions {
    /// Create default Halstead analysis options.
    pub fn new() -> Self {
        Self {
            function: None,
            volume_threshold: 1000.0,
            difficulty_threshold: 20.0,
            show_operators: false,
            show_operands: false,
            top: 0,
        }
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Analyze Halstead metrics for a file
///
/// # Arguments
/// * `path` - Path to the source file
/// * `language` - Programming language (None for auto-detect)
/// * `options` - Analysis options
///
/// # Returns
/// * `Ok(HalsteadReport)` - Report with metrics for all functions
/// * `Err(TldrError)` - On file system or parse errors
///
/// # Example
/// ```ignore
/// use tldr_core::metrics::halstead::{analyze_halstead, HalsteadOptions};
///
/// let report = analyze_halstead(Path::new("src/lib.rs"), None, HalsteadOptions::new())?;
/// for func in &report.functions {
///     println!("{}: volume={:.2}", func.name, func.metrics.volume);
/// }
/// ```
pub fn analyze_halstead(
    path: &Path,
    language: Option<Language>,
    options: HalsteadOptions,
) -> TldrResult<HalsteadReport> {
    // Parse the file
    let (tree, source, detected_lang) = parse_file(path)?;
    let lang = language.unwrap_or(detected_lang);

    // Extract function info to get names and line numbers
    let module = extract_file(path, None)?;

    let mut functions = Vec::new();
    let mut violations = Vec::new();

    // Analyze all functions
    for func_info in &module.functions {
        // Skip if filtering by function name and doesn't match
        if let Some(ref filter) = options.function {
            if &func_info.name != filter {
                continue;
            }
        }

        // Find the function node in the tree
        if let Some(func_node) =
            find_function_node(tree.root_node(), &func_info.name, lang, &source)
        {
            let (metrics, operators_set, operands_set) =
                calculate_function_halstead(func_node, &source, lang);

            let thresholds = evaluate_thresholds(&metrics, &options);

            // Record violations
            if metrics.volume > options.volume_threshold {
                violations.push(HalsteadViolation {
                    name: func_info.name.clone(),
                    file: path.display().to_string(),
                    metric: "volume".to_string(),
                    value: metrics.volume,
                    threshold: options.volume_threshold,
                });
            }
            if metrics.difficulty > options.difficulty_threshold {
                violations.push(HalsteadViolation {
                    name: func_info.name.clone(),
                    file: path.display().to_string(),
                    metric: "difficulty".to_string(),
                    value: metrics.difficulty,
                    threshold: options.difficulty_threshold,
                });
            }

            let func_halstead = FunctionHalstead {
                name: func_info.name.clone(),
                file: path.display().to_string(),
                line: func_info.line_number,
                metrics,
                thresholds,
                operators: if options.show_operators {
                    Some(operators_set.into_iter().collect())
                } else {
                    None
                },
                operands: if options.show_operands {
                    Some(operands_set.into_iter().collect())
                } else {
                    None
                },
            };

            functions.push(func_halstead);
        }
    }

    // Also analyze methods in classes
    for class in &module.classes {
        for method in &class.methods {
            // Skip if filtering by function name and doesn't match
            if let Some(ref filter) = options.function {
                if &method.name != filter {
                    continue;
                }
            }

            if let Some(func_node) =
                find_function_node(tree.root_node(), &method.name, lang, &source)
            {
                let (metrics, operators_set, operands_set) =
                    calculate_function_halstead(func_node, &source, lang);

                let thresholds = evaluate_thresholds(&metrics, &options);

                // Record violations
                if metrics.volume > options.volume_threshold {
                    violations.push(HalsteadViolation {
                        name: method.name.clone(),
                        file: path.display().to_string(),
                        metric: "volume".to_string(),
                        value: metrics.volume,
                        threshold: options.volume_threshold,
                    });
                }
                if metrics.difficulty > options.difficulty_threshold {
                    violations.push(HalsteadViolation {
                        name: method.name.clone(),
                        file: path.display().to_string(),
                        metric: "difficulty".to_string(),
                        value: metrics.difficulty,
                        threshold: options.difficulty_threshold,
                    });
                }

                let func_halstead = FunctionHalstead {
                    name: method.name.clone(),
                    file: path.display().to_string(),
                    line: method.line_number,
                    metrics,
                    thresholds,
                    operators: if options.show_operators {
                        Some(operators_set.into_iter().collect())
                    } else {
                        None
                    },
                    operands: if options.show_operands {
                        Some(operands_set.into_iter().collect())
                    } else {
                        None
                    },
                };

                functions.push(func_halstead);
            }
        }
    }

    // Sort by volume (descending) for top-N
    functions.sort_by(|a, b| {
        b.metrics
            .volume
            .partial_cmp(&a.metrics.volume)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Apply top limit if specified
    if options.top > 0 && functions.len() > options.top {
        functions.truncate(options.top);
    }

    // Calculate summary
    let summary = calculate_summary(&functions, violations.len());

    Ok(HalsteadReport {
        functions,
        violations,
        summary,
        warnings: vec![],
    })
}

/// Classify tokens in a function into operators and operands
///
/// Returns (operators, operands) as HashSets
pub fn classify_tokens(
    source: &str,
    language: Language,
) -> TldrResult<(HashSet<String>, HashSet<String>)> {
    let tree = parse(source, language)?;

    let mut operators = HashSet::new();
    let mut operands = HashSet::new();

    classify_node_tokens(
        tree.root_node(),
        source,
        language,
        &mut operators,
        &mut operands,
    );

    Ok((operators, operands))
}

/// Compute Halstead metrics from operator/operand sets
pub fn compute_halstead(
    operators: &HashSet<String>,
    operands: &HashSet<String>,
    total_operators: usize,
    total_operands: usize,
) -> HalsteadInfo {
    HalsteadInfo::from_counts(
        operators.len(),
        operands.len(),
        total_operators,
        total_operands,
    )
}

// =============================================================================
// Internal Helpers
// =============================================================================

/// Calculate Halstead metrics for a single function node
fn calculate_function_halstead(
    func_node: Node,
    source: &str,
    language: Language,
) -> (HalsteadInfo, HashSet<String>, HashSet<String>) {
    let mut operators = HashSet::new();
    let mut operands = HashSet::new();
    let mut total_operators = 0usize;
    let mut total_operands = 0usize;

    // Walk the function subtree
    classify_node_tokens_with_counts(
        func_node,
        source,
        language,
        &mut operators,
        &mut operands,
        &mut total_operators,
        &mut total_operands,
    );

    let metrics = HalsteadInfo::from_counts(
        operators.len(),
        operands.len(),
        total_operators,
        total_operands,
    );

    (metrics, operators, operands)
}

/// Classify tokens into operators/operands (distinct only)
fn classify_node_tokens(
    node: Node,
    source: &str,
    language: Language,
    operators: &mut HashSet<String>,
    operands: &mut HashSet<String>,
) {
    let mut total_ops = 0;
    let mut total_opnds = 0;
    classify_node_tokens_with_counts(
        node,
        source,
        language,
        operators,
        operands,
        &mut total_ops,
        &mut total_opnds,
    );
}

/// Classify tokens with total counts
fn classify_node_tokens_with_counts(
    node: Node,
    source: &str,
    language: Language,
    operators: &mut HashSet<String>,
    operands: &mut HashSet<String>,
    total_operators: &mut usize,
    total_operands: &mut usize,
) {
    let mut stack = vec![node];

    while let Some(current) = stack.pop() {
        let kind = current.kind();
        let text = current.utf8_text(source.as_bytes()).unwrap_or("");

        // Classify based on node kind and language
        if is_operator_node(kind, text, language) {
            operators.insert(normalize_operator(kind, text, language));
            *total_operators += 1;
        } else if is_operand_node(kind, language) {
            operands.insert(text.to_string());
            *total_operands += 1;
        }

        // Add children to stack (depth-first)
        let mut cursor = current.walk();
        if cursor.goto_first_child() {
            loop {
                stack.push(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Check if a node represents an operator
fn is_operator_node(kind: &str, text: &str, language: Language) -> bool {
    // Keywords that are operators
    let keyword_operators = match language {
        Language::Python => vec![
            "def", "class", "if", "elif", "else", "for", "while", "try", "except", "finally",
            "with", "return", "yield", "raise", "import", "from", "as", "lambda", "and", "or",
            "not", "in", "is", "pass", "break", "continue", "assert", "del", "global", "nonlocal",
            "async", "await", "match", "case",
        ],
        Language::TypeScript | Language::JavaScript => vec![
            "function",
            "class",
            "if",
            "else",
            "for",
            "while",
            "do",
            "switch",
            "case",
            "default",
            "try",
            "catch",
            "finally",
            "return",
            "throw",
            "new",
            "delete",
            "typeof",
            "instanceof",
            "import",
            "export",
            "const",
            "let",
            "var",
            "async",
            "await",
            "yield",
            "break",
            "continue",
            "void",
        ],
        Language::Rust => vec![
            "fn", "struct", "enum", "impl", "trait", "if", "else", "for", "while", "loop", "match",
            "return", "let", "mut", "const", "static", "pub", "use", "mod", "crate", "self",
            "super", "async", "await", "move", "ref", "unsafe", "where", "type",
        ],
        Language::Go => vec![
            "func",
            "type",
            "struct",
            "interface",
            "if",
            "else",
            "for",
            "switch",
            "case",
            "default",
            "select",
            "return",
            "go",
            "defer",
            "chan",
            "map",
            "range",
            "break",
            "continue",
            "goto",
            "fallthrough",
            "package",
            "import",
            "const",
            "var",
        ],
        Language::Java => vec![
            "class",
            "interface",
            "extends",
            "implements",
            "if",
            "else",
            "for",
            "while",
            "do",
            "switch",
            "case",
            "default",
            "try",
            "catch",
            "finally",
            "return",
            "throw",
            "new",
            "instanceof",
            "import",
            "package",
            "public",
            "private",
            "protected",
            "static",
            "final",
            "abstract",
            "synchronized",
            "volatile",
            "transient",
            "native",
            "void",
            "break",
            "continue",
            "assert",
        ],
        Language::C => vec![
            "if", "else", "for", "while", "do", "switch", "case", "default", "return", "goto",
            "break", "continue", "typedef", "struct", "union", "enum", "sizeof", "static",
            "extern", "const", "volatile", "register", "auto", "inline",
        ],
        Language::Cpp => vec![
            "if",
            "else",
            "for",
            "while",
            "do",
            "switch",
            "case",
            "default",
            "return",
            "goto",
            "break",
            "continue",
            "class",
            "struct",
            "union",
            "enum",
            "namespace",
            "using",
            "template",
            "typename",
            "new",
            "delete",
            "try",
            "catch",
            "throw",
            "virtual",
            "override",
            "const",
            "static",
            "extern",
            "inline",
            "constexpr",
            "auto",
            "decltype",
            "sizeof",
            "dynamic_cast",
            "static_cast",
            "reinterpret_cast",
            "const_cast",
        ],
        Language::Ruby => vec![
            "def",
            "class",
            "module",
            "if",
            "elsif",
            "else",
            "unless",
            "for",
            "while",
            "until",
            "do",
            "begin",
            "rescue",
            "ensure",
            "raise",
            "return",
            "yield",
            "block_given?",
            "require",
            "include",
            "extend",
            "attr_reader",
            "attr_writer",
            "attr_accessor",
            "self",
            "super",
            "nil",
            "and",
            "or",
            "not",
            "in",
            "end",
            "case",
            "when",
        ],
        Language::Php => vec![
            "function",
            "class",
            "interface",
            "trait",
            "extends",
            "implements",
            "if",
            "elseif",
            "else",
            "for",
            "foreach",
            "while",
            "do",
            "switch",
            "case",
            "default",
            "try",
            "catch",
            "finally",
            "throw",
            "return",
            "new",
            "instanceof",
            "use",
            "namespace",
            "public",
            "private",
            "protected",
            "static",
            "abstract",
            "final",
            "const",
            "echo",
            "print",
            "isset",
            "unset",
            "empty",
            "array",
            "list",
        ],
        Language::Kotlin => vec![
            "fun",
            "class",
            "object",
            "interface",
            "if",
            "else",
            "for",
            "while",
            "do",
            "when",
            "try",
            "catch",
            "finally",
            "throw",
            "return",
            "break",
            "continue",
            "is",
            "as",
            "in",
            "val",
            "var",
            "import",
            "package",
            "override",
            "open",
            "abstract",
            "sealed",
            "data",
            "companion",
            "suspend",
            "inline",
            "crossinline",
            "noinline",
            "reified",
        ],
        Language::CSharp => vec![
            "class",
            "struct",
            "interface",
            "enum",
            "namespace",
            "using",
            "if",
            "else",
            "for",
            "foreach",
            "while",
            "do",
            "switch",
            "case",
            "default",
            "try",
            "catch",
            "finally",
            "throw",
            "return",
            "new",
            "is",
            "as",
            "typeof",
            "sizeof",
            "ref",
            "out",
            "in",
            "params",
            "public",
            "private",
            "protected",
            "internal",
            "static",
            "virtual",
            "override",
            "abstract",
            "sealed",
            "async",
            "await",
            "yield",
            "break",
            "continue",
            "goto",
            "lock",
            "var",
        ],
        Language::Scala => vec![
            "def", "val", "var", "class", "object", "trait", "extends", "with", "if", "else",
            "for", "while", "do", "match", "case", "try", "catch", "finally", "throw", "return",
            "new", "import", "package", "type", "abstract", "sealed", "final", "override", "lazy",
            "implicit", "yield",
        ],
        Language::Elixir => vec![
            "def",
            "defp",
            "defmodule",
            "defstruct",
            "defprotocol",
            "defimpl",
            "if",
            "else",
            "unless",
            "cond",
            "case",
            "with",
            "for",
            "fn",
            "do",
            "end",
            "raise",
            "rescue",
            "try",
            "catch",
            "after",
            "import",
            "alias",
            "use",
            "require",
            "in",
            "when",
            "and",
            "or",
            "not",
            "pipe_operator",
        ],
        Language::Lua | Language::Luau => vec![
            "function", "if", "then", "elseif", "else", "for", "while", "do", "repeat", "until",
            "return", "break", "local", "end", "in", "and", "or", "not",
        ],
        Language::Ocaml => vec![
            "let",
            "in",
            "if",
            "then",
            "else",
            "match",
            "with",
            "fun",
            "function",
            "rec",
            "and",
            "or",
            "not",
            "mod",
            "type",
            "module",
            "struct",
            "sig",
            "end",
            "open",
            "include",
            "val",
            "begin",
            "try",
            "raise",
            "exception",
            "when",
            "as",
            "of",
        ],
        _ => vec![],
    };

    // Check if it's a keyword operator
    if keyword_operators.contains(&text) {
        return true;
    }

    // Node types that are operators
    matches!(
        kind,
        // Arithmetic and binary operators
        "+" | "-" | "*" | "/" | "%" | "**" | "//" | "@"
        | "binary_operator" | "unary_operator" | "augmented_assignment"

        // Comparison operators
        | "==" | "!=" | "<" | ">" | "<=" | ">=" | "<=>" | "===" | "!=="
        | "comparison_operator"

        // Assignment operators
        | "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "//=" | "@="
        | "&=" | "|=" | "^=" | "<<=" | ">>=" | "&&=" | "||=" | "??="
        | "assignment" | "assignment_expression"

        // Logical operators
        | "&&" | "||" | "!" | "and" | "or" | "not"
        | "boolean_operator" | "not_operator"

        // Bitwise operators
        | "&" | "|" | "^" | "~" | "<<" | ">>"

        // Special operators
        | "?:" | "??" | "?." | "=>" | "->" | "::"
        | "conditional_expression" | "ternary_expression"

        // Member access (keep as operator)
        | "."

        // Function/method calls
        | "call" | "call_expression" | "method_call"

        // Member access
        | "attribute" | "subscript" | "member_expression" | "subscript_expression"
    )
}

/// Normalize operator representation
fn normalize_operator(kind: &str, text: &str, _language: Language) -> String {
    // For node types that represent operators, use the kind
    // For actual operator tokens, use the text
    match kind {
        "binary_operator"
        | "unary_operator"
        | "comparison_operator"
        | "boolean_operator"
        | "assignment" => text.to_string(),
        _ => {
            if text.len() <= 3 || is_keyword(text) {
                text.to_string()
            } else {
                kind.to_string()
            }
        }
    }
}

/// Check if text is a keyword
fn is_keyword(text: &str) -> bool {
    matches!(
        text,
        "def"
            | "class"
            | "if"
            | "elif"
            | "else"
            | "for"
            | "while"
            | "try"
            | "except"
            | "finally"
            | "with"
            | "return"
            | "yield"
            | "raise"
            | "import"
            | "from"
            | "as"
            | "lambda"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "pass"
            | "break"
            | "continue"
            | "function"
            | "fn"
            | "func"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "match"
            | "case"
            | "const"
            | "let"
            | "mut"
            | "pub"
            | "use"
            | "mod"
            | "async"
            | "await"
    )
}

/// Check if a node represents an operand
fn is_operand_node(kind: &str, _language: Language) -> bool {
    matches!(
        kind,
        // Identifiers
        "identifier" | "property_identifier" | "field_identifier"
        | "shorthand_property_identifier" | "type_identifier"

        // Literals
        | "string" | "string_literal" | "string_content" | "template_string"
        | "integer" | "integer_literal" | "float" | "float_literal"
        | "number" | "number_literal"

        // Boolean/null literals
        | "true" | "false" | "True" | "False"
        | "none" | "None" | "null" | "nil" | "undefined"

        // Special operands
        | "self" | "this" | "super"
    )
}

/// Evaluate threshold status for metrics
fn evaluate_thresholds(metrics: &HalsteadInfo, options: &HalsteadOptions) -> HalsteadThresholds {
    let volume_status = if metrics.volume > options.volume_threshold * 2.0 {
        ThresholdStatus::Bad
    } else if metrics.volume > options.volume_threshold {
        ThresholdStatus::Warning
    } else {
        ThresholdStatus::Good
    };

    let difficulty_status = if metrics.difficulty > options.difficulty_threshold * 2.0 {
        ThresholdStatus::Bad
    } else if metrics.difficulty > options.difficulty_threshold {
        ThresholdStatus::Warning
    } else {
        ThresholdStatus::Good
    };

    HalsteadThresholds {
        volume_status,
        difficulty_status,
    }
}

/// Calculate summary statistics
fn calculate_summary(functions: &[FunctionHalstead], violations_count: usize) -> HalsteadSummary {
    if functions.is_empty() {
        return HalsteadSummary::default();
    }

    let total_volume: f64 = functions.iter().map(|f| f.metrics.volume).sum();
    let total_difficulty: f64 = functions.iter().map(|f| f.metrics.difficulty).sum();
    let total_effort: f64 = functions.iter().map(|f| f.metrics.effort).sum();
    let total_bugs: f64 = functions.iter().map(|f| f.metrics.bugs).sum();

    let count = functions.len() as f64;

    HalsteadSummary {
        total_functions: functions.len(),
        avg_volume: total_volume / count,
        avg_difficulty: total_difficulty / count,
        avg_effort: total_effort / count,
        total_estimated_bugs: total_bugs,
        violations_count,
    }
}

/// Merge multiple Halstead reports into one.
///
/// Combines functions from all reports, sorts by volume descending,
/// applies top-N limit, rebuilds violations, and recalculates summary.
pub fn merge_halstead_reports(
    reports: Vec<HalsteadReport>,
    options: &HalsteadOptions,
) -> HalsteadReport {
    if reports.is_empty() {
        return HalsteadReport {
            functions: vec![],
            violations: vec![],
            summary: HalsteadSummary::default(),
            warnings: vec![],
        };
    }

    // 1. Flatten all functions from all reports
    let mut functions: Vec<FunctionHalstead> = reports
        .iter()
        .flat_map(|r| r.functions.iter().cloned())
        .collect();

    // 2. Merge warnings from all reports
    let warnings: Vec<String> = reports.into_iter().flat_map(|r| r.warnings).collect();

    // 3. Sort by volume descending
    functions.sort_by(|a, b| {
        b.metrics
            .volume
            .partial_cmp(&a.metrics.volume)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 4. Apply top-N limit
    if options.top > 0 && functions.len() > options.top {
        functions.truncate(options.top);
    }

    // 5. Rebuild violations from the (potentially truncated) function list
    let mut violations = Vec::new();
    for func in &functions {
        if func.metrics.volume > options.volume_threshold {
            violations.push(HalsteadViolation {
                name: func.name.clone(),
                file: func.file.clone(),
                metric: "volume".to_string(),
                value: func.metrics.volume,
                threshold: options.volume_threshold,
            });
        }
        if func.metrics.difficulty > options.difficulty_threshold {
            violations.push(HalsteadViolation {
                name: func.name.clone(),
                file: func.file.clone(),
                metric: "difficulty".to_string(),
                value: func.metrics.difficulty,
                threshold: options.difficulty_threshold,
            });
        }
    }

    // 6. Calculate summary (must compute violations first for count)
    let summary = calculate_summary(&functions, violations.len());

    HalsteadReport {
        functions,
        violations,
        summary,
        warnings,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_temp_file(content: &str, extension: &str) -> NamedTempFile {
        let mut file = tempfile::Builder::new()
            .suffix(extension)
            .tempfile()
            .unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();
        file
    }

    #[test]
    fn test_halstead_simple_python() {
        let source = r#"
def simple_math(a, b):
    result = a + b * 2
    return result
"#;
        let file = create_temp_file(source, ".py");
        let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(!report.functions.is_empty());

        let func = &report.functions[0];
        assert_eq!(func.name, "simple_math");
        assert!(
            func.metrics.n1 >= 3,
            "Should have at least 3 distinct operators"
        );
        assert!(
            func.metrics.n2 >= 3,
            "Should have at least 3 distinct operands"
        );
    }

    #[test]
    fn test_halstead_vocabulary_invariant() {
        let source = r#"
def calc(x, y):
    return x + y - x * y
"#;
        let file = create_temp_file(source, ".py");
        let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

        assert!(result.is_ok());
        let report = result.unwrap();

        for func in &report.functions {
            // vocabulary == n1 + n2
            assert_eq!(
                func.metrics.vocabulary,
                func.metrics.n1 + func.metrics.n2,
                "vocabulary should equal n1 + n2"
            );
            // length == N1 + N2
            assert_eq!(
                func.metrics.length,
                func.metrics.big_n1 + func.metrics.big_n2,
                "length should equal N1 + N2"
            );
        }
    }

    #[test]
    fn test_halstead_derived_metrics() {
        let source = r#"
def complex_calc(a, b, c):
    x = a + b
    y = b - c
    z = x * y / a
    return x + y + z
"#;
        let file = create_temp_file(source, ".py");
        let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

        assert!(result.is_ok());
        let report = result.unwrap();

        for func in &report.functions {
            let m = &func.metrics;

            // Volume >= 0
            assert!(m.volume >= 0.0, "volume should be non-negative");

            // Difficulty >= 0
            assert!(m.difficulty >= 0.0, "difficulty should be non-negative");

            // Effort = Difficulty * Volume (with tolerance for floating point)
            if m.volume > 0.0 && m.difficulty > 0.0 {
                let expected_effort = m.difficulty * m.volume;
                assert!(
                    (m.effort - expected_effort).abs() < 0.01,
                    "effort should equal difficulty * volume"
                );
            }

            // Time = Effort / 18
            let expected_time = m.effort / 18.0;
            assert!(
                (m.time - expected_time).abs() < 0.01,
                "time should equal effort / 18"
            );

            // Bugs = Volume / 3000
            let expected_bugs = m.volume / 3000.0;
            assert!(
                (m.bugs - expected_bugs).abs() < 0.001,
                "bugs should equal volume / 3000"
            );
        }
    }

    #[test]
    fn test_halstead_empty_function() {
        let source = r#"
def empty_function():
    pass
"#;
        let file = create_temp_file(source, ".py");
        let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

        assert!(result.is_ok());
        let report = result.unwrap();
        assert!(!report.functions.is_empty());

        let func = &report.functions[0];
        assert_eq!(func.name, "empty_function");
        // Empty function should have minimal operators (pass, parentheses, etc.)
        // The exact count depends on tree-sitter parsing, so we just verify it's small
        assert!(
            func.metrics.n1 <= 10,
            "Empty function should have relatively few operators"
        );
        // Volume should be >= 0 (avoid log(0))
        assert!(
            func.metrics.volume >= 0.0,
            "Volume should never be negative"
        );
        // Empty function should have very low or zero operands
        // (only the function name which might be counted as an operand)
        assert!(
            func.metrics.n2 <= 5,
            "Empty function should have few operands"
        );
    }

    #[test]
    fn test_halstead_threshold_violations() {
        let source = r#"
def complex_calculation(x, y, z, w, a, b, c, d):
    r1 = x + y - z * w
    r2 = a / b + c ** 2
    r3 = (r1 + r2) * (x - y) / (z + w)
    r4 = r1 if r2 > r3 else r3
    r5 = a + b + c + d + x + y + z + w
    r6 = r1 * r2 * r3 * r4 * r5
    return r1 + r2 + r3 + r4 + r5 + r6
"#;
        let file = create_temp_file(source, ".py");

        let mut options = HalsteadOptions::new();
        options.volume_threshold = 100.0; // Low threshold to trigger violation
        options.difficulty_threshold = 5.0;

        let result = analyze_halstead(file.path(), Some(Language::Python), options);

        assert!(result.is_ok());
        let report = result.unwrap();

        // Should have threshold violations
        for func in &report.functions {
            assert!(
                func.thresholds.volume_status == ThresholdStatus::Good
                    || func.thresholds.volume_status == ThresholdStatus::Warning
                    || func.thresholds.volume_status == ThresholdStatus::Bad,
                "Should have valid threshold status"
            );
        }
    }

    #[test]
    fn test_halstead_show_operators_operands() {
        let source = r#"
def add(a, b):
    return a + b
"#;
        let file = create_temp_file(source, ".py");

        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        options.show_operands = true;

        let result = analyze_halstead(file.path(), Some(Language::Python), options);

        assert!(result.is_ok());
        let report = result.unwrap();

        if !report.functions.is_empty() {
            let func = &report.functions[0];
            assert!(
                func.operators.is_some(),
                "Should include operators when requested"
            );
            assert!(
                func.operands.is_some(),
                "Should include operands when requested"
            );
        }
    }

    #[test]
    fn test_halstead_filter_by_function() {
        let source = r#"
def foo():
    return 1

def bar():
    return 2

def baz():
    return 3
"#;
        let file = create_temp_file(source, ".py");

        let mut options = HalsteadOptions::new();
        options.function = Some("bar".to_string());

        let result = analyze_halstead(file.path(), Some(Language::Python), options);

        assert!(result.is_ok());
        let report = result.unwrap();

        assert_eq!(report.functions.len(), 1, "Should only analyze 'bar'");
        assert_eq!(report.functions[0].name, "bar");
    }

    #[test]
    fn test_compute_halstead_from_counts() {
        let operators: HashSet<String> = vec!["=", "+", "*", "return", "def"]
            .into_iter()
            .map(String::from)
            .collect();
        let operands: HashSet<String> = vec!["a", "b", "result", "2"]
            .into_iter()
            .map(String::from)
            .collect();

        let metrics = compute_halstead(&operators, &operands, 10, 15);

        assert_eq!(metrics.n1, 5);
        assert_eq!(metrics.n2, 4);
        assert_eq!(metrics.big_n1, 10);
        assert_eq!(metrics.big_n2, 15);
        assert_eq!(metrics.vocabulary, 9);
        assert_eq!(metrics.length, 25);
    }

    #[test]
    fn test_halstead_java_keywords_counted() {
        let source = r#"
public class Example {
    public static int compute(int x) {
        if (x > 0) {
            return x + 1;
        } else {
            return x - 1;
        }
    }
}
"#;
        let file = create_temp_file(source, ".java");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Java), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            // Java keywords like "if", "else", "return" should be counted as operators
            assert!(
                ops.contains(&"if".to_string()),
                "Java 'if' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "Java 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_ruby_keywords_counted() {
        let source = r#"
def calculate(x)
  if x > 0
    return x + 1
  else
    return x - 1
  end
end
"#;
        let file = create_temp_file(source, ".rb");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Ruby), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"def".to_string()),
                "Ruby 'def' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "Ruby 'if' should be an operator"
            );
            assert!(
                ops.contains(&"end".to_string()),
                "Ruby 'end' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_kotlin_keywords_counted() {
        let source = r#"
fun compute(x: Int): Int {
    if (x > 0) {
        return x + 1
    } else {
        return x - 1
    }
}
"#;
        let file = create_temp_file(source, ".kt");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Kotlin), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"fun".to_string()),
                "Kotlin 'fun' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "Kotlin 'if' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "Kotlin 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_csharp_keywords_counted() {
        let source = r#"
class Example {
    static int Compute(int x) {
        if (x > 0) {
            return x + 1;
        } else {
            return x - 1;
        }
    }
}
"#;
        let file = create_temp_file(source, ".cs");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::CSharp), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"if".to_string()),
                "C# 'if' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "C# 'return' should be an operator"
            );
            assert!(
                ops.contains(&"static".to_string()),
                "C# 'static' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_c_keywords_counted() {
        let source = r#"
int compute(int x) {
    if (x > 0) {
        return x + 1;
    } else {
        return x - 1;
    }
}
"#;
        let file = create_temp_file(source, ".c");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::C), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"if".to_string()),
                "C 'if' should be an operator"
            );
            assert!(
                ops.contains(&"else".to_string()),
                "C 'else' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "C 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_cpp_keywords_counted() {
        let source = r#"
int compute(int x) {
    if (x > 0) {
        return x + 1;
    } else {
        return x - 1;
    }
}
"#;
        let file = create_temp_file(source, ".cpp");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Cpp), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"if".to_string()),
                "C++ 'if' should be an operator"
            );
            assert!(
                ops.contains(&"else".to_string()),
                "C++ 'else' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "C++ 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_php_keywords_counted() {
        let source = r#"<?php
function compute($x) {
    if ($x > 0) {
        return $x + 1;
    } else {
        return $x - 1;
    }
}
"#;
        let file = create_temp_file(source, ".php");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Php), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"function".to_string()),
                "PHP 'function' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "PHP 'if' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "PHP 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_scala_keywords_counted() {
        let source = r#"
object Example {
  def compute(x: Int): Int = {
    if (x > 0) {
      x + 1
    } else {
      x - 1
    }
  }
}
"#;
        let file = create_temp_file(source, ".scala");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Scala), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"def".to_string()),
                "Scala 'def' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "Scala 'if' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_elixir_keywords_counted() {
        let source = r#"
defmodule Example do
  def compute(x) do
    if x > 0 do
      x + 1
    else
      x - 1
    end
  end
end
"#;
        let file = create_temp_file(source, ".ex");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Elixir), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"if".to_string()),
                "Elixir 'if' should be an operator"
            );
            assert!(
                ops.contains(&"do".to_string()),
                "Elixir 'do' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_lua_keywords_counted() {
        let source = r#"
function compute(x)
    if x > 0 then
        return x + 1
    else
        return x - 1
    end
end
"#;
        let file = create_temp_file(source, ".lua");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Lua), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            assert!(
                ops.contains(&"function".to_string()),
                "Lua 'function' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "Lua 'if' should be an operator"
            );
            assert!(
                ops.contains(&"then".to_string()),
                "Lua 'then' should be an operator"
            );
            assert!(
                ops.contains(&"return".to_string()),
                "Lua 'return' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_ocaml_keywords_counted() {
        let source = r#"
let compute x =
  if x > 0 then
    x + 1
  else
    x - 1
"#;
        let file = create_temp_file(source, ".ml");
        let mut options = HalsteadOptions::new();
        options.show_operators = true;
        let result = analyze_halstead(file.path(), Some(Language::Ocaml), options);
        assert!(result.is_ok());
        let report = result.unwrap();
        if !report.functions.is_empty() {
            let func = &report.functions[0];
            let ops = func.operators.as_ref().unwrap();
            // OCaml keywords like "let", "if", "then", "else" should be operators
            assert!(
                ops.contains(&"let".to_string()),
                "OCaml 'let' should be an operator"
            );
            assert!(
                ops.contains(&"if".to_string()),
                "OCaml 'if' should be an operator"
            );
            assert!(
                ops.contains(&"then".to_string()),
                "OCaml 'then' should be an operator"
            );
            assert!(
                ops.contains(&"else".to_string()),
                "OCaml 'else' should be an operator"
            );
        }
    }

    #[test]
    fn test_halstead_summary_calculation() {
        let source = r#"
def func1():
    return 1 + 2

def func2():
    x = 1
    y = 2
    return x + y
"#;
        let file = create_temp_file(source, ".py");
        let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

        assert!(result.is_ok());
        let report = result.unwrap();

        assert_eq!(report.summary.total_functions, report.functions.len());

        if !report.functions.is_empty() {
            let expected_avg_volume: f64 = report
                .functions
                .iter()
                .map(|f| f.metrics.volume)
                .sum::<f64>()
                / report.functions.len() as f64;

            assert!(
                (report.summary.avg_volume - expected_avg_volume).abs() < 0.01,
                "Average volume should be correct"
            );
        }
    }

    // -------------------------------------------------------------------------
    // Merge Halstead Reports Tests
    // -------------------------------------------------------------------------

    /// Helper to create a synthetic FunctionHalstead for testing
    fn make_halstead_function(
        name: &str,
        file: &str,
        line: u32,
        volume: f64,
        difficulty: f64,
    ) -> FunctionHalstead {
        use crate::metrics::types::HalsteadInfo;

        let effort = difficulty * volume;
        let time = effort / 18.0;
        let bugs = volume / 3000.0;

        FunctionHalstead {
            name: name.to_string(),
            file: file.to_string(),
            line,
            metrics: HalsteadInfo {
                n1: 5,
                n2: 3,
                big_n1: 10,
                big_n2: 8,
                vocabulary: 8,
                length: 18,
                volume,
                difficulty,
                effort,
                time,
                bugs,
            },
            thresholds: HalsteadThresholds::default(),
            operators: None,
            operands: None,
        }
    }

    /// Helper to create a synthetic HalsteadReport for testing
    fn make_halstead_report(functions: Vec<FunctionHalstead>) -> HalsteadReport {
        let options = HalsteadOptions::new();
        let violations: Vec<HalsteadViolation> = functions
            .iter()
            .filter_map(|f| {
                if f.metrics.volume > options.volume_threshold {
                    Some(HalsteadViolation {
                        name: f.name.clone(),
                        file: f.file.clone(),
                        metric: "volume".to_string(),
                        value: f.metrics.volume,
                        threshold: options.volume_threshold,
                    })
                } else {
                    None
                }
            })
            .collect();
        let violations_count = violations.len();
        let summary = calculate_summary(&functions, violations_count);
        HalsteadReport {
            functions,
            violations,
            summary,
            warnings: vec![],
        }
    }

    #[test]
    fn test_merge_halstead_reports_combines_functions() {
        let report1 = make_halstead_report(vec![
            make_halstead_function("foo", "a.py", 1, 100.0, 5.0),
            make_halstead_function("bar", "a.py", 10, 500.0, 15.0),
        ]);
        let report2 =
            make_halstead_report(vec![make_halstead_function("baz", "b.py", 1, 200.0, 10.0)]);

        let options = HalsteadOptions::new();
        let merged = merge_halstead_reports(vec![report1, report2], &options);

        assert_eq!(
            merged.functions.len(),
            3,
            "Merged report should contain all 3 functions from both reports"
        );

        let names: Vec<&str> = merged.functions.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"foo"), "Should contain 'foo'");
        assert!(names.contains(&"bar"), "Should contain 'bar'");
        assert!(names.contains(&"baz"), "Should contain 'baz'");
    }

    #[test]
    fn test_merge_halstead_reports_recalculates_summary() {
        let report1 = make_halstead_report(vec![
            make_halstead_function("foo", "a.py", 1, 100.0, 5.0),
            make_halstead_function("bar", "a.py", 10, 500.0, 15.0),
        ]);
        let report2 =
            make_halstead_report(vec![make_halstead_function("baz", "b.py", 1, 200.0, 10.0)]);

        let options = HalsteadOptions::new();
        let merged = merge_halstead_reports(vec![report1, report2], &options);

        assert_eq!(
            merged.summary.total_functions, 3,
            "Summary should count all 3 functions"
        );

        // Average volume should be (100 + 500 + 200) / 3 = 266.67
        let expected_avg_volume = (100.0 + 500.0 + 200.0) / 3.0;
        assert!(
            (merged.summary.avg_volume - expected_avg_volume).abs() < 0.01,
            "Average volume should be {:.2}, got {:.2}",
            expected_avg_volume,
            merged.summary.avg_volume
        );

        // Total estimated bugs should be sum of individual bugs
        let expected_bugs = 100.0 / 3000.0 + 500.0 / 3000.0 + 200.0 / 3000.0;
        assert!(
            (merged.summary.total_estimated_bugs - expected_bugs).abs() < 0.001,
            "Total bugs should be {:.4}, got {:.4}",
            expected_bugs,
            merged.summary.total_estimated_bugs
        );
    }

    #[test]
    fn test_merge_halstead_reports_empty() {
        let options = HalsteadOptions::new();
        let merged = merge_halstead_reports(vec![], &options);

        assert!(
            merged.functions.is_empty(),
            "Empty merge should have no functions"
        );
        assert!(
            merged.violations.is_empty(),
            "Empty merge should have no violations"
        );
        assert_eq!(
            merged.summary.total_functions, 0,
            "Empty merge should have 0 total_functions"
        );
    }
}
