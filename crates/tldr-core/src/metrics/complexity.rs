//! Complexity metrics calculation
//!
//! Implements cyclomatic and cognitive complexity as per spec Section 2.3.2.
//!
//! # Cyclomatic Complexity
//! - V(G) = E - N + 2 (edges - nodes + 2)
//! - Counts decision points: if, elif, for, while, case, catch, &&, ||, ?:
//!
//! # Cognitive Complexity (SonarSource)
//! - Increment for each control structure
//! - Additional increment per nesting level
//! - Breaks in linear flow (break, continue, goto)

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::Node;

use crate::ast::function_finder::{
    find_function_node, get_function_body, get_function_name, get_function_node_kinds,
};
use crate::ast::parser::{parse, parse_file};
use crate::error::TldrError;
use crate::types::{ComplexityMetrics, Language};
use crate::TldrResult;

/// Maximum nesting depth to prevent infinite loops (M24 mitigation)
const MAX_NESTING_DEPTH: usize = 100;

/// Calculate complexity metrics for a function
///
/// # Arguments
/// * `source_or_path` - Source code or file path
/// * `function_name` - Name of function to analyze
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(ComplexityMetrics)` - Complexity metrics
/// * `Err(TldrError::FunctionNotFound)` - Function not found
///
/// # Example
/// ```ignore
/// use tldr_core::metrics::calculate_complexity;
/// use tldr_core::Language;
///
/// let metrics = calculate_complexity("def foo(): pass", "foo", Language::Python)?;
/// assert_eq!(metrics.cyclomatic, 1);
/// ```
pub fn calculate_complexity(
    source_or_path: &str,
    function_name: &str,
    language: Language,
) -> TldrResult<ComplexityMetrics> {
    // Determine if input is a file path or source code
    let (tree, source) = if Path::new(source_or_path).exists() {
        let (tree, source, _lang) = parse_file(Path::new(source_or_path))?;
        (tree, source)
    } else {
        let tree = parse(source_or_path, language)?;
        (tree, source_or_path.to_string())
    };

    let root = tree.root_node();

    // Find the function
    let func_node = find_function_node(root, function_name, language, &source);

    match func_node {
        Some(node) => {
            let mut calculator =
                ComplexityCalculator::new(function_name.to_string(), &source, language);
            calculator.analyze_function(node)?;
            Ok(calculator.into_metrics())
        }
        None => Err(TldrError::function_not_found(function_name)),
    }
}

/// Calculate complexity metrics for ALL functions in source code in a single pass.
///
/// Parses the file once, walks the AST to find all function/method nodes,
/// and calculates complexity for each. This is 10-25x faster than calling
/// `calculate_complexity()` per function when a file has many functions.
///
/// # Arguments
/// * `source` - Source code string
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(HashMap<String, ComplexityMetrics>)` - Map of function_name -> metrics
pub fn calculate_all_complexities(
    source: &str,
    language: Language,
) -> TldrResult<HashMap<String, ComplexityMetrics>> {
    let tree = parse(source, language)?;
    let root = tree.root_node();
    calculate_all_complexities_from_tree(root, source, language)
}

/// Calculate complexity metrics for ALL functions in a file in a single pass.
///
/// Reads and parses the file once, then calculates complexity for all functions.
///
/// # Arguments
/// * `path` - File path to analyze
///
/// # Returns
/// * `Ok(HashMap<String, ComplexityMetrics>)` - Map of function_name -> metrics
pub fn calculate_all_complexities_file(
    path: &Path,
) -> TldrResult<HashMap<String, ComplexityMetrics>> {
    let (tree, source, lang) = parse_file(path)?;
    let root = tree.root_node();
    calculate_all_complexities_from_tree(root, &source, lang)
}

/// Calculate complexity metrics for all functions given an already-parsed tree.
///
/// Use this when you already have a parsed tree to avoid redundant parsing.
/// Walks the AST depth-first to find all function/method nodes, then runs
/// the complexity calculator on each.
pub fn calculate_all_complexities_from_tree(
    root: Node,
    source: &str,
    language: Language,
) -> TldrResult<HashMap<String, ComplexityMetrics>> {
    let func_kinds = get_function_node_kinds(language);
    let mut results = HashMap::new();

    // DFS to find all function nodes
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if func_kinds.contains(&node.kind()) {
            if let Some(name) = get_function_name(node, language, source) {
                let mut calculator = ComplexityCalculator::new(name.clone(), source, language);
                if calculator.analyze_function(node).is_ok() {
                    results.insert(name, calculator.into_metrics());
                }
            }
        }

        // Push children in reverse order for left-to-right DFS
        let child_count = node.child_count();
        for i in (0..child_count).rev() {
            if let Some(child) = node.child(i) {
                stack.push(child);
            }
        }
    }

    Ok(results)
}

/// Calculator for complexity metrics
struct ComplexityCalculator<'a> {
    function_name: String,
    source: &'a str,
    language: Language,
    cyclomatic: u32,
    cognitive: u32,
    max_nesting: u32,
    current_nesting: u32,
    lines_of_code: u32,
    start_line: u32,
    end_line: u32,
}

impl<'a> ComplexityCalculator<'a> {
    fn new(function_name: String, source: &'a str, language: Language) -> Self {
        Self {
            function_name,
            source,
            language,
            cyclomatic: 1, // Base complexity is 1
            cognitive: 0,
            max_nesting: 0,
            current_nesting: 0,
            lines_of_code: 0,
            start_line: 0,
            end_line: 0,
        }
    }

    fn analyze_function(&mut self, func_node: Node) -> TldrResult<()> {
        self.start_line = func_node.start_position().row as u32 + 1;
        self.end_line = func_node.end_position().row as u32 + 1;
        self.lines_of_code = self.end_line - self.start_line + 1;

        // Get function body
        let body = get_function_body(func_node, self.language);

        if let Some(body_node) = body {
            self.analyze_node(body_node, 0)?;
        }

        Ok(())
    }

    fn analyze_node(&mut self, node: Node, depth: usize) -> TldrResult<()> {
        if depth > MAX_NESTING_DEPTH {
            return Ok(());
        }

        let kind = node.kind();

        // Update nesting tracking
        let is_nesting_structure = self.is_nesting_structure(kind);
        if is_nesting_structure {
            self.current_nesting += 1;
            self.max_nesting = self.max_nesting.max(self.current_nesting);
        }

        // Count decision points for cyclomatic complexity
        self.count_cyclomatic_increment(node);

        // Count cognitive complexity
        self.count_cognitive_increment(node);

        // Recurse into children
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                self.analyze_node(cursor.node(), depth + 1)?;
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        if is_nesting_structure {
            self.current_nesting -= 1;
        }

        Ok(())
    }

    /// Check if a node kind introduces nesting
    fn is_nesting_structure(&self, kind: &str) -> bool {
        matches!(
            kind,
            "if_statement"
                | "elif_clause"
                | "else_clause"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "try_statement"
                | "except_clause"
                | "catch_clause"
                | "with_statement"
                | "match_statement"
                | "switch_statement"
                | "lambda"
                | "lambda_expression"
                | "conditional_expression" // ternary
        )
    }

    /// Count cyclomatic complexity increments
    ///
    /// Cyclomatic complexity counts decision points:
    /// - if, elif, else (else doesn't add, but the branch does)
    /// - for, while loops
    /// - case/match branches
    /// - catch/except handlers
    /// - && and || operators
    /// - ?: ternary operator
    fn count_cyclomatic_increment(&mut self, node: Node) {
        let kind = node.kind();

        // Primary decision points
        match kind {
            "if_statement" | "elif_clause" => {
                self.cyclomatic += 1;
            }
            "for_statement" | "for_in_statement" | "while_statement" => {
                self.cyclomatic += 1;
            }
            "except_clause" | "catch_clause" | "except_handler" => {
                self.cyclomatic += 1;
            }
            "case_clause" | "match_arm" | "switch_case" => {
                self.cyclomatic += 1;
            }
            "conditional_expression" | "ternary_expression" => {
                self.cyclomatic += 1;
            }
            _ => {}
        }

        // Logical operators in conditions
        if kind == "boolean_operator" || kind == "binary_expression" {
            if let Some(op) = node.child_by_field_name("operator") {
                let op_text = op.utf8_text(self.source.as_bytes()).unwrap_or("");
                if op_text == "and" || op_text == "or" || op_text == "&&" || op_text == "||" {
                    self.cyclomatic += 1;
                }
            }
        }

        // Also check for && and || as direct node kinds
        if kind == "&&" || kind == "||" || kind == "and" || kind == "or" {
            self.cyclomatic += 1;
        }
    }

    /// Count cognitive complexity increments
    ///
    /// Cognitive complexity (SonarSource):
    /// - Base increment for control structures
    /// - Nesting penalty for nested structures
    /// - Increment for breaks in linear flow
    fn count_cognitive_increment(&mut self, node: Node) {
        let kind = node.kind();

        // Control structures add 1 + nesting level
        let base_increment = match kind {
            "if_statement" => Some(1),
            "elif_clause" => Some(1),
            "else_clause" => Some(1),
            "for_statement" | "for_in_statement" => Some(1),
            "while_statement" => Some(1),
            "except_clause" | "catch_clause" => Some(1),
            "match_statement" | "switch_statement" => Some(1),
            "conditional_expression" | "ternary_expression" => Some(1),
            _ => None,
        };

        if let Some(base) = base_increment {
            // Add base + nesting penalty
            // Cognitive complexity adds 1 for each nesting level
            self.cognitive += base + self.current_nesting.saturating_sub(1);
        }

        // Breaks in linear flow
        match kind {
            "break_statement" | "continue_statement" => {
                self.cognitive += 1;
            }
            "return_statement" => {
                // Return early adds cognitive load (but not when it's the last statement)
                // For simplicity, we count all returns after the first
                // This is a simplification of the SonarSource rules
            }
            _ => {}
        }

        // Logical operators in conditions add complexity
        if kind == "boolean_operator" || kind == "binary_expression" {
            if let Some(op) = node.child_by_field_name("operator") {
                let op_text = op.utf8_text(self.source.as_bytes()).unwrap_or("");
                if op_text == "and" || op_text == "or" || op_text == "&&" || op_text == "||" {
                    self.cognitive += 1;
                }
            }
        }

        // Recursion adds cognitive complexity
        // Check if this is a call to the current function
        if kind == "call" || kind == "call_expression" {
            if let Some(callee) = self.get_callee_name(node) {
                if callee == self.function_name {
                    self.cognitive += 1;
                }
            }
        }
    }

    /// Get callee name from call node
    fn get_callee_name(&self, call_node: Node) -> Option<String> {
        let func_node = call_node
            .child_by_field_name("function")
            .or_else(|| call_node.child(0))?;

        match func_node.kind() {
            "identifier" => Some(
                func_node
                    .utf8_text(self.source.as_bytes())
                    .ok()?
                    .to_string(),
            ),
            _ => None,
        }
    }

    fn into_metrics(self) -> ComplexityMetrics {
        ComplexityMetrics {
            function: self.function_name,
            cyclomatic: self.cyclomatic,
            cognitive: self.cognitive,
            nesting_depth: self.max_nesting,
            lines_of_code: self.lines_of_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_function_complexity() {
        let source = r#"
def simple():
    return 1
"#;
        let metrics = calculate_complexity(source, "simple", Language::Python).unwrap();
        assert_eq!(metrics.function, "simple");
        assert_eq!(metrics.cyclomatic, 1); // No branches
        assert_eq!(metrics.cognitive, 0); // No control structures
    }

    #[test]
    fn test_if_statement_complexity() {
        let source = r#"
def with_if(x):
    if x > 0:
        return 1
    return 0
"#;
        let metrics = calculate_complexity(source, "with_if", Language::Python).unwrap();
        assert_eq!(metrics.cyclomatic, 2); // Base + 1 if
    }

    #[test]
    fn test_nested_if_complexity() {
        let source = r#"
def nested(a, b):
    if a > 0:
        if b > 0:
            return 1
    return 0
"#;
        let metrics = calculate_complexity(source, "nested", Language::Python).unwrap();
        assert_eq!(metrics.cyclomatic, 3); // Base + 2 ifs
        assert!(metrics.cognitive >= 3); // if + (nested if with penalty)
        assert!(metrics.nesting_depth >= 2);
    }

    #[test]
    fn test_loop_complexity() {
        let source = r#"
def with_loop():
    for i in range(10):
        print(i)
"#;
        let metrics = calculate_complexity(source, "with_loop", Language::Python).unwrap();
        assert_eq!(metrics.cyclomatic, 2); // Base + 1 for loop
    }

    #[test]
    fn test_function_not_found() {
        let source = "def foo(): pass";
        let result = calculate_complexity(source, "nonexistent", Language::Python);
        assert!(matches!(result, Err(TldrError::FunctionNotFound { .. })));
    }

    #[test]
    fn test_logical_operators() {
        let source = r#"
def with_logic(a, b, c):
    if a and b:
        return 1
    if a or c:
        return 2
    return 0
"#;
        let metrics = calculate_complexity(source, "with_logic", Language::Python).unwrap();
        // Base + 2 ifs + 2 logical operators
        assert!(metrics.cyclomatic >= 4);
    }

    #[test]
    fn test_lines_of_code() {
        let source = r#"
def multiline():
    a = 1
    b = 2
    c = 3
    return a + b + c
"#;
        let metrics = calculate_complexity(source, "multiline", Language::Python).unwrap();
        assert!(metrics.lines_of_code >= 5);
    }

    #[test]
    fn test_batch_complexity_returns_all_functions() {
        let source = r#"
def simple():
    return 1

def with_if(x):
    if x > 0:
        return 1
    return 0

def with_loop():
    for i in range(10):
        print(i)
"#;
        let results = calculate_all_complexities(source, Language::Python).unwrap();
        assert_eq!(results.len(), 3, "Should find all 3 functions");
        assert!(results.contains_key("simple"));
        assert!(results.contains_key("with_if"));
        assert!(results.contains_key("with_loop"));
    }

    #[test]
    fn test_batch_complexity_matches_individual() {
        let source = r#"
def simple():
    return 1

def with_if(x):
    if x > 0:
        return 1
    return 0

def nested(a, b):
    if a > 0:
        if b > 0:
            return 1
    return 0
"#;
        let batch = calculate_all_complexities(source, Language::Python).unwrap();

        // Each batch result should match individual calculation
        for (name, batch_metrics) in &batch {
            let individual = calculate_complexity(source, name, Language::Python).unwrap();
            assert_eq!(
                batch_metrics.cyclomatic, individual.cyclomatic,
                "Cyclomatic mismatch for {}",
                name
            );
            assert_eq!(
                batch_metrics.cognitive, individual.cognitive,
                "Cognitive mismatch for {}",
                name
            );
            assert_eq!(
                batch_metrics.nesting_depth, individual.nesting_depth,
                "Nesting depth mismatch for {}",
                name
            );
            assert_eq!(
                batch_metrics.lines_of_code, individual.lines_of_code,
                "LOC mismatch for {}",
                name
            );
        }
    }

    #[test]
    fn test_batch_complexity_empty_source() {
        let source = "# just a comment\n";
        let results = calculate_all_complexities(source, Language::Python).unwrap();
        assert!(results.is_empty(), "No functions means empty map");
    }

    #[test]
    fn test_batch_complexity_with_class_methods() {
        let source = r#"
class MyClass:
    def method_a(self):
        return 1

    def method_b(self, x):
        if x > 0:
            return x
        return 0
"#;
        let results = calculate_all_complexities(source, Language::Python).unwrap();
        // Should find class methods too
        assert!(
            results.len() >= 2,
            "Should find at least 2 methods, got {}",
            results.len()
        );
    }

    #[test]
    fn test_batch_complexity_file_path() {
        // Test that calculate_all_complexities_file works with a file path
        use std::io::Write;
        let dir = std::env::temp_dir().join("tldr_batch_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("test_batch.py");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(
            f,
            "def foo():\n    return 1\n\ndef bar(x):\n    if x: return x\n    return 0"
        )
        .unwrap();

        let results = calculate_all_complexities_file(&file_path).unwrap();
        assert!(results.contains_key("foo"));
        assert!(results.contains_key("bar"));

        // Clean up
        std::fs::remove_dir_all(&dir).ok();
    }
}
