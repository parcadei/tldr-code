//! Technical Debt Analysis using SQALE Method
//!
//! This module analyzes source code to estimate technical debt using the
//! SQALE (Software Quality Assessment based on Lifecycle Expectations) method.
//!
//! # Overview
//!
//! The debt command detects issues that contribute to technical debt:
//! - Cyclomatic complexity violations
//! - God classes (low cohesion)
//! - Long methods and parameter lists
//! - TODO/FIXME/HACK comments
//! - Deep nesting
//! - High coupling
//! - Missing documentation
//!
//! Each issue is assigned a remediation time in minutes, which is aggregated
//! into summary statistics including debt ratio and density.
//!
//! # Example
//!
//! ```ignore
//! use tldr_core::quality::debt::{analyze_debt, DebtOptions};
//!
//! let options = DebtOptions {
//!     path: PathBuf::from("src/"),
//!     ..Default::default()
//! };
//!
//! let report = analyze_debt(options)?;
//! println!("Total debt: {} hours", report.summary.total_hours);
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use rayon::prelude::*;

use crate::ast::function_finder::{
    get_function_body as shared_get_function_body, get_function_node_kinds_vec,
};
use crate::ast::parser::parse;
use crate::metrics::calculate_all_complexities_from_tree;
use crate::quality::cohesion::extract_self_accesses;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Enums
// =============================================================================

/// SQALE categories based on ISO 25010 quality characteristics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DebtCategory {
    /// Code correctness, error handling, TODO/FIXME markers
    Reliability,
    /// Vulnerabilities, injection risks (not currently implemented)
    Security,
    /// Code clarity, complexity, naming, documentation
    Maintainability,
    /// Performance bottlenecks (not currently implemented)
    Efficiency,
    /// Coupling, cohesion, god classes
    Changeability,
    /// Cyclomatic complexity, dependencies
    Testability,
}

/// Debt rules with remediation estimates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebtRule {
    /// Cyclomatic complexity > 10 (20 minutes)
    ComplexityHigh,
    /// Cyclomatic complexity > 15 (30 minutes)
    ComplexityVeryHigh,
    /// Cyclomatic complexity > 25 (60 minutes)
    ComplexityExtreme,
    /// Class with >20 methods and LCOM4 > 0.8 (60 minutes)
    GodClass,
    /// Method with >100 lines of code (30 minutes)
    LongMethod,
    /// Function with >5 parameters (15 minutes)
    LongParamList,
    /// Nesting > 4 levels (15 minutes)
    DeepNesting,
    /// TODO/FIXME/HACK/XXX comment (10 minutes)
    TodoComment,
    /// Module coupling too high (20 minutes)
    HighCoupling,
    /// Public API missing documentation (10 minutes)
    MissingDocs,
}

impl DebtRule {
    /// Get remediation time in minutes
    pub fn minutes(&self) -> u32 {
        match self {
            Self::ComplexityHigh => 20,
            Self::ComplexityVeryHigh => 30,
            Self::ComplexityExtreme => 60,
            Self::GodClass => 60,
            Self::LongMethod => 30,
            Self::LongParamList => 15,
            Self::DeepNesting => 15,
            Self::TodoComment => 10,
            Self::HighCoupling => 20,
            Self::MissingDocs => 10,
        }
    }

    /// Get SQALE category for this rule
    pub fn category(&self) -> DebtCategory {
        match self {
            Self::ComplexityHigh
            | Self::ComplexityVeryHigh
            | Self::ComplexityExtreme
            | Self::LongMethod
            | Self::DeepNesting
            | Self::MissingDocs => DebtCategory::Maintainability,
            Self::GodClass | Self::HighCoupling => DebtCategory::Changeability,
            Self::LongParamList => DebtCategory::Testability,
            Self::TodoComment => DebtCategory::Reliability,
        }
    }

    /// Get human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            Self::ComplexityHigh => "Cyclomatic complexity > 10",
            Self::ComplexityVeryHigh => "Cyclomatic complexity > 15",
            Self::ComplexityExtreme => "Cyclomatic complexity > 25",
            Self::GodClass => "Large class with low cohesion",
            Self::LongMethod => "Method too long (LOC > 100)",
            Self::LongParamList => "Too many parameters (> 5)",
            Self::DeepNesting => "Nesting > 4 levels",
            Self::TodoComment => "TODO/FIXME/HACK comment",
            Self::HighCoupling => "Module coupling too high",
            Self::MissingDocs => "Public API undocumented",
        }
    }

    /// Serialize to snake_case string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ComplexityHigh => "complexity.high",
            Self::ComplexityVeryHigh => "complexity.very_high",
            Self::ComplexityExtreme => "complexity.extreme",
            Self::GodClass => "god_class",
            Self::LongMethod => "long_method",
            Self::LongParamList => "long_param_list",
            Self::DeepNesting => "deep_nesting",
            Self::TodoComment => "todo_comment",
            Self::HighCoupling => "high_coupling",
            Self::MissingDocs => "missing_docs",
        }
    }
}

// =============================================================================
// Structs
// =============================================================================

/// A single issue contributing to debt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtIssue {
    /// File path (relative or absolute, matching input)
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: u32,
    /// Element name (function, class, or None for TODO comments)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    /// Rule ID (e.g., "complexity.high")
    pub rule: String,
    /// Human-readable message
    pub message: String,
    /// Category (lowercase string)
    pub category: String,
    /// Remediation time in minutes
    pub debt_minutes: u32,
}

/// Debt summary for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDebt {
    /// File path
    pub file: PathBuf,
    /// Total debt in minutes
    pub total_minutes: u32,
    /// Number of issues
    pub issue_count: usize,
    /// Issues in this file (not serialized in top_files list)
    #[serde(skip)]
    pub issues: Vec<DebtIssue>,
}

/// Project-wide debt summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtSummary {
    /// Total debt in minutes
    pub total_minutes: u32,
    /// Total debt in hours (rounded to 2 decimals)
    pub total_hours: f64,
    /// Estimated cost if hourly_rate provided (rounded to 2 decimals)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    /// Debt ratio: debt_minutes / LOC (rounded to 3 decimals)
    pub debt_ratio: f64,
    /// Debt density: minutes per KLOC (rounded to 2 decimals)
    pub debt_density: f64,
    /// Debt by category (category_name -> minutes)
    /// Uses BTreeMap for deterministic JSON key ordering (PM-6 mitigation)
    pub by_category: BTreeMap<String, u32>,
    /// Debt by rule (rule_name -> minutes)
    /// Uses BTreeMap for deterministic JSON key ordering (PM-6 mitigation)
    pub by_rule: BTreeMap<String, u32>,
}

/// Complete debt analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtReport {
    /// All issues sorted by debt_minutes descending
    pub issues: Vec<DebtIssue>,
    /// Top files by debt (limited by top_k)
    pub top_files: Vec<FileDebt>,
    /// Summary statistics
    pub summary: DebtSummary,
}

impl DebtReport {
    /// Convert to human-readable text format
    pub fn to_text(&self) -> String {
        let s = &self.summary;
        let mut lines = vec![
            "Technical Debt Report".to_string(),
            "=".repeat(50),
            format!(
                "Total Debt: {:.1} hours ({} minutes)",
                s.total_hours, s.total_minutes
            ),
        ];

        if let Some(cost) = s.total_cost {
            lines.push(format!("Estimated Cost: ${:.2}", cost));
        }

        // Rating interpretation
        let rating = if s.debt_ratio < 0.05 {
            "Excellent"
        } else if s.debt_ratio < 0.10 {
            "Good"
        } else if s.debt_ratio < 0.20 {
            "Concerning"
        } else {
            "Critical"
        };

        lines.push(format!(
            "Debt Ratio: {:.1}% ({})",
            s.debt_ratio * 100.0,
            rating
        ));
        lines.push(format!(
            "Debt Density: {:.1} minutes per KLOC",
            s.debt_density
        ));

        // By category
        if !s.by_category.is_empty() {
            lines.push(String::new());
            lines.push("By Category:".to_string());
            let mut sorted_cats: Vec<_> = s.by_category.iter().collect();
            sorted_cats.sort_by(|a, b| b.1.cmp(a.1));
            for (cat, minutes) in sorted_cats {
                let hours = *minutes as f64 / 60.0;
                let pct = if s.total_minutes > 0 {
                    (*minutes as f64 / s.total_minutes as f64) * 100.0
                } else {
                    0.0
                };
                // Title case the category
                let cat_title: String = cat
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>())
                    .unwrap_or_default()
                    + &cat[1..];
                lines.push(format!("  {}: {:.1}h ({:.0}%)", cat_title, hours, pct));
            }
        }

        // Top debtors (max 10)
        if !self.top_files.is_empty() {
            lines.push(String::new());
            lines.push("Top Debtors:".to_string());
            for (i, f) in self.top_files.iter().take(10).enumerate() {
                let hours = f.total_minutes as f64 / 60.0;
                let fname = f
                    .file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                lines.push(format!(
                    "  {}. {} - {:.1}h ({} issues)",
                    i + 1,
                    fname,
                    hours,
                    f.issue_count
                ));
            }
        }

        // Top issues (max 10)
        if !self.issues.is_empty() {
            lines.push(String::new());
            lines.push("Top Issues:".to_string());
            for issue in self.issues.iter().take(10) {
                let fname = issue
                    .file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let cat = issue.category.to_uppercase();
                let loc = format!("{}:{}", fname, issue.line);
                lines.push(format!(
                    "  [{}] {} - {} ({}m)",
                    cat, loc, issue.message, issue.debt_minutes
                ));
            }
        }

        lines.join("\n")
    }
}

/// Options for debt analysis
#[derive(Debug, Clone)]
pub struct DebtOptions {
    /// File or directory path
    pub path: PathBuf,
    /// Filter by SQALE category
    pub category_filter: Option<String>,
    /// Number of top files to include
    pub top_k: usize,
    /// Hourly rate for cost calculation ($/hour)
    pub hourly_rate: Option<f64>,
    /// Minimum debt (minutes) to report
    pub min_debt: u32,
    /// Language override
    pub language: Option<Language>,
}

impl Default for DebtOptions {
    fn default() -> Self {
        Self {
            path: PathBuf::from("."),
            category_filter: None,
            top_k: 20,
            hourly_rate: None,
            min_debt: 0,
            language: None,
        }
    }
}

// =============================================================================
// Public Functions - STUBS (to be implemented)
// =============================================================================

/// Count non-empty, non-comment lines of code
///
/// # Arguments
/// * `source` - Source code content
/// * `language` - Programming language for comment detection
///
/// # Returns
/// Count of logical lines of code
///
/// # Algorithm
/// - Skip empty lines and comment-only lines
/// - For Python: handle triple-quoted docstrings (both """ and ''')
/// - Lines with code + inline comments still count as code
pub fn count_loc(source: &str, language: Language) -> usize {
    let mut count = 0;
    let mut in_multiline_string = false;
    let mut multiline_quote_style: Option<&str> = None;

    for line in source.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Handle Python multiline strings (docstrings)
        if language == Language::Python {
            // Count triple quotes in this line for both styles
            let double_quote_count = trimmed.matches("\"\"\"").count();
            let single_quote_count = trimmed.matches("'''").count();

            if in_multiline_string {
                // We're inside a multiline string, check if it ends on this line
                let quote_style = multiline_quote_style.unwrap_or("\"\"\"");
                let quote_count = if quote_style == "\"\"\"" {
                    double_quote_count
                } else {
                    single_quote_count
                };

                if quote_count >= 1 {
                    // Found closing quote, exit multiline mode
                    in_multiline_string = false;
                    multiline_quote_style = None;
                }
                // Don't count lines inside docstrings
                continue;
            }

            // Not in multiline mode - check for new docstrings
            // Check if this is a single-line docstring (quote_count >= 2)
            if double_quote_count >= 2 || single_quote_count >= 2 {
                // Single-line docstring like """doc""" or '''doc''' - don't count
                continue;
            } else if double_quote_count == 1 {
                // Start of multiline docstring with """
                in_multiline_string = true;
                multiline_quote_style = Some("\"\"\"");
                continue;
            } else if single_quote_count == 1 {
                // Start of multiline docstring with '''
                in_multiline_string = true;
                multiline_quote_style = Some("'''");
                continue;
            }
        }

        // Skip comment-only lines based on language
        if is_comment_line(trimmed, language) {
            continue;
        }

        count += 1;
    }

    count
}

/// Check if a line is a comment-only line
fn is_comment_line(trimmed: &str, language: Language) -> bool {
    match language {
        // Python uses # for comments
        Language::Python => trimmed.starts_with('#'),
        // Ruby uses # for comments
        Language::Ruby => trimmed.starts_with('#'),
        // Elixir uses # for comments
        Language::Elixir => trimmed.starts_with('#'),
        // Lua/Luau use -- for comments
        Language::Lua | Language::Luau => trimmed.starts_with("--"),
        // OCaml uses (* ... *) for comments
        Language::Ocaml => {
            trimmed.starts_with("(*") || trimmed.starts_with("*)") || trimmed.starts_with('*')
        }
        // PHP uses // or # or /* for comments
        Language::Php => {
            trimmed.starts_with("//")
                || trimmed.starts_with('#')
                || trimmed.starts_with("/*")
                || trimmed.starts_with("*/")
                || trimmed.starts_with("*")
        }
        // C-style comment languages (// and /* */)
        Language::Rust
        | Language::Go
        | Language::TypeScript
        | Language::JavaScript
        | Language::Java
        | Language::C
        | Language::Cpp
        | Language::Kotlin
        | Language::Swift
        | Language::CSharp
        | Language::Scala => {
            trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with("*/")
                || trimmed.starts_with("*")
        }
    }
}

/// Returns tree-sitter node kinds that represent comments for each language.
///
/// These are the AST node types emitted by each language's tree-sitter grammar.
/// Every language variant is explicitly listed -- no wildcard fallback.
fn comment_node_kinds(language: Language) -> &'static [&'static str] {
    match language {
        Language::Python => &["comment"],
        Language::TypeScript | Language::JavaScript => &["comment"],
        Language::Go => &["comment"],
        Language::Rust => &["line_comment", "block_comment"],
        Language::Java => &["line_comment", "block_comment"],
        Language::C | Language::Cpp => &["comment"],
        Language::Ruby => &["comment"],
        Language::Kotlin => &["line_comment", "multiline_comment"],
        Language::Swift => &["comment", "multiline_comment"],
        Language::CSharp => &["comment"],
        Language::Scala => &["comment"],
        Language::Php => &["comment"],
        Language::Lua | Language::Luau => &["comment"],
        Language::Elixir => &["comment"],
        Language::Ocaml => &["comment"],
    }
}

/// Strip comment prefix characters to extract the inner text.
///
/// Handles all comment syntax across 18 languages:
/// - `#` for Python, Ruby, Elixir, PHP
/// - `//` for JS/TS, Go, Java, Rust, C/C++, Kotlin, Swift, C#, Scala
/// - `/* ... */` for block comments in C-family languages
/// - `--` for Lua/Luau
/// - `(* ... *)` for OCaml
fn strip_comment_prefix(text: &str) -> &str {
    let trimmed = text.trim();
    // Try multi-char prefixes first (order matters)
    if let Some(rest) = trimmed.strip_prefix("//") {
        return rest.trim_start();
    }
    if let Some(rest) = trimmed.strip_prefix("/*") {
        // Remove trailing */
        let rest = rest.strip_suffix("*/").unwrap_or(rest);
        return rest.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("(*") {
        // OCaml block comment
        let rest = rest.strip_suffix("*)").unwrap_or(rest);
        return rest.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("--") {
        // Lua/Luau line comment
        return rest.trim_start();
    }
    if let Some(rest) = trimmed.strip_prefix('#') {
        return rest.trim_start();
    }
    trimmed
}

/// Find TODO, FIXME, HACK, XXX comments in source code using AST comment nodes.
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language for tree-sitter parsing
///
/// # Returns
/// Vector of DebtIssue for each TODO-style comment found
///
/// # Algorithm
/// 1. Parse source with tree-sitter to get AST
/// 2. Walk all nodes, collecting those whose kind matches comment_node_kinds(language)
/// 3. For each comment node, strip the comment prefix and check for TODO/FIXME/HACK/XXX
/// 4. This ensures string literals containing "TODO" are NOT false-positively detected
/// 5. Falls back to regex for languages without tree-sitter support (e.g., Swift)
///
/// # Invariant
/// - Each TODO = 10 minutes debt
/// - Content truncated to 50 characters
/// - Case-insensitive tag matching
/// - Word boundary prevents matching TODONE as TODO
pub fn find_todo_comments(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_todo_comments_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree to avoid redundant parsing.
fn find_todo_comments_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    if let Some(tree) = pre_parsed {
        find_todo_comments_ast(source, filepath, language, tree)
    } else {
        match parse(source, language) {
            Ok(tree) => find_todo_comments_ast(source, filepath, language, &tree),
            Err(_) => find_todo_comments_regex(source, filepath, language),
        }
    }
}

/// AST-based TODO comment detection: walks tree-sitter nodes to find comment nodes
fn find_todo_comments_ast(
    source: &str,
    filepath: &Path,
    language: Language,
    tree: &tree_sitter::Tree,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();
    let comment_kinds = comment_node_kinds(language);
    // Anchor at start: the TODO tag must appear at the beginning of the comment content
    // (after stripping the comment prefix). This prevents matching "todo" inside
    // arbitrary comment text like "# TODONE: not a todo".
    let tag_pattern = Regex::new(r"(?i)^(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)").unwrap();

    // Walk the entire tree using a stack-based DFS
    let mut visit_stack = vec![tree.root_node()];

    while let Some(node) = visit_stack.pop() {
        if comment_kinds.contains(&node.kind()) {
            // Extract the comment text from source
            let start = node.start_byte();
            let end = node.end_byte();
            if start < source.len() && end <= source.len() {
                let comment_text = &source[start..end];

                // For multi-line block comments, check each line
                for (offset, line) in comment_text.lines().enumerate() {
                    let stripped = strip_comment_prefix(line);
                    // Also strip leading * in block comment continuation lines (e.g., " * TODO: ...")
                    let stripped = stripped
                        .strip_prefix('*')
                        .map(|s| s.trim_start())
                        .unwrap_or(stripped);

                    if let Some(captures) = tag_pattern.captures(stripped) {
                        let tag = captures
                            .get(1)
                            .map(|m| m.as_str().to_uppercase())
                            .unwrap_or_default();
                        let content = captures
                            .get(2)
                            .map(|m| m.as_str().trim())
                            .unwrap_or("")
                            .chars()
                            .take(50)
                            .collect::<String>();

                        let message = if content.is_empty() {
                            tag.clone()
                        } else {
                            format!("{}: {}", tag, content)
                        };

                        // Line number: node's start row + offset within multi-line comment
                        let line_num = node.start_position().row + offset;

                        issues.push(DebtIssue {
                            file: filepath.to_path_buf(),
                            line: (line_num + 1) as u32, // 1-indexed
                            element: None,
                            rule: "todo_comment".to_string(),
                            message,
                            category: "reliability".to_string(),
                            debt_minutes: 10,
                        });
                    }
                }
            }
        }

        // Push children in reverse order so we process left-to-right
        let child_count = node.child_count();
        for i in (0..child_count).rev() {
            if let Some(child) = node.child(i) {
                visit_stack.push(child);
            }
        }
    }

    // Sort by line number to ensure consistent ordering
    issues.sort_by_key(|i| i.line);
    issues
}

/// Regex fallback for languages without tree-sitter support (e.g., Swift).
///
/// Uses language-aware comment prefix patterns instead of Python-only `#`.
fn find_todo_comments_regex(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Build regex pattern based on language comment syntax
    let pattern_str = match language {
        Language::Python | Language::Ruby | Language::Elixir => {
            r"(?i)#\s*(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)$"
        }
        Language::Lua | Language::Luau => r"(?i)--\s*(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)$",
        Language::Ocaml => r"(?i)\(\*\s*(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)$",
        Language::Php => {
            // PHP supports both # and //
            r"(?i)(?://|#)\s*(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)$"
        }
        _ => {
            // C-family: //, /* */
            r"(?i)//\s*(TODO|FIXME|HACK|XXX)\b[\s:]*(.*)$"
        }
    };

    let pattern = Regex::new(pattern_str).unwrap();

    for (line_num, line) in source.lines().enumerate() {
        if let Some(captures) = pattern.captures(line) {
            let tag = captures
                .get(1)
                .map(|m| m.as_str().to_uppercase())
                .unwrap_or_default();
            let content = captures
                .get(2)
                .map(|m| m.as_str().trim())
                .unwrap_or("")
                .chars()
                .take(50)
                .collect::<String>();

            let message = if content.is_empty() {
                tag.clone()
            } else {
                format!("{}: {}", tag, content)
            };

            issues.push(DebtIssue {
                file: filepath.to_path_buf(),
                line: (line_num + 1) as u32, // 1-indexed
                element: None,
                rule: "todo_comment".to_string(),
                message,
                category: "reliability".to_string(),
                debt_minutes: 10,
            });
        }
    }

    issues
}

/// Find functions with high cyclomatic complexity and related issues
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language
///
/// # Returns
/// Vector of DebtIssue for complexity violations
///
/// # Detected Issues
/// - `complexity.high`: CC > 10 (20 minutes)
/// - `complexity.very_high`: CC > 15 (30 minutes)
/// - `complexity.extreme`: CC > 25 (60 minutes)
/// - `long_method`: LOC > 100 (30 minutes)
/// - `long_param_list`: params > 5 excluding self/cls (15 minutes)
///
/// # Invariant
/// Only the HIGHEST complexity threshold is reported per function.
/// A CC=30 function gets only `complexity.extreme`, not also `high` and `very_high`.
pub fn find_complexity_issues(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_complexity_issues_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree to avoid redundant parsing.
fn find_complexity_issues_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Use pre-parsed tree if available, otherwise parse (graceful degradation on error)
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = match parse(source, language) {
                Ok(t) => t,
                Err(_) => return issues,
            };
            &owned_tree
        }
    };

    let root = tree.root_node();

    // Find all functions and collect their info
    let function_infos = extract_function_infos_for_debt(root, source, language);

    // Batch calculate complexity using the already-parsed tree (zero extra parses)
    let complexity_map =
        calculate_all_complexities_from_tree(root, source, language).unwrap_or_default();

    for func_info in function_infos {
        let file = filepath.to_path_buf();

        // Cyclomatic complexity - only report HIGHEST threshold (PM-3)
        // Use pre-computed batch metrics instead of per-function parse
        if let Some(metrics) = complexity_map.get(&func_info.name) {
            let cc = metrics.cyclomatic;

            // Only report the highest applicable threshold
            let complexity_issue = if cc > 25 {
                Some((
                    "complexity.extreme",
                    60,
                    format!("Cyclomatic complexity {} exceeds threshold", cc),
                ))
            } else if cc > 15 {
                Some((
                    "complexity.very_high",
                    30,
                    format!("Cyclomatic complexity {} exceeds threshold", cc),
                ))
            } else if cc > 10 {
                Some((
                    "complexity.high",
                    20,
                    format!("Cyclomatic complexity {} exceeds threshold", cc),
                ))
            } else {
                None
            };

            if let Some((rule, minutes, message)) = complexity_issue {
                issues.push(DebtIssue {
                    file: file.clone(),
                    line: func_info.start_line,
                    element: Some(func_info.full_name.clone()),
                    rule: rule.to_string(),
                    message,
                    category: "maintainability".to_string(),
                    debt_minutes: minutes,
                });
            }
        }
        // PM-3: On error, silently skip complexity check (don't add issue, don't default CC=1)

        // Long method check (LOC > 100)
        let func_loc = func_info.end_line.saturating_sub(func_info.start_line);
        if func_loc > 100 {
            issues.push(DebtIssue {
                file: file.clone(),
                line: func_info.start_line,
                element: Some(func_info.full_name.clone()),
                rule: "long_method".to_string(),
                message: format!("Method has {} lines (> 100)", func_loc),
                category: "maintainability".to_string(),
                debt_minutes: 30,
            });
        }

        // Long parameter list (> 5 params, excluding self/cls)
        let param_count = count_params_excluding_self(&func_info.params);
        if param_count > 5 {
            issues.push(DebtIssue {
                file: file.clone(),
                line: func_info.start_line,
                element: Some(func_info.full_name.clone()),
                rule: "long_param_list".to_string(),
                message: format!("Function has {} parameters (> 5)", param_count),
                category: "testability".to_string(),
                debt_minutes: 15,
            });
        }
    }

    issues
}

/// Information about a function extracted for debt analysis
#[derive(Debug)]
struct FunctionInfoForDebt {
    /// Function name (without class prefix)
    name: String,
    /// Full name including class prefix (e.g., "ClassName.method_name")
    full_name: String,
    /// Start line (1-indexed)
    start_line: u32,
    /// End line (1-indexed)
    end_line: u32,
    /// Parameter names
    params: Vec<String>,
}

/// Extract function information for debt analysis from AST
fn extract_function_infos_for_debt(
    root: Node,
    source: &str,
    language: Language,
) -> Vec<FunctionInfoForDebt> {
    let mut functions = Vec::new();

    match language {
        Language::Python => extract_python_functions_for_debt(root, source, &mut functions, None),
        Language::TypeScript | Language::JavaScript => {
            extract_ts_functions_for_debt(root, source, &mut functions, None)
        }
        Language::Go => extract_go_functions_for_debt(root, source, &mut functions),
        Language::Rust => extract_rust_functions_for_debt(root, source, &mut functions, None),
        Language::Java => extract_java_functions_for_debt(root, source, &mut functions, None),
        _ => {} // Unsupported language - return empty
    }

    functions
}

/// Extract Python functions for debt analysis
fn extract_python_functions_for_debt(
    node: Node,
    source: &str,
    functions: &mut Vec<FunctionInfoForDebt>,
    class_name: Option<&str>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(info) =
                    extract_python_function_info_for_debt(&child, source, class_name)
                {
                    functions.push(info);
                }
            }
            "decorated_definition" => {
                // Handle decorated functions
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(info) =
                            extract_python_function_info_for_debt(&def, source, class_name)
                        {
                            functions.push(info);
                        }
                    }
                }
            }
            "class_definition" => {
                // Extract class name and recurse into methods
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = get_node_text(&name_node, source);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_python_functions_for_debt(body, source, functions, Some(&cls_name));
                    }
                }
            }
            _ => {
                // Recurse into other nodes (but not class bodies, handled above)
                if class_name.is_none() {
                    extract_python_functions_for_debt(child, source, functions, None);
                }
            }
        }
    }
}

/// Extract a single Python function's info for debt analysis
fn extract_python_function_info_for_debt(
    node: &Node,
    source: &str,
    class_name: Option<&str>,
) -> Option<FunctionInfoForDebt> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(&name_node, source);

    let full_name = if let Some(cls) = class_name {
        format!("{}.{}", cls, name)
    } else {
        name.clone()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract parameters
    let params = extract_python_params_for_debt(node, source);

    Some(FunctionInfoForDebt {
        name,
        full_name,
        start_line,
        end_line,
        params,
    })
}

/// Extract Python function parameters
fn extract_python_params_for_debt(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(get_node_text(&child, source));
                }
                "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                    // The identifier is the first child
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "identifier" {
                            params.push(get_node_text(&inner_child, source));
                            break;
                        }
                    }
                }
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    // *args, **kwargs - get the name without the * or **
                    if let Some(name_child) = child.child(0) {
                        if name_child.kind() == "identifier" {
                            params.push(get_node_text(&name_child, source));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract TypeScript/JavaScript functions for debt analysis
fn extract_ts_functions_for_debt(
    node: Node,
    source: &str,
    functions: &mut Vec<FunctionInfoForDebt>,
    class_name: Option<&str>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "function" => {
                if let Some(info) = extract_ts_function_info_for_debt(&child, source, class_name) {
                    functions.push(info);
                }
            }
            "method_definition" => {
                if let Some(info) = extract_ts_function_info_for_debt(&child, source, class_name) {
                    functions.push(info);
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = get_node_text(&name_node, source);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_ts_functions_for_debt(body, source, functions, Some(&cls_name));
                    }
                }
            }
            _ => {
                if class_name.is_none() {
                    extract_ts_functions_for_debt(child, source, functions, None);
                }
            }
        }
    }
}

/// Extract a single TypeScript function's info for debt analysis
fn extract_ts_function_info_for_debt(
    node: &Node,
    source: &str,
    class_name: Option<&str>,
) -> Option<FunctionInfoForDebt> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let full_name = if let Some(cls) = class_name {
        format!("{}.{}", cls, name)
    } else {
        name.clone()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract parameters
    let params = extract_ts_params_for_debt(node, source);

    Some(FunctionInfoForDebt {
        name,
        full_name,
        start_line,
        end_line,
        params,
    })
}

/// Extract TypeScript function parameters
fn extract_ts_params_for_debt(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "identifier" => {
                    params.push(get_node_text(&child, source));
                }
                "required_parameter" | "optional_parameter" => {
                    if let Some(pattern) = child.child_by_field_name("pattern") {
                        params.push(get_node_text(&pattern, source));
                    }
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract Go functions for debt analysis
fn extract_go_functions_for_debt(
    node: Node,
    source: &str,
    functions: &mut Vec<FunctionInfoForDebt>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "method_declaration" => {
                if let Some(info) = extract_go_function_info_for_debt(&child, source) {
                    functions.push(info);
                }
            }
            _ => {
                extract_go_functions_for_debt(child, source, functions);
            }
        }
    }
}

/// Extract a single Go function's info for debt analysis
fn extract_go_function_info_for_debt(node: &Node, source: &str) -> Option<FunctionInfoForDebt> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    // For methods, get receiver type
    let full_name = if let Some(receiver) = node.child_by_field_name("receiver") {
        // Try to extract type name from receiver
        if let Some(type_name) = extract_go_receiver_type(&receiver, source) {
            format!("{}.{}", type_name, name)
        } else {
            name.clone()
        }
    } else {
        name.clone()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract parameters
    let params = extract_go_params_for_debt(node, source);

    Some(FunctionInfoForDebt {
        name,
        full_name,
        start_line,
        end_line,
        params,
    })
}

/// Extract Go receiver type name
fn extract_go_receiver_type(receiver: &Node, source: &str) -> Option<String> {
    let mut cursor = receiver.walk();
    for child in receiver.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            // Look for type_identifier or pointer_type
            if let Some(type_node) = child.child_by_field_name("type") {
                return Some(
                    get_node_text(&type_node, source)
                        .trim_start_matches('*')
                        .to_string(),
                );
            }
        }
    }
    None
}

/// Extract Go function parameters
fn extract_go_params_for_debt(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter_declaration" {
                // Get all identifiers in this parameter declaration
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "identifier" {
                        params.push(get_node_text(&inner_child, source));
                    }
                }
            }
        }
    }

    params
}

/// Extract Rust functions for debt analysis
fn extract_rust_functions_for_debt(
    node: Node,
    source: &str,
    functions: &mut Vec<FunctionInfoForDebt>,
    impl_name: Option<&str>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(info) = extract_rust_function_info_for_debt(&child, source, impl_name) {
                    functions.push(info);
                }
            }
            "impl_item" => {
                // Extract type name from impl
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_name = get_node_text(&type_node, source);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_rust_functions_for_debt(body, source, functions, Some(&type_name));
                    }
                }
            }
            _ => {
                if impl_name.is_none() {
                    extract_rust_functions_for_debt(child, source, functions, None);
                }
            }
        }
    }
}

/// Extract a single Rust function's info for debt analysis
fn extract_rust_function_info_for_debt(
    node: &Node,
    source: &str,
    impl_name: Option<&str>,
) -> Option<FunctionInfoForDebt> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let full_name = if let Some(impl_type) = impl_name {
        format!("{}.{}", impl_type, name)
    } else {
        name.clone()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract parameters
    let params = extract_rust_params_for_debt(node, source);

    Some(FunctionInfoForDebt {
        name,
        full_name,
        start_line,
        end_line,
        params,
    })
}

/// Extract Rust function parameters
fn extract_rust_params_for_debt(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            match child.kind() {
                "parameter" => {
                    if let Some(pattern) = child.child_by_field_name("pattern") {
                        params.push(get_node_text(&pattern, source));
                    }
                }
                "self_parameter" => {
                    params.push("self".to_string());
                }
                _ => {}
            }
        }
    }

    params
}

/// Extract Java functions for debt analysis
fn extract_java_functions_for_debt(
    node: Node,
    source: &str,
    functions: &mut Vec<FunctionInfoForDebt>,
    class_name: Option<&str>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" | "constructor_declaration" => {
                if let Some(info) = extract_java_function_info_for_debt(&child, source, class_name)
                {
                    functions.push(info);
                }
            }
            "class_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let cls_name = get_node_text(&name_node, source);
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_java_functions_for_debt(body, source, functions, Some(&cls_name));
                    }
                }
            }
            _ => {
                if class_name.is_none() {
                    extract_java_functions_for_debt(child, source, functions, None);
                }
            }
        }
    }
}

/// Extract a single Java function's info for debt analysis
fn extract_java_function_info_for_debt(
    node: &Node,
    source: &str,
    class_name: Option<&str>,
) -> Option<FunctionInfoForDebt> {
    let name = node
        .child_by_field_name("name")
        .map(|n| get_node_text(&n, source))?;

    let full_name = if let Some(cls) = class_name {
        format!("{}.{}", cls, name)
    } else {
        name.clone()
    };

    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;

    // Extract parameters
    let params = extract_java_params_for_debt(node, source);

    Some(FunctionInfoForDebt {
        name,
        full_name,
        start_line,
        end_line,
        params,
    })
}

/// Extract Java function parameters
fn extract_java_params_for_debt(node: &Node, source: &str) -> Vec<String> {
    let mut params = Vec::new();

    if let Some(params_node) = node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "formal_parameter" || child.kind() == "spread_parameter" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    params.push(get_node_text(&name_node, source));
                }
            }
        }
    }

    params
}

/// Count parameters excluding self/cls (Python convention)
fn count_params_excluding_self(params: &[String]) -> usize {
    params
        .iter()
        .filter(|p| *p != "self" && *p != "cls")
        .count()
}

/// Get text content of a tree-sitter node
fn get_node_text(node: &Node, source: &str) -> String {
    node.utf8_text(source.as_bytes()).unwrap_or("").to_string()
}

/// Find God Class issues using LCOM4 cohesion analysis
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language
///
/// # Returns
/// Vector of DebtIssue for god class violations
///
/// # Algorithm
/// - Extract all classes from source
/// - For each class with > 20 non-dunder methods:
///   - Compute LCOM4 using Union-Find
///   - If LCOM4 > 0.8, flag as God Class
pub fn find_god_classes(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_god_classes_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree.
fn find_god_classes_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Use pre-parsed tree if available, otherwise parse
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = match parse(source, language) {
                Ok(t) => t,
                Err(_) => return issues,
            };
            &owned_tree
        }
    };

    let root = tree.root_node();

    // Extract all classes with their methods
    let classes = extract_classes_for_lcom4(root, source, language);

    for class_info in classes {
        // Count non-dunder methods
        let non_dunder_methods: Vec<_> = class_info
            .methods
            .iter()
            .filter(|m| !is_dunder_method(&m.name))
            .collect();

        let method_count = non_dunder_methods.len();

        // God class threshold: >20 methods AND LCOM4 > 0.8
        // (need >= 2 methods to meaningfully calculate LCOM4)
        if method_count > 20 {
            let lcom4 = compute_lcom4_for_class(&non_dunder_methods, source);

            // Flag as God Class if LCOM4 > 0.8
            if lcom4 > 0.8 {
                issues.push(DebtIssue {
                    file: filepath.to_path_buf(),
                    line: class_info.start_line,
                    element: Some(class_info.name.clone()),
                    rule: "god_class".to_string(),
                    message: format!("God class: {} methods, LCOM4={:.2}", method_count, lcom4),
                    category: "changeability".to_string(),
                    debt_minutes: 60,
                });
            }
        }
    }

    issues
}

/// Find functions with deep nesting (> 4 levels)
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language
///
/// # Returns
/// Vector of DebtIssue for deep nesting violations
///
/// # Algorithm
/// - Parse source code
/// - Extract all functions
/// - For each function, calculate max nesting depth
/// - Flag functions where depth > 4
pub fn find_deep_nesting(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_deep_nesting_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree.
fn find_deep_nesting_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Use pre-parsed tree if available, otherwise parse
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = match parse(source, language) {
                Ok(t) => t,
                Err(_) => return issues,
            };
            &owned_tree
        }
    };

    let root = tree.root_node();

    // Extract all functions with their info
    let function_infos = extract_function_infos_for_debt(root, source, language);

    for func_info in function_infos {
        // Get the function node to calculate nesting depth
        let max_depth = calculate_function_nesting_depth(source, &func_info, language);

        // Threshold: > 4 levels = 15 minutes debt
        if max_depth > 4 {
            issues.push(DebtIssue {
                file: filepath.to_path_buf(),
                line: func_info.start_line,
                element: Some(func_info.full_name.clone()),
                rule: "deep_nesting".to_string(),
                message: format!("Deep nesting: {} levels", max_depth),
                category: "maintainability".to_string(),
                debt_minutes: 15,
            });
        }
    }

    issues
}

/// Calculate the maximum nesting depth within a function
fn calculate_function_nesting_depth(
    source: &str,
    func_info: &FunctionInfoForDebt,
    language: Language,
) -> usize {
    // Re-parse to get the tree for walking
    let tree = match parse(source, language) {
        Ok(t) => t,
        Err(_) => return 0,
    };

    let root = tree.root_node();

    // Find the function node by line number
    if let Some(func_node) = find_function_node_by_line(&root, func_info.start_line, language) {
        // Get nesting node kinds for this language
        let nesting_kinds = get_nesting_node_kinds(language);

        // Calculate max depth within the function body
        if let Some(body) = get_function_body(&func_node, language) {
            return walk_nesting_depth(&body, &nesting_kinds, 0);
        }
    }

    0
}

/// Get node kinds that count as nesting levels for a language
fn get_nesting_node_kinds(language: Language) -> Vec<&'static str> {
    match language {
        Language::Python => vec![
            "if_statement",
            "for_statement",
            "while_statement",
            "try_statement",
            "with_statement",
            "match_statement",
        ],
        Language::TypeScript | Language::JavaScript => vec![
            "if_statement",
            "for_statement",
            "while_statement",
            "for_in_statement",
            "try_statement",
            "switch_statement",
        ],
        Language::Go => vec![
            "if_statement",
            "for_statement",
            "switch_statement",
            "select_statement",
        ],
        Language::Rust => vec![
            "if_expression",
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_expression",
        ],
        Language::Java => vec![
            "if_statement",
            "for_statement",
            "while_statement",
            "try_statement",
            "switch_expression",
        ],
        _ => vec!["if_statement", "for_statement", "while_statement"],
    }
}

/// Find a function node by its start line
fn find_function_node_by_line<'a>(
    node: &Node<'a>,
    target_line: u32,
    language: Language,
) -> Option<Node<'a>> {
    let func_kinds = get_function_node_kinds(language);
    let node_line = node.start_position().row as u32 + 1;

    if func_kinds.contains(&node.kind()) && node_line == target_line {
        return Some(*node);
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_function_node_by_line(&child, target_line, language) {
            return Some(found);
        }
    }

    None
}

/// Get function node kinds for a language (delegates to shared function_finder)
fn get_function_node_kinds(language: Language) -> Vec<&'static str> {
    get_function_node_kinds_vec(language)
}

/// Get the body of a function node (delegates to shared function_finder)
fn get_function_body<'a>(node: &Node<'a>, language: Language) -> Option<Node<'a>> {
    shared_get_function_body(*node, language)
}

/// Recursively walk the AST and calculate max nesting depth
fn walk_nesting_depth(node: &Node, nesting_kinds: &[&str], current_depth: usize) -> usize {
    let kind = node.kind();
    let new_depth = if nesting_kinds.contains(&kind) {
        current_depth + 1
    } else {
        current_depth
    };

    let mut max_depth = new_depth;

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_max = walk_nesting_depth(&child, nesting_kinds, new_depth);
        max_depth = max_depth.max(child_max);
    }

    max_depth
}

/// Find modules with high coupling (> 15 unique imports)
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language
///
/// # Returns
/// Vector of DebtIssue for high coupling violations
///
/// # Algorithm (PM-2 Mitigation)
/// - Parse source code to get AST
/// - Extract imports using extract_imports_from_tree
/// - Count unique module names
/// - Flag if > 15 unique imports
pub fn find_high_coupling(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_high_coupling_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree.
fn find_high_coupling_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Use pre-parsed tree if available, otherwise parse
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = match parse(source, language) {
                Ok(t) => t,
                Err(_) => return issues,
            };
            &owned_tree
        }
    };

    // Extract imports using existing import extraction (PM-2 mitigation)
    let imports = match crate::ast::imports::extract_imports_from_tree(tree, source, language) {
        Ok(imports) => imports,
        Err(_) => return issues, // Graceful degradation for unsupported languages
    };

    // Count unique module names
    let unique_modules: HashSet<String> = imports
        .iter()
        .map(|i| {
            // For "from X import Y", X is the module
            // For "import X.Y.Z", take the root module
            let module = &i.module;
            // Get the root module (first part before any dot)
            module.split('.').next().unwrap_or(module).to_string()
        })
        .collect();

    // Threshold: > 15 unique imports = 20 minutes debt
    if unique_modules.len() > 15 {
        issues.push(DebtIssue {
            file: filepath.to_path_buf(),
            line: 1, // Module-level issue
            element: None,
            rule: "high_coupling".to_string(),
            message: format!("File imports {} modules (> 15)", unique_modules.len()),
            category: "changeability".to_string(),
            debt_minutes: 20,
        });
    }

    issues
}

/// Find public APIs missing documentation
///
/// # Arguments
/// * `source` - Source code content
/// * `filepath` - File path for reporting
/// * `language` - Programming language
///
/// # Returns
/// Vector of DebtIssue for missing documentation
///
/// # Algorithm
/// - Parse source code
/// - Extract public functions and classes
/// - For Python: check for docstring immediately after def/class
/// - Exclude private functions (_prefix) and dunder methods (__name__)
/// - Each undocumented public API = 10 minutes debt
pub fn find_missing_docs(source: &str, filepath: &Path, language: Language) -> Vec<DebtIssue> {
    find_missing_docs_inner(source, filepath, language, None)
}

/// Inner implementation that accepts an optional pre-parsed tree.
fn find_missing_docs_inner(
    source: &str,
    filepath: &Path,
    language: Language,
    pre_parsed: Option<&tree_sitter::Tree>,
) -> Vec<DebtIssue> {
    let mut issues = Vec::new();

    // Use pre-parsed tree if available, otherwise parse
    let owned_tree;
    let tree = match pre_parsed {
        Some(t) => t,
        None => {
            owned_tree = match parse(source, language) {
                Ok(t) => t,
                Err(_) => return issues,
            };
            &owned_tree
        }
    };

    let root = tree.root_node();

    // Currently only Python is fully supported for docstring detection
    if language == Language::Python {
        find_python_missing_docs(&root, source, filepath, &mut issues, None);
    }

    issues
}

/// Find Python functions and classes missing documentation
fn find_python_missing_docs(
    node: &Node,
    source: &str,
    filepath: &Path,
    issues: &mut Vec<DebtIssue>,
    class_name: Option<&str>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                check_python_function_docs(&child, source, filepath, issues, class_name);
            }
            "decorated_definition" => {
                // Handle decorated functions
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        check_python_function_docs(&def, source, filepath, issues, class_name);
                    } else if def.kind() == "class_definition" {
                        check_python_class_docs(&def, source, filepath, issues);
                    }
                }
            }
            "class_definition" => {
                check_python_class_docs(&child, source, filepath, issues);
            }
            _ => {
                // Recurse into other nodes (but not class bodies, handled separately)
                if class_name.is_none() {
                    find_python_missing_docs(&child, source, filepath, issues, None);
                }
            }
        }
    }
}

/// Check if a Python function has documentation
fn check_python_function_docs(
    node: &Node,
    source: &str,
    filepath: &Path,
    issues: &mut Vec<DebtIssue>,
    class_name: Option<&str>,
) {
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = get_node_text(&name_node, source);

        // Skip private functions (single underscore prefix)
        // But don't skip dunder methods here - they're excluded separately
        if name.starts_with('_') && !name.starts_with("__") {
            return;
        }

        // Skip dunder methods (__init__, __str__, etc.)
        if is_dunder_method(&name) {
            return;
        }

        // Check for docstring in function body
        let has_docstring = if let Some(body) = node.child_by_field_name("body") {
            has_python_docstring(&body, source)
        } else {
            false
        };

        if !has_docstring {
            let full_name = if let Some(cls) = class_name {
                format!("{}.{}", cls, name)
            } else {
                name.clone()
            };

            let element_type = if class_name.is_some() {
                "method"
            } else {
                "function"
            };

            issues.push(DebtIssue {
                file: filepath.to_path_buf(),
                line: node.start_position().row as u32 + 1,
                element: Some(full_name),
                rule: "missing_docs".to_string(),
                message: format!("Public {} '{}' lacks documentation", element_type, name),
                category: "maintainability".to_string(),
                debt_minutes: 10,
            });
        }
    }
}

/// Check if a Python class has documentation and recurse into methods
fn check_python_class_docs(
    node: &Node,
    source: &str,
    filepath: &Path,
    issues: &mut Vec<DebtIssue>,
) {
    if let Some(name_node) = node.child_by_field_name("name") {
        let name = get_node_text(&name_node, source);

        // Skip private classes (single underscore prefix)
        if name.starts_with('_') && !name.starts_with("__") {
            return;
        }

        // Check for docstring in class body
        let has_docstring = if let Some(body) = node.child_by_field_name("body") {
            has_python_docstring(&body, source)
        } else {
            false
        };

        if !has_docstring {
            issues.push(DebtIssue {
                file: filepath.to_path_buf(),
                line: node.start_position().row as u32 + 1,
                element: Some(name.clone()),
                rule: "missing_docs".to_string(),
                message: format!("Public class '{}' lacks documentation", name),
                category: "maintainability".to_string(),
                debt_minutes: 10,
            });
        }

        // Check methods within this class
        if let Some(body) = node.child_by_field_name("body") {
            find_python_missing_docs(&body, source, filepath, issues, Some(&name));
        }
    }
}

/// Check if a Python block has a docstring as its first statement
fn has_python_docstring(body: &Node, source: &str) -> bool {
    // In Python, a docstring is an expression_statement containing a string
    // as the first statement in a function/class body
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        // Skip comments and pass statements
        match child.kind() {
            "comment" | "pass_statement" => continue,
            "expression_statement" => {
                // Check if this expression statement contains a string literal
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "string" {
                        let text = get_node_text(&inner, source);
                        // Docstrings use triple quotes
                        if text.starts_with("\"\"\"")
                            || text.starts_with("'''")
                            || text.starts_with("r\"\"\"")
                            || text.starts_with("r'''")
                        {
                            return true;
                        }
                    }
                }
                // If we found an expression statement that's not a docstring, no docstring
                return false;
            }
            _ => {
                // Any other statement type means no docstring
                return false;
            }
        }
    }

    false
}

/// Compute LCOM4 metric for a class
///
/// LCOM4 counts connected components in the method-field graph.
/// Returns normalized value: (components - 1) / max(methods - 1, 1)
///
/// - 0.0 = perfectly cohesive (all methods share fields)
/// - 1.0 = maximally incohesive (no methods share fields)
///
/// Note: This is the public API for testing. The actual implementation
/// for god class detection uses compute_lcom4_for_class internally.
pub fn compute_lcom4() -> f64 {
    // This is a placeholder for tests - actual LCOM4 calculation
    // is done via compute_lcom4_for_class with method data
    0.0
}

// =============================================================================
// LCOM4 Helper Types and Functions
// =============================================================================

/// Information about a class extracted for LCOM4 analysis
#[derive(Debug)]
struct ClassInfoForLcom4 {
    /// Class name
    name: String,
    /// Start line (1-indexed)
    start_line: u32,
    /// Methods in this class
    methods: Vec<MethodInfoForLcom4>,
}

/// Information about a method extracted for LCOM4 analysis
#[derive(Debug)]
struct MethodInfoForLcom4 {
    /// Method name
    name: String,
    /// Start byte offset in source
    start_byte: usize,
    /// End byte offset in source
    end_byte: usize,
}

/// Union-Find data structure for LCOM4 connected component calculation
/// Uses iterative path compression to avoid stack overflow
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
        (0..n).map(|i| self.find(i)).collect::<HashSet<_>>().len()
    }
}

/// Check if a method name is a dunder method (__name__)
fn is_dunder_method(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__")
}

/// Compute LCOM4 for a set of methods
///
/// # Arguments
/// * `methods` - Non-dunder methods to analyze
/// * `source` - Full source code
///
/// # Returns
/// LCOM4 value between 0.0 and 1.0
fn compute_lcom4_for_class(methods: &[&MethodInfoForLcom4], source: &str) -> f64 {
    let n = methods.len();

    // With < 2 methods, LCOM4 is 0.0 by definition (perfectly cohesive)
    if n < 2 {
        return 0.0;
    }

    // Extract field accesses for each method
    let method_fields: Vec<HashSet<String>> = methods
        .iter()
        .map(|m| {
            let method_source = &source[m.start_byte..m.end_byte];
            extract_self_accesses(method_source)
        })
        .collect();

    // Check if any method accesses any fields
    let all_fields: HashSet<String> = method_fields.iter().flatten().cloned().collect();
    if all_fields.is_empty() {
        // No shared state - each method is its own component
        // LCOM4 = 1.0 (maximally incohesive)
        return 1.0;
    }

    // Build Union-Find and connect methods that share fields
    let mut uf = UnionFind::new(n);

    for i in 0..n {
        for j in (i + 1)..n {
            // Check if methods i and j share any fields
            if !method_fields[i].is_disjoint(&method_fields[j]) {
                uf.union(i, j);
            }
        }
    }

    // Count connected components
    let components = uf.count_components();

    // Normalize: (components - 1) / max(methods - 1, 1)
    let denominator = (n - 1).max(1) as f64;
    (components as f64 - 1.0) / denominator
}

/// Extract classes with their methods for LCOM4 analysis
fn extract_classes_for_lcom4(
    root: Node,
    source: &str,
    language: Language,
) -> Vec<ClassInfoForLcom4> {
    let mut classes = Vec::new();

    if language == Language::Python {
        extract_python_classes_for_lcom4(root, source, &mut classes);
    }

    classes
}

/// Extract Python classes for LCOM4 analysis
fn extract_python_classes_for_lcom4(
    node: Node,
    source: &str,
    classes: &mut Vec<ClassInfoForLcom4>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "class_definition" => {
                if let Some(class_info) = extract_python_class_info_for_lcom4(&child, source) {
                    classes.push(class_info);
                }
            }
            "decorated_definition" => {
                // Handle decorated classes
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "class_definition" {
                        if let Some(class_info) = extract_python_class_info_for_lcom4(&def, source)
                        {
                            classes.push(class_info);
                        }
                    }
                }
            }
            _ => {
                // Recurse into other nodes (module level)
                extract_python_classes_for_lcom4(child, source, classes);
            }
        }
    }
}

/// Extract a single Python class's info for LCOM4 analysis
fn extract_python_class_info_for_lcom4(node: &Node, source: &str) -> Option<ClassInfoForLcom4> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(&name_node, source);
    let start_line = node.start_position().row as u32 + 1;

    // Extract methods from class body
    let body = node.child_by_field_name("body")?;
    let methods = extract_python_methods_for_lcom4(&body, source);

    Some(ClassInfoForLcom4 {
        name,
        start_line,
        methods,
    })
}

/// Extract Python methods from a class body for LCOM4 analysis
fn extract_python_methods_for_lcom4(body: &Node, source: &str) -> Vec<MethodInfoForLcom4> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();

    for child in body.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(method_info) = extract_python_method_info_for_lcom4(&child, source) {
                    methods.push(method_info);
                }
            }
            "decorated_definition" => {
                // Handle decorated methods
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(method_info) =
                            extract_python_method_info_for_lcom4(&def, source)
                        {
                            methods.push(method_info);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    methods
}

/// Extract a single Python method's info for LCOM4 analysis
fn extract_python_method_info_for_lcom4(node: &Node, source: &str) -> Option<MethodInfoForLcom4> {
    let name_node = node.child_by_field_name("name")?;
    let name = get_node_text(&name_node, source);

    Some(MethodInfoForLcom4 {
        name,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    })
}

/// Analyze a single file for all debt issues
///
/// # Arguments
/// * `filepath` - Path to file
/// * `category_filter` - Optional category to filter by
/// * `language` - Optional language override
///
/// # Returns
/// Tuple of (issues, lines_of_code)
///
/// # Algorithm
/// - Read file content (graceful degradation on read error)
/// - Detect language (or use override)
/// - Run all detectors: count_loc, find_todo_comments, find_complexity_issues,
///   find_god_classes, find_deep_nesting, find_high_coupling, find_missing_docs
/// - Apply category filter if provided
/// - Return (issues, loc)
pub fn analyze_file(
    filepath: &Path,
    category_filter: Option<&str>,
    language_override: Option<Language>,
) -> TldrResult<(Vec<DebtIssue>, usize)> {
    // Read file - graceful degradation on read error
    let source = match std::fs::read_to_string(filepath) {
        Ok(s) => s,
        Err(_) => return Ok((vec![], 0)), // Graceful degradation
    };

    // Detect language (or use override)
    let lang = language_override.or_else(|| Language::from_path(filepath));
    let lang = match lang {
        Some(l) => l,
        None => return Ok((vec![], 0)), // Unsupported file type
    };

    // Count LOC
    let loc = count_loc(&source, lang);

    // Parse the file ONCE and share the tree across all detectors (Fix 3: 3-5x speedup)
    let tree = parse(&source, lang).ok();
    let tree_ref = tree.as_ref();

    // Collect all issues from detectors, passing shared tree
    let mut issues = Vec::new();

    // TODO comments (AST-based, works for all 18 languages)
    issues.extend(find_todo_comments_inner(&source, filepath, lang, tree_ref));

    // Complexity issues (CC, long method, long params) - uses batch complexity internally
    issues.extend(find_complexity_issues_inner(
        &source, filepath, lang, tree_ref,
    ));

    // God classes
    issues.extend(find_god_classes_inner(&source, filepath, lang, tree_ref));

    // Deep nesting
    issues.extend(find_deep_nesting_inner(&source, filepath, lang, tree_ref));

    // High coupling
    issues.extend(find_high_coupling_inner(&source, filepath, lang, tree_ref));

    // Missing docs
    issues.extend(find_missing_docs_inner(&source, filepath, lang, tree_ref));

    // Apply category filter if provided
    if let Some(category) = category_filter {
        issues.retain(|issue| issue.category == category);
    }

    Ok((issues, loc))
}

/// Extra skip dirs beyond what [`crate::walker::DEFAULT_EXCLUDE_DIRS`]
/// already covers. The shared walker handles `node_modules`, `target`,
/// `dist`, `build`, `.next`, `__pycache__`, `vendor`, `.git`, and all
/// hidden dirs. Python virtualenv and cache dirs still have to be
/// filtered post-walk since those names are project-specific.
const DEBT_EXTRA_SKIP_DIRS: &[&str] = &[".venv", "venv", ".tox", ".mypy_cache"];

/// Return `true` if any component of `path` (relative to `root`) is in
/// the extra skip list. The shared walker already handles hidden dirs
/// and the project's standard vendor directories.
fn debt_has_skipped_component(path: &std::path::Path, root: &std::path::Path) -> bool {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| DEBT_EXTRA_SKIP_DIRS.contains(&s))
            .unwrap_or(false)
    })
}

/// Main entry point for analyzing technical debt
///
/// # Arguments
/// * `options` - Analysis options including path, filters, etc.
///
/// # Returns
/// Complete debt report with issues, top files, and summary
///
/// # Algorithm
/// 1. If path is file, analyze single file
/// 2. If path is directory, walk recursively (skipping SKIP_DIRS)
/// 3. Skip hidden files/dirs (starting with `.`)
/// 4. Collect all issues
/// 5. Build FileDebt for each file with issues
/// 6. Sort by total_minutes descending
/// 7. Take top_k files
/// 8. Build DebtSummary with aggregations
/// 9. Return DebtReport
pub fn analyze_debt(options: DebtOptions) -> TldrResult<DebtReport> {
    use std::collections::HashMap;

    let path = &options.path;

    // Validate path exists
    if !path.exists() {
        return Err(crate::TldrError::PathNotFound(path.clone()));
    }

    let mut all_issues: Vec<DebtIssue> = Vec::new();
    let mut total_loc: usize = 0;
    let mut file_debts: HashMap<PathBuf, FileDebt> = HashMap::new();

    if path.is_file() {
        // Analyze single file
        let (issues, loc) =
            analyze_file(path, options.category_filter.as_deref(), options.language)?;
        total_loc += loc;

        if !issues.is_empty() {
            let total_minutes: u32 = issues.iter().map(|i| i.debt_minutes).sum();
            file_debts.insert(
                path.clone(),
                FileDebt {
                    file: path.clone(),
                    total_minutes,
                    issue_count: issues.len(),
                    issues: issues.clone(),
                },
            );
            all_issues.extend(issues);
        }
    } else if path.is_dir() {
        // Max file size to analyze (500KB) - skip minified/generated files
        const MAX_FILE_SIZE: u64 = 500 * 1024;

        // Walk directory recursively via the shared walker (handles
        // `.git`, `node_modules`, `target`, `dist`, `build`, `.next`,
        // `__pycache__`, `vendor`, hidden dirs, and symlink guards).
        // Post-filter for `.venv`/`venv`/`.tox`/`.mypy_cache` which are
        // in this module's historical SKIP_DIRS but not in the walker's
        // defaults.
        let file_paths: Vec<PathBuf> = crate::walker::walk_project(path)
            .filter(|e| !debt_has_skipped_component(e.path(), path))
            .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
            .filter(|e| Language::from_path(e.path()).is_some() || options.language.is_some())
            .filter(|e| {
                e.metadata()
                    .map(|m| m.len() <= MAX_FILE_SIZE)
                    .unwrap_or(true)
            })
            .map(|e| e.path().to_path_buf())
            .collect();

        // Process files in parallel using rayon (Fix 2: 4-8x speedup)
        let category_filter = options.category_filter.as_deref();
        let language_opt = options.language;
        let results: Vec<(PathBuf, Vec<DebtIssue>, usize)> = file_paths
            .par_iter()
            .filter_map(
                |fpath| match analyze_file(fpath, category_filter, language_opt) {
                    Ok((issues, loc)) => Some((fpath.clone(), issues, loc)),
                    Err(_) => None,
                },
            )
            .collect();

        // Merge parallel results (single-threaded aggregation)
        for (fpath, issues, loc) in results {
            total_loc += loc;

            if !issues.is_empty() {
                let total_minutes: u32 = issues.iter().map(|i| i.debt_minutes).sum();
                file_debts.insert(
                    fpath.clone(),
                    FileDebt {
                        file: fpath,
                        total_minutes,
                        issue_count: issues.len(),
                        issues: issues.clone(),
                    },
                );
                all_issues.extend(issues);
            }
        }
    }

    // Filter by minimum debt threshold
    if options.min_debt > 0 {
        all_issues.retain(|i| i.debt_minutes >= options.min_debt);
    }

    // Calculate summary statistics
    let total_minutes: u32 = all_issues.iter().map(|i| i.debt_minutes).sum();
    let total_hours = (total_minutes as f64 / 60.0 * 100.0).round() / 100.0;

    // Debt ratio: total_minutes / total_loc (rounded to 3 decimals)
    let debt_ratio = if total_loc > 0 {
        ((total_minutes as f64 / total_loc as f64) * 1000.0).round() / 1000.0
    } else {
        0.0
    };

    // Debt density: debt_ratio * 1000 (minutes per KLOC, rounded to 2 decimals)
    let debt_density = (debt_ratio * 1000.0 * 100.0).round() / 100.0;

    // Total cost if hourly rate provided
    let total_cost = options
        .hourly_rate
        .map(|rate| (total_hours * rate * 100.0).round() / 100.0);

    // Group by category
    let mut by_category: BTreeMap<String, u32> = BTreeMap::new();
    for issue in &all_issues {
        *by_category.entry(issue.category.clone()).or_default() += issue.debt_minutes;
    }

    // Group by rule
    let mut by_rule: BTreeMap<String, u32> = BTreeMap::new();
    for issue in &all_issues {
        *by_rule.entry(issue.rule.clone()).or_default() += issue.debt_minutes;
    }

    // Sort issues by debt_minutes descending
    all_issues.sort_by(|a, b| b.debt_minutes.cmp(&a.debt_minutes));

    // Top files by debt (sorted by total_minutes descending, limited to top_k)
    let mut sorted_files: Vec<_> = file_debts.values().cloned().collect();
    sorted_files.sort_by(|a, b| b.total_minutes.cmp(&a.total_minutes));
    let top_files: Vec<_> = sorted_files.into_iter().take(options.top_k).collect();

    Ok(DebtReport {
        issues: all_issues,
        top_files,
        summary: DebtSummary {
            total_minutes,
            total_hours,
            total_cost,
            debt_ratio,
            debt_density,
            by_category,
            by_rule,
        },
    })
}
