//! Temporal Command - Temporal Constraint Mining
//!
//! Mines temporal constraints (method call sequences) from a codebase.
//!
//! # Algorithm
//!
//! 1. Extract method call sequences from each function
//! 2. Build frequency table of (before, after) pairs (bigrams)
//! 3. Calculate confidence: count(A->B) / count(A)
//! 4. Filter by min_support and min_confidence
//! 5. Optionally mine trigrams (3-method sequences)
//!
//! # TIGER Mitigations
//!
//! - **T05**: MAX_TRIGRAMS=10000 with BinaryHeap top-K selection
//! - **E03**: --timeout flag (default 60s)
//!
//! # Example
//!
//! ```bash
//! # Mine constraints from a directory
//! tldr temporal src/ --min-support 2 --min-confidence 0.5
//!
//! # Filter for specific method
//! tldr temporal src/ --query open
//!
//! # Include trigram patterns
//! tldr temporal src/ --include-trigrams
//! ```

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Args;
use tree_sitter::{Node, Parser};

use tldr_core::types::Language;

use crate::output::OutputFormat as GlobalOutputFormat;

use super::error::{PatternsError, PatternsResult};
use super::types::{
    OutputFormat, TemporalConstraint, TemporalExample, TemporalMetadata, TemporalReport, Trigram,
};
use super::validation::{
    check_directory_file_count, read_file_safe, validate_directory_path, validate_file_path,
    validate_file_path_in_project, MAX_TRIGRAMS,
};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Mine temporal constraints (method call sequences) from a codebase.
#[derive(Debug, Args)]
pub struct TemporalArgs {
    /// Directory or file to analyze
    pub path: PathBuf,

    /// Minimum occurrences for a pattern
    #[arg(long, default_value = "2")]
    pub min_support: u32,

    /// Minimum confidence threshold (0.0-1.0)
    #[arg(long, default_value = "0.5")]
    pub min_confidence: f64,

    /// Filter for specific method
    #[arg(long)]
    pub query: Option<String>,

    /// Source language (only 'python' supported)
    #[arg(long = "source-lang", default_value = "python")]
    pub source_lang: String,

    /// Maximum files to analyze
    #[arg(long, default_value = "1000")]
    pub max_files: u32,

    /// Mine 3-method sequences
    #[arg(long)]
    pub include_trigrams: bool,

    /// Number of examples per constraint
    #[arg(long, default_value = "3")]
    pub include_examples: u32,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output",
        short = 'o',
        hide = true,
        default_value = "json",
        value_enum
    )]
    pub output_format: OutputFormat,

    /// Timeout in seconds (E03 mitigation)
    #[arg(long, default_value = "60")]
    pub timeout: u64,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,

    /// Language filter (auto-detected if omitted)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl TemporalArgs {
    /// Run the temporal analysis command
    pub fn run(&self, global_format: GlobalOutputFormat) -> anyhow::Result<()> {
        run(self.clone(), global_format)
    }
}

impl Clone for TemporalArgs {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            min_support: self.min_support,
            min_confidence: self.min_confidence,
            query: self.query.clone(),
            source_lang: self.source_lang.clone(),
            max_files: self.max_files,
            include_trigrams: self.include_trigrams,
            include_examples: self.include_examples,
            output_format: self.output_format,
            timeout: self.timeout,
            project_root: self.project_root.clone(),
            lang: self.lang,
        }
    }
}

// =============================================================================
// Sequence Extraction
// =============================================================================

/// Extractor for method call sequences from source code.
#[derive(Debug, Default)]
pub struct SequenceExtractor {
    /// Current function being analyzed
    current_function: String,
    /// Extracted sequences: object_key -> list of method names
    sequences: HashMap<String, Vec<String>>,
    /// Variable assignments: variable -> assigned from (for tracking objects)
    var_assignments: HashMap<String, String>,
    /// Current line number
    current_line: u32,
}

impl SequenceExtractor {
    /// Create a new sequence extractor for a file
    pub fn new() -> Self {
        Self::default()
    }

    /// Extract sequences from a function node
    pub fn extract_function(&mut self, func_node: Node, source: &[u8]) {
        // Get function name
        let func_name = self.get_function_name(func_node, source);
        if func_name.is_empty() {
            return;
        }
        self.current_function = func_name;
        self.var_assignments.clear();

        // Walk the function body and extract call sequences
        self.extract_calls_recursive(func_node, source, 0);
    }

    /// Recursively extract method calls from AST nodes
    fn extract_calls_recursive(&mut self, node: Node, source: &[u8], depth: usize) {
        // Prevent stack overflow
        if depth > 100 {
            return;
        }

        self.current_line = node.start_position().row as u32 + 1;

        match node.kind() {
            // Track assignments: x = open(...) or x = something.method()
            "assignment" => {
                self.handle_assignment(node, source);
            }

            // Track method calls: x.read(), x.close(), etc.
            "call" => {
                self.handle_call(node, source);
            }

            // Track with statements: with open(...) as f
            "with_statement" => {
                self.handle_with_statement(node, source);
            }

            _ => {}
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_calls_recursive(child, source, depth + 1);
        }
    }

    /// Handle an assignment statement
    fn handle_assignment(&mut self, node: Node, source: &[u8]) {
        // Get the left side (variable name)
        let var_name = if let Some(left) = node.child_by_field_name("left") {
            self.node_text(left, source).to_string()
        } else {
            // Try to find pattern targets (for simple assignments)
            let mut var = String::new();
            for child in node.children(&mut node.walk()) {
                if child.kind() == "identifier" {
                    var = self.node_text(child, source).to_string();
                    break;
                }
            }
            var
        };

        if var_name.is_empty() {
            return;
        }

        // Get the right side (value)
        if let Some(right) = node.child_by_field_name("right") {
            // Check if it's a call expression
            if right.kind() == "call" {
                let call_name = self.extract_call_name(right, source);
                if !call_name.is_empty() {
                    // Track the assignment: var_name was assigned from call_name
                    self.var_assignments
                        .insert(var_name.clone(), call_name.clone());

                    // Add to sequence: func:var -> [constructor_call]
                    let key = format!("{}:{}", self.current_function, var_name);
                    self.sequences.entry(key).or_default().push(call_name);
                }
            }
        }
    }

    /// Handle a call expression
    fn handle_call(&mut self, node: Node, source: &[u8]) {
        // Extract the call structure: object.method() or function()
        if let Some(func) = node.child_by_field_name("function") {
            if func.kind() == "attribute" {
                // Method call: obj.method()
                if let Some(obj) = func.child_by_field_name("object") {
                    let obj_name = self.node_text(obj, source).to_string();
                    if let Some(method) = func.child_by_field_name("attribute") {
                        let method_name = self.node_text(method, source).to_string();

                        // Add to sequence for this object
                        let key = format!("{}:{}", self.current_function, obj_name);
                        self.sequences.entry(key).or_default().push(method_name);
                    }
                }
            }
        }
    }

    /// Handle a with statement
    fn handle_with_statement(&mut self, node: Node, source: &[u8]) {
        // Extract: with open(path) as f
        for child in node.children(&mut node.walk()) {
            if child.kind() == "with_clause" {
                for item in child.children(&mut child.walk()) {
                    if item.kind() == "with_item" {
                        // Get the expression (open(...))
                        let mut call_name = String::new();
                        let mut var_name = String::new();

                        for part in item.children(&mut item.walk()) {
                            if part.kind() == "call" {
                                call_name = self.extract_call_name(part, source);
                            } else if part.kind() == "as_pattern" || part.kind() == "identifier" {
                                // Get the alias
                                if part.kind() == "identifier" {
                                    var_name = self.node_text(part, source).to_string();
                                } else {
                                    for as_child in part.children(&mut part.walk()) {
                                        if as_child.kind() == "identifier" {
                                            var_name = self.node_text(as_child, source).to_string();
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if !call_name.is_empty() && !var_name.is_empty() {
                            let key = format!("{}:{}", self.current_function, var_name);
                            self.sequences
                                .entry(key.clone())
                                .or_default()
                                .push(call_name);
                            // with statement implies automatic close
                            self.sequences
                                .entry(key)
                                .or_default()
                                .push("__exit__".to_string());
                        }
                    }
                }
            }
        }
    }

    /// Extract the call name from a call node
    fn extract_call_name(&self, node: Node, source: &[u8]) -> String {
        if let Some(func) = node.child_by_field_name("function") {
            return self.extract_name_from_expr(func, source);
        }

        // Fallback: iterate children
        for child in node.children(&mut node.walk()) {
            match child.kind() {
                "identifier" => return self.node_text(child, source).to_string(),
                "attribute" => return self.extract_name_from_expr(child, source),
                _ => continue,
            }
        }
        String::new()
    }

    /// Extract a dotted name from an expression
    fn extract_name_from_expr(&self, node: Node, source: &[u8]) -> String {
        match node.kind() {
            "identifier" => self.node_text(node, source).to_string(),
            "attribute" => {
                // Get just the last part (method name)
                if let Some(attr) = node.child_by_field_name("attribute") {
                    self.node_text(attr, source).to_string()
                } else {
                    String::new()
                }
            }
            _ => self.node_text(node, source).to_string(),
        }
    }

    /// Get function name from a function definition
    fn get_function_name(&self, node: Node, source: &[u8]) -> String {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "identifier" {
                return self.node_text(child, source).to_string();
            }
        }
        String::new()
    }

    /// Get text for a node
    fn node_text<'a>(&self, node: Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    /// Get extracted sequences
    pub fn get_sequences(&self) -> &HashMap<String, Vec<String>> {
        &self.sequences
    }
}

/// Extract method call sequences from source code
pub fn extract_sequences(source: &str) -> HashMap<String, Vec<String>> {
    let mut extractor = SequenceExtractor::new();

    // Parse with tree-sitter
    let mut parser = match get_python_parser() {
        Ok(p) => p,
        Err(_) => return HashMap::new(),
    };

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return HashMap::new(),
    };

    let root = tree.root_node();
    let source_bytes = source.as_bytes();

    // Find all function definitions and extract sequences
    extract_functions_recursive(root, source_bytes, &mut extractor);

    extractor.sequences
}

/// Recursively find function definitions and extract sequences
fn extract_functions_recursive(node: Node, source: &[u8], extractor: &mut SequenceExtractor) {
    match node.kind() {
        "function_definition" | "async_function_definition" => {
            extractor.extract_function(node, source);
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_functions_recursive(child, source, extractor);
    }
}

// =============================================================================
// Bigram Mining
// =============================================================================

/// Counter for bigrams with example tracking
#[derive(Debug, Default)]
pub struct BigramCounter {
    /// Bigram counts: (before, after) -> count
    pub counts: HashMap<(String, String), u32>,
    /// Before counts: method -> count of times it's followed by something
    pub before_counts: HashMap<String, u32>,
    /// Example locations: (before, after) -> list of (file, line)
    pub examples: HashMap<(String, String), Vec<TemporalExample>>,
}

impl BigramCounter {
    /// Create a new bigram counter
    pub fn new() -> Self {
        Self::default()
    }

    /// Add sequences from extraction
    pub fn add_sequences(&mut self, sequences: &HashMap<String, Vec<String>>, file: &str) {
        for calls in sequences.values() {
            // Parse function name from key (func:var)
            let line = 1u32; // Would need more tracking for accurate line numbers

            for i in 0..calls.len().saturating_sub(1) {
                let before = &calls[i];
                let after = &calls[i + 1];

                // Skip self-loops
                if before == after {
                    continue;
                }

                let pair = (before.clone(), after.clone());

                // Increment bigram count
                *self.counts.entry(pair.clone()).or_default() += 1;

                // Increment before count
                *self.before_counts.entry(before.clone()).or_default() += 1;

                // Add example
                self.examples
                    .entry(pair)
                    .or_default()
                    .push(TemporalExample {
                        file: file.to_string(),
                        line,
                    });
            }
        }
    }
}

/// Mine bigram constraints from sequences
pub fn mine_bigrams(
    sequences: &HashMap<String, Vec<String>>,
    file: &str,
    args: &TemporalArgs,
) -> (BigramCounter, Vec<TemporalConstraint>) {
    let mut counter = BigramCounter::new();
    counter.add_sequences(sequences, file);

    let mut constraints = Vec::new();

    for ((before, after), count) in &counter.counts {
        // Filter by min_support
        if *count < args.min_support {
            continue;
        }

        // Calculate confidence
        let before_total = *counter.before_counts.get(before).unwrap_or(&1);
        let confidence = (*count as f64) / (before_total as f64);

        // Filter by min_confidence
        if confidence < args.min_confidence {
            continue;
        }

        // Get examples (limited)
        let examples = counter
            .examples
            .get(&(before.clone(), after.clone()))
            .map(|ex| {
                ex.iter()
                    .take(args.include_examples as usize)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        constraints.push(TemporalConstraint {
            before: before.clone(),
            after: after.clone(),
            support: *count,
            confidence,
            examples,
        });
    }

    // Sort by confidence (descending), then support (descending)
    constraints.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.support.cmp(&a.support))
    });

    (counter, constraints)
}

// =============================================================================
// Trigram Mining (TIGER-05: MAX_TRIGRAMS limit)
// =============================================================================

/// Mine trigram patterns with MAX_TRIGRAMS limit (TIGER-05)
pub fn mine_trigrams(
    sequences: &HashMap<String, Vec<String>>,
    args: &TemporalArgs,
) -> Vec<Trigram> {
    // Count trigrams
    let mut trigram_counts: HashMap<(String, String, String), u32> = HashMap::new();
    let mut bigram_follows: HashMap<(String, String), u32> = HashMap::new();

    for calls in sequences.values() {
        for i in 0..calls.len().saturating_sub(2) {
            let a = &calls[i];
            let b = &calls[i + 1];
            let c = &calls[i + 2];

            // Skip if any self-loops
            if a == b || b == c {
                continue;
            }

            *trigram_counts
                .entry((a.clone(), b.clone(), c.clone()))
                .or_default() += 1;

            // Count bigram follows
            if a != b {
                *bigram_follows.entry((a.clone(), b.clone())).or_default() += 1;
            }
        }
    }

    // TIGER-05: Use BinaryHeap for top-K selection to limit memory
    // We use a min-heap of size MAX_TRIGRAMS, keeping the largest support values
    let mut heap: BinaryHeap<Reverse<(u32, String, String, String)>> = BinaryHeap::new();

    for ((a, b, c), count) in &trigram_counts {
        if *count < args.min_support {
            continue;
        }

        // Calculate confidence
        let bigram_total = *bigram_follows.get(&(a.clone(), b.clone())).unwrap_or(&1);
        let confidence = (*count as f64) / (bigram_total as f64);

        if confidence < args.min_confidence {
            continue;
        }

        // Add to heap with support as priority
        if heap.len() < MAX_TRIGRAMS {
            heap.push(Reverse((*count, a.clone(), b.clone(), c.clone())));
        } else if let Some(&Reverse((min_support, _, _, _))) = heap.peek() {
            if *count > min_support {
                heap.pop();
                heap.push(Reverse((*count, a.clone(), b.clone(), c.clone())));
            }
        }
    }

    // Convert heap to sorted vector
    let mut trigrams: Vec<Trigram> = heap
        .into_iter()
        .map(|Reverse((support, a, b, c))| {
            let bigram_total = *bigram_follows.get(&(a.clone(), b.clone())).unwrap_or(&1);
            let confidence = (support as f64) / (bigram_total as f64);

            Trigram {
                sequence: [a, b, c],
                support,
                confidence,
            }
        })
        .collect();

    // Sort by confidence (descending), then support (descending)
    trigrams.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.support.cmp(&a.support))
    });

    trigrams
}

// =============================================================================
// Query Filtering
// =============================================================================

/// Filter constraints by query string
pub fn filter_by_query(
    constraints: Vec<TemporalConstraint>,
    query: &str,
) -> Vec<TemporalConstraint> {
    constraints
        .into_iter()
        .filter(|c| c.before.contains(query) || c.after.contains(query))
        .collect()
}

/// Filter trigrams by query string
pub fn filter_trigrams_by_query(trigrams: Vec<Trigram>, query: &str) -> Vec<Trigram> {
    trigrams
        .into_iter()
        .filter(|t| t.sequence.iter().any(|s| s.contains(query)))
        .collect()
}

// =============================================================================
// Tree-sitter Parser
// =============================================================================

/// Initialize tree-sitter parser for Python
fn get_python_parser() -> PatternsResult<Parser> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE;
    parser.set_language(&language.into()).map_err(|e| {
        PatternsError::parse_error(PathBuf::new(), format!("Failed to set language: {}", e))
    })?;
    Ok(parser)
}

// =============================================================================
// File Analysis
// =============================================================================

type TemporalFileAnalysis = (HashMap<String, Vec<String>>, Vec<TemporalConstraint>);

/// Analyze temporal constraints for a single file
fn analyze_temporal_file(
    path: &Path,
    args: &TemporalArgs,
) -> PatternsResult<TemporalFileAnalysis> {
    // Validate path
    let canonical = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(path, root)?
    } else {
        validate_file_path(path)?
    };

    // Read source
    let source = read_file_safe(&canonical)?;
    let file_path_str = canonical.to_string_lossy().to_string();

    // Extract sequences
    let sequences = extract_sequences(&source);

    // Mine bigrams
    let (_, constraints) = mine_bigrams(&sequences, &file_path_str, args);

    Ok((sequences, constraints))
}

/// Analyze temporal constraints for a directory
fn analyze_temporal_directory(
    path: &Path,
    args: &TemporalArgs,
    start_time: Instant,
) -> PatternsResult<TemporalReport> {
    let canonical = validate_directory_path(path)?;
    let timeout = Duration::from_secs(args.timeout);

    let mut all_sequences: HashMap<String, Vec<String>> = HashMap::new();
    let mut all_examples: HashMap<(String, String), Vec<TemporalExample>> = HashMap::new();
    let mut bigram_counts: HashMap<(String, String), u32> = HashMap::new();
    let mut before_counts: HashMap<String, u32> = HashMap::new();
    let mut files_analyzed = 0u32;

    // Walk directory
    for entry in walkdir::WalkDir::new(&canonical)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        // Check timeout (E03 mitigation)
        if start_time.elapsed() > timeout {
            break;
        }

        let entry_path = entry.path();

        // Skip non-Python files
        if entry_path.extension().is_none_or(|ext| ext != "py") {
            continue;
        }

        // Check file count limit
        files_analyzed += 1;
        if files_analyzed > args.max_files {
            break;
        }
        check_directory_file_count(files_analyzed as usize)?;

        // Analyze file
        let file_path_str = entry_path.to_string_lossy().to_string();
        if let Ok(source) = read_file_safe(entry_path) {
            let sequences = extract_sequences(&source);

            // Aggregate sequences
            for (key, calls) in &sequences {
                all_sequences
                    .entry(key.clone())
                    .or_default()
                    .extend(calls.clone());

                // Count bigrams
                for i in 0..calls.len().saturating_sub(1) {
                    let before = &calls[i];
                    let after = &calls[i + 1];

                    if before == after {
                        continue;
                    }

                    let pair = (before.clone(), after.clone());
                    *bigram_counts.entry(pair.clone()).or_default() += 1;
                    *before_counts.entry(before.clone()).or_default() += 1;

                    // Track examples
                    let examples = all_examples.entry(pair).or_default();
                    if examples.len() < args.include_examples as usize {
                        examples.push(TemporalExample {
                            file: file_path_str.clone(),
                            line: 1, // Would need better line tracking
                        });
                    }
                }
            }
        }
    }

    // Build constraints from aggregated data
    let mut constraints = Vec::new();

    for ((before, after), count) in &bigram_counts {
        if *count < args.min_support {
            continue;
        }

        let before_total = *before_counts.get(before).unwrap_or(&1);
        let confidence = (*count as f64) / (before_total as f64);

        if confidence < args.min_confidence {
            continue;
        }

        let examples = all_examples
            .get(&(before.clone(), after.clone()))
            .cloned()
            .unwrap_or_default();

        constraints.push(TemporalConstraint {
            before: before.clone(),
            after: after.clone(),
            support: *count,
            confidence,
            examples,
        });
    }

    // Sort by confidence, then support
    constraints.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.support.cmp(&a.support))
    });

    // Apply query filter if specified
    if let Some(ref query) = args.query {
        constraints = filter_by_query(constraints, query);
    }

    // Mine trigrams if requested
    let trigrams = if args.include_trigrams {
        let mut trigrams = mine_trigrams(&all_sequences, args);
        if let Some(ref query) = args.query {
            trigrams = filter_trigrams_by_query(trigrams, query);
        }
        trigrams
    } else {
        Vec::new()
    };

    let sequences_extracted: u32 = all_sequences.values().map(|v| v.len() as u32).sum();

    Ok(TemporalReport {
        constraints,
        trigrams,
        metadata: TemporalMetadata {
            files_analyzed,
            sequences_extracted,
            min_support: args.min_support,
            min_confidence: args.min_confidence,
        },
    })
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format a temporal report as human-readable text
pub fn format_temporal_text(report: &TemporalReport) -> String {
    let mut lines = Vec::new();

    lines.push("Temporal Constraints".to_string());
    lines.push("=".repeat(40));
    lines.push(String::new());

    if report.constraints.is_empty() {
        lines.push("No constraints found matching criteria.".to_string());
    } else {
        lines.push(format!("Found {} constraints:", report.constraints.len()));
        lines.push(String::new());

        for constraint in &report.constraints {
            lines.push(format!("  {} -> {}", constraint.before, constraint.after));
            lines.push(format!(
                "    support: {}, confidence: {:.2}",
                constraint.support, constraint.confidence
            ));

            if !constraint.examples.is_empty() {
                lines.push("    examples:".to_string());
                for example in &constraint.examples {
                    lines.push(format!("      - {}:{}", example.file, example.line));
                }
            }
            lines.push(String::new());
        }
    }

    if !report.trigrams.is_empty() {
        lines.push(String::new());
        lines.push("Trigrams".to_string());
        lines.push("-".repeat(40));
        lines.push(String::new());

        for trigram in &report.trigrams {
            lines.push(format!(
                "  {} -> {} -> {}",
                trigram.sequence[0], trigram.sequence[1], trigram.sequence[2]
            ));
            lines.push(format!(
                "    support: {}, confidence: {:.2}",
                trigram.support, trigram.confidence
            ));
            lines.push(String::new());
        }
    }

    lines.push(String::new());
    lines.push("Metadata".to_string());
    lines.push("-".repeat(40));
    lines.push(format!(
        "  Files analyzed: {}",
        report.metadata.files_analyzed
    ));
    lines.push(format!(
        "  Sequences extracted: {}",
        report.metadata.sequences_extracted
    ));
    lines.push(format!("  Min support: {}", report.metadata.min_support));
    lines.push(format!(
        "  Min confidence: {:.2}",
        report.metadata.min_confidence
    ));

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

/// Execute the temporal command
pub fn run(args: TemporalArgs, global_format: GlobalOutputFormat) -> anyhow::Result<()> {
    let start_time = Instant::now();
    let path = &args.path;

    // Validate language
    if args.source_lang.to_lowercase() != "python" && args.source_lang.to_lowercase() != "auto" {
        return Err(PatternsError::UnsupportedLanguage {
            language: args.source_lang.clone(),
        }
        .into());
    }

    let report = if path.is_dir() {
        analyze_temporal_directory(path, &args, start_time)?
    } else {
        let (sequences, mut constraints) = analyze_temporal_file(path, &args)?;

        // Apply query filter if specified
        if let Some(ref query) = args.query {
            constraints = filter_by_query(constraints, query);
        }

        // Mine trigrams if requested
        let trigrams = if args.include_trigrams {
            let mut trigrams = mine_trigrams(&sequences, &args);
            if let Some(ref query) = args.query {
                trigrams = filter_trigrams_by_query(trigrams, query);
            }
            trigrams
        } else {
            Vec::new()
        };

        let sequences_extracted: u32 = sequences.values().map(|v| v.len() as u32).sum();

        TemporalReport {
            constraints,
            trigrams,
            metadata: TemporalMetadata {
                files_analyzed: 1,
                sequences_extracted,
                min_support: args.min_support,
                min_confidence: args.min_confidence,
            },
        }
    };

    // Resolve format: global -f flag takes priority over hidden --output-format
    let use_text = matches!(global_format, GlobalOutputFormat::Text)
        || matches!(args.output_format, OutputFormat::Text);

    // Check if no constraints found -> exit code 2
    if report.constraints.is_empty() && report.trigrams.is_empty() {
        if use_text {
            println!("{}", format_temporal_text(&report));
        } else {
            let json = serde_json::to_string_pretty(&report)?;
            println!("{}", json);
        }
        std::process::exit(2);
    }

    if use_text {
        println!("{}", format_temporal_text(&report));
    } else {
        let json = serde_json::to_string_pretty(&report)?;
        println!("{}", json);
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    
    

    #[test]
    fn test_extract_sequences_simple() {
        let code = r#"
def read_config(path):
    f = open(path)
    content = f.read()
    f.close()
    return content
"#;
        let sequences = extract_sequences(code);

        // Should have a sequence for f
        let has_f_sequence = sequences.keys().any(|k| k.contains(":f"));
        assert!(has_f_sequence, "Should extract sequence for variable f");
    }

    #[test]
    fn test_bigram_counter() {
        let mut sequences = HashMap::new();
        sequences.insert(
            "func:f".to_string(),
            vec!["open".to_string(), "read".to_string(), "close".to_string()],
        );

        let mut counter = BigramCounter::new();
        counter.add_sequences(&sequences, "test.py");

        assert_eq!(
            counter
                .counts
                .get(&("open".to_string(), "read".to_string())),
            Some(&1)
        );
        assert_eq!(
            counter
                .counts
                .get(&("read".to_string(), "close".to_string())),
            Some(&1)
        );
    }

    #[test]
    fn test_mine_bigrams_filter() {
        let mut sequences = HashMap::new();
        sequences.insert(
            "func:f".to_string(),
            vec!["open".to_string(), "read".to_string(), "close".to_string()],
        );

        let args = TemporalArgs {
            path: PathBuf::new(),
            min_support: 1,
            min_confidence: 0.0,
            query: None,
            source_lang: "python".to_string(),
            max_files: 1000,
            include_trigrams: false,
            include_examples: 3,
            output_format: OutputFormat::Json,
            timeout: 60,
            project_root: None,
            lang: None,
        };

        let (_, constraints) = mine_bigrams(&sequences, "test.py", &args);

        assert!(!constraints.is_empty(), "Should find bigram constraints");
    }

    #[test]
    fn test_filter_by_query() {
        let constraints = vec![
            TemporalConstraint {
                before: "open".to_string(),
                after: "read".to_string(),
                support: 5,
                confidence: 0.8,
                examples: vec![],
            },
            TemporalConstraint {
                before: "acquire".to_string(),
                after: "release".to_string(),
                support: 3,
                confidence: 0.9,
                examples: vec![],
            },
        ];

        let filtered = filter_by_query(constraints, "open");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].before, "open");
    }

    #[test]
    fn test_mine_trigrams_limit() {
        // Create sequences that would generate many trigrams
        let mut sequences = HashMap::new();
        let calls: Vec<String> = (0..100).map(|i| format!("method{}", i)).collect();
        sequences.insert("func:obj".to_string(), calls);

        let args = TemporalArgs {
            path: PathBuf::new(),
            min_support: 1,
            min_confidence: 0.0,
            query: None,
            source_lang: "python".to_string(),
            max_files: 1000,
            include_trigrams: true,
            include_examples: 3,
            output_format: OutputFormat::Json,
            timeout: 60,
            project_root: None,
            lang: None,
        };

        let trigrams = mine_trigrams(&sequences, &args);

        // Should respect MAX_TRIGRAMS limit
        assert!(trigrams.len() <= MAX_TRIGRAMS);
    }

    #[test]
    fn test_format_temporal_text() {
        let report = TemporalReport {
            constraints: vec![TemporalConstraint {
                before: "open".to_string(),
                after: "close".to_string(),
                support: 10,
                confidence: 0.95,
                examples: vec![TemporalExample {
                    file: "test.py".to_string(),
                    line: 5,
                }],
            }],
            trigrams: vec![],
            metadata: TemporalMetadata {
                files_analyzed: 1,
                sequences_extracted: 5,
                min_support: 2,
                min_confidence: 0.5,
            },
        };

        let text = format_temporal_text(&report);
        assert!(text.contains("open -> close"));
        assert!(text.contains("support: 10"));
        assert!(text.contains("confidence: 0.95"));
    }

    #[test]
    fn test_temporal_args_lang_flag() {
        use tldr_core::types::Language;

        // Verify TemporalArgs has a lang field of type Option<Language>
        let args = TemporalArgs {
            path: PathBuf::from("src/"),
            min_support: 2,
            min_confidence: 0.5,
            query: None,
            source_lang: "python".to_string(),
            max_files: 1000,
            include_trigrams: false,
            include_examples: 3,
            output_format: OutputFormat::Json,
            timeout: 60,
            project_root: None,
            lang: Some(Language::Python),
        };
        assert_eq!(args.lang, Some(Language::Python));

        // Also test None case (auto-detect)
        let args_auto = TemporalArgs {
            path: PathBuf::from("src/"),
            min_support: 2,
            min_confidence: 0.5,
            query: None,
            source_lang: "python".to_string(),
            max_files: 1000,
            include_trigrams: false,
            include_examples: 3,
            output_format: OutputFormat::Json,
            timeout: 60,
            project_root: None,
            lang: None,
        };
        assert_eq!(args_auto.lang, None);
    }
}
