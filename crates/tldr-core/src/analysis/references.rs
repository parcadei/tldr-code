//! Reference Finding Core Types and Functions
//!
//! This module provides reference finding for the `references` CLI command.
//!
//! # Type Overview
//!
//! - [`ReferencesReport`]: Complete reference finding report
//! - [`Reference`]: A single reference to a symbol
//! - [`Definition`]: Location where symbol is defined
//! - [`DefinitionKind`]: Kind of definition (function, class, variable, etc.)
//! - [`ReferenceKind`]: Kind of reference (call, read, write, import, type)
//! - [`SearchScope`]: Search scope for reference finding
//! - [`ReferenceStats`]: Search statistics
//! - [`ReferencesOptions`]: Configuration for reference finding
//! - [`TextCandidate`]: Candidate match from text search (Phase 9)
//!
//! # Risk Mitigations
//!
//! - S7-R17: Reference context truncation - limit context to 200 chars
//! - S7-R38: Line numbers - ensure 1-indexed throughout
//! - S7-R9: Memory usage - read one file at a time, don't load all (Phase 9)
//! - S7-R10: Regex compilation per file - compile once, reuse (Phase 9)
//! - S7-R4: Unicode position mapping - use byte offsets consistently (Phase 9, 10)
//! - S7-R5: Method call classification - check grandparent for call expression (Phase 10)
//! - S7-R6: Multi-match same line - verify each match independently (Phase 10)
//! - S7-R12: Re-parsing files - group candidates by file, parse once per file (Phase 10)
//! - S7-R22: f-string interpolation - handle formatted_string AST node (Phase 10)
//! - S7-R48: String matches - check AST node type is identifier, not string_literal (Phase 10)
//!
//! # References
//!
//! - Spec: session7-spec.md section 2.2 (Type Definitions)
//! - Phased plan: session7-phased-plan.yaml Phase 8, 9, 10

use crate::walker::walk_project;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tree_sitter::Node;

use crate::ast::parser::parse_file;
use crate::security::ast_utils;
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Core Types
// =============================================================================

/// Maximum context line length before truncation (S7-R17)
const MAX_CONTEXT_LENGTH: usize = 200;

/// Complete reference finding report
///
/// Contains all information about references to a symbol including
/// the definition location, all references found, and search statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReferencesReport {
    /// Symbol that was searched for
    pub symbol: String,

    /// Definition location (if found)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<Definition>,

    /// All references found
    pub references: Vec<Reference>,

    /// Total number of references
    pub total_references: usize,

    /// Search scope used
    pub search_scope: SearchScope,

    /// Search statistics
    pub stats: ReferenceStats,
}

impl ReferencesReport {
    /// Create a new empty report for a symbol
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            ..Default::default()
        }
    }

    /// Create a report with no matches found
    pub fn no_matches(symbol: String, scope: SearchScope, stats: ReferenceStats) -> Self {
        Self {
            symbol,
            definition: None,
            references: Vec::new(),
            total_references: 0,
            search_scope: scope,
            stats,
        }
    }
}

/// Location where symbol is defined
///
/// Contains file path, position (1-indexed), kind, and optional signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definition {
    /// File containing the definition
    pub file: PathBuf,

    /// Line number (1-indexed, S7-R38)
    pub line: usize,

    /// Column number (1-indexed, S7-R38)
    pub column: usize,

    /// Kind of definition
    pub kind: DefinitionKind,

    /// Function/class signature if applicable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl Definition {
    /// Create a new definition with minimal information
    pub fn new(file: PathBuf, line: usize, column: usize, kind: DefinitionKind) -> Self {
        Self {
            file,
            line,
            column,
            kind,
            signature: None,
        }
    }

    /// Create a definition with signature
    pub fn with_signature(
        file: PathBuf,
        line: usize,
        column: usize,
        kind: DefinitionKind,
        signature: String,
    ) -> Self {
        Self {
            file,
            line,
            column,
            kind,
            signature: Some(signature),
        }
    }
}

/// Kind of definition
///
/// Categorizes what type of construct the definition is.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum DefinitionKind {
    /// Function definition
    Function,

    /// Class definition
    Class,

    /// Variable definition
    Variable,

    /// Constant definition
    Constant,

    /// Type alias or type definition
    Type,

    /// Module definition
    Module,

    /// Method definition (function in a class)
    Method,

    /// Property or field definition
    Property,

    /// Unknown or other kind
    #[default]
    Other,
}

impl DefinitionKind {
    /// Get a human-readable string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            DefinitionKind::Function => "function",
            DefinitionKind::Class => "class",
            DefinitionKind::Variable => "variable",
            DefinitionKind::Constant => "constant",
            DefinitionKind::Type => "type",
            DefinitionKind::Module => "module",
            DefinitionKind::Method => "method",
            DefinitionKind::Property => "property",
            DefinitionKind::Other => "other",
        }
    }
}

impl std::fmt::Display for DefinitionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A single reference to the symbol
///
/// Contains file path, position (1-indexed), kind, and context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    /// File containing the reference
    pub file: PathBuf,

    /// Line number (1-indexed, S7-R38)
    pub line: usize,

    /// Column number (1-indexed, S7-R38)
    pub column: usize,

    /// Kind of reference
    pub kind: ReferenceKind,

    /// Line of code containing the reference (context)
    /// Truncated to MAX_CONTEXT_LENGTH (S7-R17)
    pub context: String,

    /// Confidence of this being a true reference (0.0 - 1.0)
    /// 1.0 = verified by AST, lower = text match only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,

    /// End column for highlighting (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<usize>,
}

impl Reference {
    /// Create a new reference with minimal information
    pub fn new(
        file: PathBuf,
        line: usize,
        column: usize,
        kind: ReferenceKind,
        context: String,
    ) -> Self {
        Self {
            file,
            line,
            column,
            kind,
            context: truncate_context(context),
            confidence: None,
            end_column: None,
        }
    }

    /// Create a reference with full information
    pub fn with_details(
        file: PathBuf,
        line: usize,
        column: usize,
        end_column: usize,
        kind: ReferenceKind,
        context: String,
        confidence: f64,
    ) -> Self {
        Self {
            file,
            line,
            column,
            kind,
            context: truncate_context(context),
            confidence: Some(confidence),
            end_column: Some(end_column),
        }
    }

    /// Create a reference verified by AST (confidence = 1.0)
    pub fn verified(
        file: PathBuf,
        line: usize,
        column: usize,
        kind: ReferenceKind,
        context: String,
    ) -> Self {
        Self {
            file,
            line,
            column,
            kind,
            context: truncate_context(context),
            confidence: Some(1.0),
            end_column: None,
        }
    }
}

/// Truncate context to MAX_CONTEXT_LENGTH (S7-R17)
fn truncate_context(context: String) -> String {
    if context.len() > MAX_CONTEXT_LENGTH {
        let truncated: String = context.chars().take(MAX_CONTEXT_LENGTH - 3).collect();
        format!("{}...", truncated)
    } else {
        context
    }
}

/// Kind of reference
///
/// Categorizes how the symbol is being used at this reference site.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceKind {
    /// Function/method invocation
    Call,

    /// Variable read
    Read,

    /// Variable assignment/write
    Write,

    /// Import statement
    Import,

    /// Type annotation
    Type,

    /// Definition site itself
    Definition,

    /// Unknown or other kind
    #[default]
    Other,
}

impl ReferenceKind {
    /// Get a human-readable string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Call => "call",
            ReferenceKind::Read => "read",
            ReferenceKind::Write => "write",
            ReferenceKind::Import => "import",
            ReferenceKind::Type => "type",
            ReferenceKind::Definition => "definition",
            ReferenceKind::Other => "other",
        }
    }

    /// Parse from string (case-insensitive)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "call" => Some(ReferenceKind::Call),
            "read" => Some(ReferenceKind::Read),
            "write" => Some(ReferenceKind::Write),
            "import" => Some(ReferenceKind::Import),
            "type" => Some(ReferenceKind::Type),
            "definition" => Some(ReferenceKind::Definition),
            "other" => Some(ReferenceKind::Other),
            _ => None,
        }
    }
}

impl std::fmt::Display for ReferenceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Search scope for reference finding
///
/// Controls how much of the workspace is searched for references.
/// Used for optimization based on symbol visibility.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchScope {
    /// Search only within the current function (local variables)
    Local,

    /// Search only within the current file (private items)
    File,

    /// Search entire workspace (public items)
    #[default]
    Workspace,
}

impl SearchScope {
    /// Get a human-readable string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            SearchScope::Local => "local",
            SearchScope::File => "file",
            SearchScope::Workspace => "workspace",
        }
    }

    /// Parse from string (case-insensitive)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "local" => Some(SearchScope::Local),
            "file" => Some(SearchScope::File),
            "workspace" => Some(SearchScope::Workspace),
            _ => None,
        }
    }
}

impl std::fmt::Display for SearchScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Search statistics
///
/// Provides information about the search process for debugging
/// and performance analysis.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReferenceStats {
    /// Number of files searched
    pub files_searched: usize,

    /// Number of text match candidates found (before AST verification)
    pub candidates_found: usize,

    /// Number of verified references (after AST pruning)
    pub verified_references: usize,

    /// Search time in milliseconds
    pub search_time_ms: u64,
}

impl ReferenceStats {
    /// Create stats with counts
    pub fn new(files_searched: usize, candidates_found: usize, verified_references: usize) -> Self {
        Self {
            files_searched,
            candidates_found,
            verified_references,
            search_time_ms: 0,
        }
    }

    /// Set search time
    pub fn with_time(mut self, time_ms: u64) -> Self {
        self.search_time_ms = time_ms;
        self
    }
}

/// Options for reference finding
///
/// Configuration options that control the behavior of reference finding.
#[derive(Debug, Clone, Default)]
pub struct ReferencesOptions {
    /// Include the definition in results
    pub include_definition: bool,

    /// Filter by reference kinds (None = all kinds)
    pub kinds: Option<Vec<ReferenceKind>>,

    /// Search scope (None = infer from symbol)
    pub scope: SearchScope,

    /// Language to analyze (None = auto-detect)
    pub language: Option<String>,

    /// Maximum results to return
    pub limit: Option<usize>,

    /// File containing the symbol definition (helps scope optimization)
    pub definition_file: Option<PathBuf>,

    /// Number of context lines to include
    pub context_lines: usize,
}

impl ReferencesOptions {
    /// Create new default options
    pub fn new() -> Self {
        Self::default()
    }

    /// Include definition in results
    pub fn with_definition(mut self) -> Self {
        self.include_definition = true;
        self
    }

    /// Filter by specific reference kinds
    pub fn with_kinds(mut self, kinds: Vec<ReferenceKind>) -> Self {
        self.kinds = Some(kinds);
        self
    }

    /// Set search scope
    pub fn with_scope(mut self, scope: SearchScope) -> Self {
        self.scope = scope;
        self
    }

    /// Set language
    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
        self
    }

    /// Set maximum results
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set definition file for scope optimization
    pub fn with_definition_file(mut self, file: PathBuf) -> Self {
        self.definition_file = Some(file);
        self
    }

    /// Set context lines
    pub fn with_context_lines(mut self, lines: usize) -> Self {
        self.context_lines = lines;
        self
    }
}

// =============================================================================
// Phase 9: Text Search for Reference Candidates
// =============================================================================

/// Candidate match from text search (before AST verification)
///
/// These are potential references found by text search that need
/// to be verified using AST parsing in Phase 10.
#[derive(Debug, Clone)]
pub struct TextCandidate {
    /// File containing the candidate match
    pub file: PathBuf,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// End column (1-indexed)
    pub end_column: usize,
    /// The full line text containing the match
    pub line_text: String,
}

impl TextCandidate {
    /// Create a new text candidate
    pub fn new(
        file: PathBuf,
        line: usize,
        column: usize,
        end_column: usize,
        line_text: String,
    ) -> Self {
        Self {
            file,
            line,
            column,
            end_column,
            line_text,
        }
    }
}

/// Find all text occurrences of a symbol (fast, overapproximating)
///
/// This is the first step in the rust-analyzer pattern:
/// "text search to find superset, then prune with semantic resolve"
///
/// # Arguments
///
/// * `symbol` - The symbol name to search for
/// * `root` - The root directory to search in
/// * `language` - Optional language filter (e.g., "python", "typescript")
///
/// # Returns
///
/// A vector of TextCandidate structs representing potential matches.
/// These need to be verified using AST parsing in Phase 10.
///
/// # Risk Mitigations
///
/// - S7-R9: Memory usage - read one file at a time, don't load all
/// - S7-R10: Regex compilation per file - compile once, reuse
pub fn find_text_candidates(
    symbol: &str,
    root: &Path,
    language: Option<&str>,
) -> TldrResult<Vec<TextCandidate>> {
    let mut candidates = Vec::new();

    // Build regex with word boundaries to avoid partial matches
    // e.g., searching for "get" shouldn't match "forget"
    // S7-R10: Compile regex once, reuse for all files
    let pattern = format!(r"\b{}\b", regex::escape(symbol));
    let re = Regex::new(&pattern)?;

    // Walk directory, filter by language extension
    // S7-R9: Files are read one at a time to bound memory usage
    // Skips node_modules, target, dist, hidden dirs via the shared walker
    for entry in walk_project(root)
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| is_source_file(e.path(), language))
    {
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue, // Skip files we can't read
        };

        for (line_num, line) in content.lines().enumerate() {
            // Skip comment lines (basic heuristic for common cases)
            if is_comment_line(line, language) {
                continue;
            }

            for mat in re.find_iter(line) {
                candidates.push(TextCandidate {
                    file: entry.path().to_path_buf(),
                    line: line_num + 1,        // 1-indexed (S7-R38)
                    column: mat.start() + 1,   // 1-indexed (S7-R38)
                    end_column: mat.end() + 1, // 1-indexed
                    line_text: line.to_string(),
                });
            }
        }
    }

    Ok(candidates)
}

/// Check if file is a source file for the given language.
///
/// Uses `Language::from_path` to support all 18 languages (Python, TypeScript,
/// JavaScript, Go, Rust, Java, C, C++, Ruby, Kotlin, Swift, C#, Scala, PHP,
/// Lua, Luau, Elixir, OCaml).
fn is_source_file(path: &Path, language: Option<&str>) -> bool {
    match Language::from_path(path) {
        Some(detected) => {
            match language {
                None => true, // No filter — accept any supported language
                Some(lang) => {
                    // Check if detected language matches the requested filter
                    let normalized = lang.to_lowercase();
                    match normalized.as_str() {
                        "python" => matches!(detected, Language::Python),
                        "typescript" => matches!(detected, Language::TypeScript),
                        "javascript" => {
                            matches!(detected, Language::JavaScript | Language::TypeScript)
                        }
                        "go" => matches!(detected, Language::Go),
                        "rust" => matches!(detected, Language::Rust),
                        "java" => matches!(detected, Language::Java),
                        "c" => matches!(detected, Language::C),
                        "cpp" => matches!(detected, Language::Cpp),
                        "csharp" => matches!(detected, Language::CSharp),
                        "kotlin" => matches!(detected, Language::Kotlin),
                        "scala" => matches!(detected, Language::Scala),
                        "swift" => matches!(detected, Language::Swift),
                        "php" => matches!(detected, Language::Php),
                        "ruby" => matches!(detected, Language::Ruby),
                        "lua" => matches!(detected, Language::Lua),
                        "luau" => matches!(detected, Language::Luau),
                        "elixir" => matches!(detected, Language::Elixir),
                        "ocaml" => matches!(detected, Language::Ocaml),
                        _ => false,
                    }
                }
            }
        }
        None => false, // Not a recognized source file
    }
}

/// Basic check if line is a comment (for filtering obvious false positives)
///
/// This is a heuristic to reduce noise from text search. Full comment
/// detection is done in AST verification (Phase 10).
fn is_comment_line(line: &str, language: Option<&str>) -> bool {
    let trimmed = line.trim();

    match language {
        // Hash-style comments: Python, Ruby, PHP, Elixir
        Some("python") | Some("ruby") | Some("elixir") => trimmed.starts_with('#'),
        // PHP also supports // and /* */
        Some("php") => {
            trimmed.starts_with("//")
                || trimmed.starts_with('#')
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
        }
        // C-style comments: TypeScript, JavaScript, Go, Rust, Java, C, C++, C#, Kotlin, Scala, Swift
        Some("typescript") | Some("javascript") | Some("go") | Some("rust") | Some("java")
        | Some("c") | Some("cpp") | Some("csharp") | Some("kotlin") | Some("scala")
        | Some("swift") => {
            trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*')
        }
        // Lua / Luau: -- comments
        Some("lua") | Some("luau") => trimmed.starts_with("--"),
        // OCaml: (* comments *)
        Some("ocaml") => trimmed.starts_with("(*"),
        // Default: handle all common comment patterns
        None => {
            trimmed.starts_with("//")
                || trimmed.starts_with('#')
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
                || trimmed.starts_with("--")
                || trimmed.starts_with("(*")
        }
        _ => false,
    }
}

/// Count source files in a directory for statistics
fn count_source_files(root: &Path, language: Option<&str>) -> usize {
    walk_project(root)
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| is_source_file(e.path(), language))
        .count()
}

// =============================================================================
// Phase 10: AST Verification and Reference Kind Classification
// =============================================================================

/// Verified reference from AST analysis
///
/// Contains the reference kind determined by AST context and confidence score.
#[derive(Debug, Clone)]
pub struct VerifiedReference {
    /// The determined reference kind
    pub kind: ReferenceKind,
    /// Confidence score (1.0 = fully verified by AST)
    pub confidence: f64,
    /// Whether this is a valid reference (not in string/comment)
    pub is_valid: bool,
}

/// Verify text candidates using AST parsing
///
/// Groups candidates by file, parses each file once, and verifies each
/// candidate against the AST. Returns only valid references with
/// correct kind classification.
///
/// # Risk Mitigations
///
/// - S7-R12: Re-parsing files - group candidates by file, parse once per file
/// - S7-R4: Unicode position mapping - use byte offsets consistently
/// - S7-R48: String matches - check AST node type is identifier, not string_literal
pub fn verify_candidates_with_ast(
    candidates: &[TextCandidate],
    symbol: &str,
    _language_str: Option<&str>,
) -> Vec<(TextCandidate, VerifiedReference)> {
    let mut verified = Vec::new();

    // S7-R12: Group candidates by file to parse each file only once
    let mut by_file: HashMap<PathBuf, Vec<&TextCandidate>> = HashMap::new();
    for candidate in candidates {
        by_file
            .entry(candidate.file.clone())
            .or_default()
            .push(candidate);
    }

    // Process each file
    for (file_path, file_candidates) in by_file {
        // Try to parse the file
        let parsed = match parse_file(&file_path) {
            Ok(p) => p,
            Err(_) => {
                // Parse failed, include candidates as unverified
                for candidate in file_candidates {
                    verified.push((
                        candidate.clone(),
                        VerifiedReference {
                            kind: ReferenceKind::Other,
                            confidence: 0.5, // Text match only
                            is_valid: true,  // Assume valid if we can't verify
                        },
                    ));
                }
                continue;
            }
        };

        let (tree, source, lang) = parsed;
        let source_bytes = source.as_bytes();

        // Verify each candidate in this file
        for candidate in file_candidates {
            if let Some(verified_ref) =
                verify_single_candidate(candidate, symbol, &tree, source_bytes, lang)
            {
                if verified_ref.is_valid {
                    verified.push((candidate.clone(), verified_ref));
                }
                // If not valid (e.g., in string), skip this candidate
            }
        }
    }

    verified
}

/// Verify a single candidate against the AST
///
/// Returns Some(VerifiedReference) if the candidate can be verified,
/// None if verification fails (should not happen normally).
fn verify_single_candidate(
    candidate: &TextCandidate,
    symbol: &str,
    tree: &tree_sitter::Tree,
    source: &[u8],
    language: Language,
) -> Option<VerifiedReference> {
    // Convert 1-indexed line/column to tree-sitter Point (0-indexed)
    let point = tree_sitter::Point::new(candidate.line - 1, candidate.column - 1);

    // Find the smallest node containing this position
    let node = tree.root_node().descendant_for_point_range(point, point)?;

    // Get the text of this node
    let node_text = node.utf8_text(source).ok()?;

    // S7-R48: Check if the node text matches the symbol exactly
    // This filters out partial matches
    if node_text != symbol {
        // The node text doesn't match - might be part of a larger identifier
        // or the position is off. Try to find exact match nearby.
        return find_exact_match_node(&node, symbol, source, language);
    }

    // Check if this node is in an invalid context (string, comment)
    if is_in_invalid_context(&node, language) {
        return Some(VerifiedReference {
            kind: ReferenceKind::Other,
            confidence: 1.0,
            is_valid: false, // In string/comment, not a real reference
        });
    }

    // Classify the reference kind based on AST context
    let kind = classify_reference_kind(&node, source, language);

    Some(VerifiedReference {
        kind,
        confidence: 1.0, // Fully verified by AST
        is_valid: true,
    })
}

/// Try to find the exact match node when position lookup returns a parent node
fn find_exact_match_node(
    node: &Node,
    symbol: &str,
    source: &[u8],
    language: Language,
) -> Option<VerifiedReference> {
    // Check this node and its descendants for exact match
    let mut cursor = node.walk();

    // Check children
    for child in node.children(&mut cursor) {
        if let Ok(text) = child.utf8_text(source) {
            if text == symbol {
                if is_in_invalid_context(&child, language) {
                    return Some(VerifiedReference {
                        kind: ReferenceKind::Other,
                        confidence: 1.0,
                        is_valid: false,
                    });
                }
                let kind = classify_reference_kind(&child, source, language);
                return Some(VerifiedReference {
                    kind,
                    confidence: 1.0,
                    is_valid: true,
                });
            }
        }
    }

    // If no exact match found, return unverified
    Some(VerifiedReference {
        kind: ReferenceKind::Other,
        confidence: 0.5,
        is_valid: true,
    })
}

/// Check if a node is in an invalid context (string literal, comment)
///
/// # Risk Mitigations
///
/// - S7-R48: String matches - check AST node type is identifier, not string_literal
/// - S7-R22: f-string interpolation - handle formatted_string AST node
fn is_in_invalid_context(node: &Node, language: Language) -> bool {
    // Check the node type itself
    let node_kind = node.kind();

    // Common string/comment node types across languages
    let invalid_self_kinds = [
        "string",
        "string_literal",
        "string_content",
        "template_string",
        "raw_string_literal",
        "comment",
        "line_comment",
        "block_comment",
        "heredoc_content",
    ];

    if invalid_self_kinds.contains(&node_kind) {
        return true;
    }

    // Walk up the tree to check ancestors
    let mut current = node.parent();
    while let Some(ancestor) = current {
        let kind = ancestor.kind();

        match language {
            Language::Python => {
                // Python string types
                if matches!(
                    kind,
                    "string" | "string_content" | "concatenated_string" | "comment"
                ) {
                    return true;
                }
                // S7-R22: f-string interpolation is OK - the code inside is real
                // formatted_string contains format_expression which should be verified
                if kind == "string" {
                    // Check if we're NOT inside a format_expression
                    if !is_inside_format_expression(node) {
                        return true;
                    }
                }
            }
            Language::TypeScript | Language::JavaScript => {
                if matches!(
                    kind,
                    "string" | "template_string" | "string_fragment" | "comment"
                ) {
                    // Template literals with ${} expressions are OK
                    if kind == "template_string" && has_template_substitution(&ancestor) {
                        // Check if we're inside the substitution
                        if is_inside_template_substitution(node) {
                            return false;
                        }
                    }
                    return true;
                }
            }
            Language::Go => {
                if matches!(
                    kind,
                    "raw_string_literal" | "interpreted_string_literal" | "comment"
                ) {
                    return true;
                }
            }
            Language::Rust => {
                if matches!(
                    kind,
                    "string_literal" | "raw_string_literal" | "line_comment" | "block_comment"
                ) {
                    return true;
                }
            }
            _ => {
                // Generic check for other languages
                if kind.contains("string") || kind.contains("comment") {
                    return true;
                }
            }
        }

        current = ancestor.parent();
    }

    false
}

/// Check if node is inside a Python f-string format expression
fn is_inside_format_expression(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "interpolation" || ancestor.kind() == "format_expression" {
            return true;
        }
        if ancestor.kind() == "string" || ancestor.kind() == "concatenated_string" {
            return false; // Hit string boundary without finding format_expression
        }
        current = ancestor.parent();
    }
    false
}

/// Check if a template string has substitutions
fn has_template_substitution(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "template_substitution" {
            return true;
        }
    }
    false
}

/// Check if node is inside a template substitution (${...})
fn is_inside_template_substitution(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "template_substitution" {
            return true;
        }
        if ancestor.kind() == "template_string" {
            return false;
        }
        current = ancestor.parent();
    }
    false
}

/// Classify reference kind based on AST context
///
/// Examines the parent and grandparent nodes to determine how the symbol
/// is being used.
///
/// # Risk Mitigations
///
/// - S7-R5: Method call classification - check grandparent for call expression
pub fn classify_reference_kind(node: &Node, source: &[u8], language: Language) -> ReferenceKind {
    let parent = match node.parent() {
        Some(p) => p,
        None => return ReferenceKind::Other,
    };

    match language {
        Language::Python => classify_python_reference(node, &parent, source),
        Language::TypeScript | Language::JavaScript => {
            classify_typescript_reference(node, &parent, source)
        }
        Language::Go => classify_go_reference(node, &parent, source),
        Language::Rust => classify_rust_reference(node, &parent, source),
        Language::Java => classify_java_reference(node, &parent, source),
        Language::C => classify_c_reference(node, &parent, source),
        Language::Cpp => classify_cpp_reference(node, &parent, source),
        Language::CSharp => classify_csharp_reference(node, &parent, source),
        Language::Kotlin => classify_kotlin_reference(node, &parent, source),
        Language::Scala => classify_scala_reference(node, &parent, source),
        Language::Swift => classify_swift_reference(node, &parent, source),
        Language::Php => classify_php_reference(node, &parent, source),
        Language::Ruby => classify_ruby_reference(node, &parent, source),
        Language::Lua => classify_lua_reference(node, &parent, source),
        Language::Luau => classify_luau_reference(node, &parent, source),
        Language::Elixir => classify_elixir_reference(node, &parent, source),
        Language::Ocaml => classify_ocaml_reference(node, &parent, source),
    }
}

/// Classify Python reference kind
fn classify_python_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Function/method call: func() or obj.method()
        "call" => ReferenceKind::Call,

        // S7-R5: Check if we're the function being called (not an argument)
        "argument_list" => {
            // We're an argument to a call, likely a Read
            ReferenceKind::Read
        }

        // Assignment: x = value
        "assignment" => {
            // Check if node is on LHS (target) or RHS (value)
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Augmented assignment: x += 1 (both read and write, report as Write)
        "augmented_assignment" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Import statements
        "import_statement" | "import_from_statement" | "dotted_name" | "aliased_import" => {
            // Check if we're actually in an import context
            let mut current = Some(*parent);
            while let Some(ancestor) = current {
                if ancestor.kind() == "import_statement"
                    || ancestor.kind() == "import_from_statement"
                {
                    return ReferenceKind::Import;
                }
                current = ancestor.parent();
            }
            ReferenceKind::Read
        }

        // Type annotations
        "type" | "annotation" | "subscript" => {
            // Check if this is in a type annotation context
            if is_type_context(parent) {
                return ReferenceKind::Type;
            }
            ReferenceKind::Read
        }

        // Function/class definition
        "function_definition" | "class_definition" => {
            // Check if this is the name being defined
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Parameter definition
        "parameter" | "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
            // The parameter name itself is a Definition
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            // Type annotation in parameter
            if let Some(type_node) = parent.child_by_field_name("type") {
                if node_contains(node, &type_node) {
                    return ReferenceKind::Type;
                }
            }
            ReferenceKind::Read
        }

        // For loop variable
        "for_statement" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Comprehension
        "for_in_clause" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Attribute access: obj.attr - we need to check grandparent for call
        "attribute" => {
            // S7-R5: Check grandparent for call expression
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call" {
                    // Check if the call's function is this attribute
                    if let Some(func) = grandparent.child_by_field_name("function") {
                        if func.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        // Default: assume it's a read
        _ => ReferenceKind::Read,
    }
}

/// Classify TypeScript/JavaScript reference kind
fn classify_typescript_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Function call
        "call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        // Assignment
        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Variable declarator: let x = ...
        "variable_declarator" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Import statements
        "import_specifier" | "import_clause" | "namespace_import" => ReferenceKind::Import,

        // Type annotations
        "type_annotation" | "type_identifier" | "generic_type" | "type_arguments" => {
            ReferenceKind::Type
        }

        // Function/class declaration
        "function_declaration" | "class_declaration" | "method_definition" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Member expression: obj.prop - check grandparent for call
        "member_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(func) = grandparent.child_by_field_name("function") {
                        if func.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Classify Go reference kind
fn classify_go_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Function call
        "call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        // Assignment
        "assignment_statement" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Short variable declaration: x := value
        "short_var_declaration" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Import
        "import_spec" => ReferenceKind::Import,

        // Type reference
        "type_identifier" | "qualified_type" | "pointer_type" | "slice_type" | "array_type" => {
            ReferenceKind::Type
        }

        // Function declaration
        "function_declaration" | "method_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Selector expression: pkg.Func or obj.Method
        "selector_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(func) = grandparent.child_by_field_name("function") {
                        if func.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Classify Rust reference kind
fn classify_rust_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Function call
        "call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        // Assignment (let with value or reassignment)
        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Let binding
        "let_declaration" => {
            if let Some(pattern) = parent.child_by_field_name("pattern") {
                if node_contains(node, &pattern) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Use declaration (imports)
        "use_declaration" | "use_clause" | "scoped_identifier" => {
            // Check if we're in a use context
            let mut current = Some(*parent);
            while let Some(ancestor) = current {
                if ancestor.kind() == "use_declaration" {
                    return ReferenceKind::Import;
                }
                current = ancestor.parent();
            }
            ReferenceKind::Read
        }

        // Type references
        "type_identifier" | "generic_type" | "scoped_type_identifier" | "reference_type" => {
            ReferenceKind::Type
        }

        // Function definition
        "function_item" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Method call: obj.method()
        "field_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(func) = grandparent.child_by_field_name("function") {
                        if func.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Return the first named child of `node` whose kind is in `kinds`.
fn first_named_child_of_kind<'a>(node: &Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if kinds.contains(&child.kind()) {
                return Some(child);
            }
        }
    }
    None
}

/// Classify Java reference kind.
///
/// Grammar reference: tree-sitter-java. `method_declaration` /
/// `constructor_declaration` hold the method name as their first `identifier`
/// child (return types are `type_identifier` / `void_type`, so not confused).
/// Call sites use `method_invocation` whose callee identifier is a direct
/// `identifier` child (for `obj.method()` the callee is under a `field_access`
/// parent — we classify on the direct identifier attached to the invocation).
fn classify_java_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Direct call site: greet()
        "method_invocation" => {
            // The callee identifier appears as the "name" field or the first
            // identifier child of the invocation. Arguments are in argument_list,
            // so an identifier that is NOT inside argument_list is the callee.
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Call;
                }
            }
            // Fallback: if no "name" field, the first identifier child is the callee.
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        // Method / constructor definition: first identifier child is the name.
        "method_declaration" | "constructor_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Class / interface / enum declarations: name field
        "class_declaration" | "interface_declaration" | "enum_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Assignment: lhs is Write
        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Imports
        "import_declaration" => ReferenceKind::Import,

        // Field access / scoped access: if grandparent is method_invocation
        // and this field_access is the callee, it's a Call.
        "field_access" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "method_invocation" {
                    if let Some(name) = grandparent.child_by_field_name("name") {
                        if node_contains(node, &name) {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        // Type references
        "type_identifier" | "generic_type" | "array_type" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify C reference kind.
///
/// Grammar reference: tree-sitter-c. `function_definition` contains a
/// `function_declarator` whose first `identifier` child is the function name.
/// `call_expression` has the callee as its first child (an `identifier` for
/// direct calls).
fn classify_c_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        // Direct call: greet()
        "call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            // Fallback: first child of call_expression is the callee
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        // Function name inside function_declarator: definition
        "function_declarator" => {
            // The first identifier child is the function name.
            if let Some(decl) = parent.child_by_field_name("declarator") {
                if node.id() == decl.id() {
                    return classify_c_declarator_as_definition(parent);
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    // Check whether this declarator is inside a function_definition
                    // — if so, this is a Definition. Otherwise it's still the name
                    // of a declared function (forward declaration), also Definition.
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Parameter declaration name
        "parameter_declaration" => {
            if let Some(decl) = parent.child_by_field_name("declarator") {
                if node_contains(node, &decl) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Initializers / assignments: lhs of assignment_expression is a Write.
        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        // Preprocessor includes
        "preproc_include" => ReferenceKind::Import,

        // Field access used as callee
        "field_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        // Type references
        "type_identifier" | "sized_type_specifier" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Helper: a function_declarator within a function_definition/declaration
/// always names a function-kind entity (not a variable), so its identifier
/// is a Definition. Kept as a tiny helper to match the established pattern.
fn classify_c_declarator_as_definition(_declarator: &Node) -> ReferenceKind {
    ReferenceKind::Definition
}

/// Classify C++ reference kind.
///
/// Grammar reference: tree-sitter-cpp. Structure mirrors C for the
/// must-have cases (`function_definition` → `function_declarator` →
/// `identifier` for the name; `call_expression` with direct `identifier`
/// callee). Adds handling for qualified identifiers (ns::func()) and
/// field expressions for method calls.
fn classify_cpp_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_declarator" => {
            if let Some(first_id) = first_named_child_of_kind(
                parent,
                &[
                    "identifier",
                    "field_identifier",
                    "qualified_identifier",
                    "destructor_name",
                ],
            ) {
                if node.id() == first_id.id() || node_contains(node, &first_id) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        // Qualified identifier: ns::func — if part of a call, it's a Call
        "qualified_identifier" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
                // Qualified identifier inside a declarator is part of a Definition.
                if grandparent.kind() == "function_declarator" {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "field_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "parameter_declaration" => {
            if let Some(decl) = parent.child_by_field_name("declarator") {
                if node_contains(node, &decl) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "preproc_include" => ReferenceKind::Import,

        "type_identifier" | "template_type" | "sized_type_specifier" => ReferenceKind::Type,

        "class_specifier" | "struct_specifier" | "namespace_definition" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node_contains(node, &name) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Classify C# reference kind.
///
/// Grammar reference: tree-sitter-c-sharp. `method_declaration`,
/// `constructor_declaration`, and `class_declaration` expose a "name" field
/// (and also have the name as a direct identifier child). Call sites are
/// `invocation_expression` — the callee is a direct `identifier` child or
/// a `member_access_expression`.
fn classify_csharp_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "invocation_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "method_declaration" | "constructor_declaration" | "local_function_statement" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            // Fallback: for method_declaration the name is the second identifier
            // (first is return type when type_identifier isn't emitted). Safely
            // take the first identifier — this is the established fallback in
            // the C# callgraph handler.
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "class_declaration"
        | "interface_declaration"
        | "struct_declaration"
        | "enum_declaration"
        | "record_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "member_access_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "invocation_expression" {
                    if let Some(func) = grandparent.child_by_field_name("function") {
                        if func.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "using_directive" | "namespace_declaration" => ReferenceKind::Import,

        "type_parameter" | "type_argument_list" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify Kotlin reference kind.
///
/// Grammar reference: tree-sitter-kotlin. `function_declaration` /
/// `class_declaration` have the name as a direct `identifier` child.
/// `call_expression` has the callee as a direct `identifier` or a
/// `navigation_expression` child.
fn classify_kotlin_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call_expression" => {
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() || node_contains(node, &first) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_declaration"
        | "class_declaration"
        | "object_declaration"
        | "property_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "navigation_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "assignment" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "import_header" | "import_list" => ReferenceKind::Import,

        "user_type" | "type_reference" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify Scala reference kind.
///
/// Grammar reference: tree-sitter-scala. `function_definition` /
/// `function_declaration` / `class_definition` / `object_definition` have the
/// name as a direct `identifier` or `type_identifier` child. Call sites are
/// `call_expression` with the callee as the first non-arguments child.
fn classify_scala_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call_expression" => {
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() || node_contains(node, &first) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_definition" | "function_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) =
                first_named_child_of_kind(parent, &["identifier", "type_identifier"])
            {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "class_definition" | "object_definition" | "trait_definition" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) =
                first_named_child_of_kind(parent, &["identifier", "type_identifier"])
            {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "field_expression" | "select_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "import_declaration" | "import_expression" | "import_selectors" => ReferenceKind::Import,

        "type_identifier" | "generic_type" | "compound_type" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify Swift reference kind.
///
/// Grammar reference: tree-sitter-swift. `function_declaration` /
/// `protocol_function_declaration` expose a "name" field (a
/// `simple_identifier`). `init_declaration` doesn't have a simple name but
/// its identifier site is still a Definition. `call_expression` has the
/// callable as its first child (usually `simple_identifier` or
/// `navigation_expression`).
fn classify_swift_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call_expression" => {
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() || node_contains(node, &first) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_declaration" | "protocol_function_declaration" | "init_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["simple_identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "class_declaration" | "protocol_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node_contains(node, &name) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "property_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node_contains(node, &name) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "navigation_expression" => {
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call_expression" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "assignment" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "import_declaration" => ReferenceKind::Import,

        "user_type" | "type_identifier" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify PHP reference kind.
///
/// Grammar reference: tree-sitter-php. `function_definition` and
/// `method_declaration` expose a "name" field (a `name` node). Call sites are
/// `function_call_expression` / `member_call_expression` /
/// `scoped_call_expression` — all with a "function"/"name" field.
fn classify_php_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "function_call_expression" => {
            if let Some(func) = parent.child_by_field_name("function") {
                if node_contains(node, &func) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "member_call_expression" | "scoped_call_expression" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node_contains(node, &name) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_definition" | "method_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "class_declaration" | "trait_declaration" | "interface_declaration" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "assignment_expression" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        "namespace_use_declaration" | "namespace_use_clause" | "namespace_name" => {
            ReferenceKind::Import
        }

        "named_type" | "primitive_type" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Classify Ruby reference kind.
///
/// Grammar reference: tree-sitter-ruby. `method` / `singleton_method` have
/// the name as a direct `identifier` child. Call sites are `call` nodes
/// where the callee identifier is also a direct `identifier` child (with
/// `argument_list` containing arguments).
fn classify_ruby_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call" => {
            // The callee is the identifier that is NOT inside argument_list and
            // NOT a receiver. Ruby's `call` can look like:
            //   call { identifier(callee), argument_list }
            //   call { identifier(receiver), ".", identifier(callee), argument_list }
            // The callee is the LAST identifier child before the argument_list
            // (or at the end if no arguments).
            if let Some(name) = parent.child_by_field_name("method") {
                if node_contains(node, &name) {
                    return ReferenceKind::Call;
                }
            }
            // Fallback: find the last identifier child before argument_list.
            let mut last_id: Option<Node> = None;
            for i in 0..parent.child_count() {
                if let Some(child) = parent.child(i) {
                    if child.kind() == "argument_list" {
                        break;
                    }
                    if child.kind() == "identifier" {
                        last_id = Some(child);
                    }
                }
            }
            if let Some(id) = last_id {
                if node.id() == id.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "method" | "singleton_method" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "class" | "module" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node_contains(node, &name) {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "assignment" | "operator_assignment" => {
            if let Some(left) = parent.child_by_field_name("left") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Classify Lua reference kind.
///
/// Grammar reference: tree-sitter-lua (nvim-treesitter). `function_declaration`
/// has the name as a direct `identifier` child (or `dot_index_expression` /
/// `method_index_expression`). Call sites are `function_call` with the callee
/// as the first child.
fn classify_lua_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    classify_lua_family_reference(node, parent)
}

/// Classify Luau reference kind.
///
/// Luau's grammar mirrors Lua's for the core call/definition nodes
/// (`function_call`, `function_declaration`, `identifier`). Share the impl.
fn classify_luau_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    classify_lua_family_reference(node, parent)
}

fn classify_lua_family_reference(node: &Node, parent: &Node) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "function_call" => {
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() || node_contains(node, &first) {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "function_declaration" | "local_function" | "function_definition_statement" => {
            if let Some(name) = parent.child_by_field_name("name") {
                if node.id() == name.id() {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "dot_index_expression" | "method_index_expression" => {
            // If we're the function name (last identifier) and grandparent is
            // function_call, it's a Call. If grandparent is function_declaration,
            // it's a Definition.
            if let Some(grandparent) = parent.parent() {
                match grandparent.kind() {
                    "function_call" => {
                        if let Some(first) = grandparent.child(0) {
                            if first.id() == parent.id() {
                                return ReferenceKind::Call;
                            }
                        }
                    }
                    "function_declaration" => {
                        return ReferenceKind::Definition;
                    }
                    _ => {}
                }
            }
            ReferenceKind::Read
        }

        "assignment_statement" => {
            if let Some(left) = parent.child_by_field_name("variables") {
                if node_contains(node, &left) {
                    return ReferenceKind::Write;
                }
            }
            // Fallback: check if the node is in the "variable_list" (first
            // named child) — that's the LHS in the nvim grammar.
            for i in 0..parent.child_count() {
                if let Some(child) = parent.child(i) {
                    if child.kind() == "variable_list" {
                        if node_contains(node, &child) {
                            return ReferenceKind::Write;
                        }
                        break;
                    }
                }
            }
            ReferenceKind::Read
        }

        "variable_declaration" => {
            // Local declarations: `local foo = ...` — the name is a Definition.
            ReferenceKind::Definition
        }

        _ => ReferenceKind::Read,
    }
}

/// Classify Elixir reference kind.
///
/// Elixir's grammar uses `call` nodes for nearly everything. The pattern for a
/// function definition is `call { identifier("def"|"defp"), arguments { call {
/// identifier(funcname), ... } } }`. So to detect Definition, we check: our
/// identifier is the first identifier child of a `call`, that call's parent is
/// `arguments`, and the argument's parent `call` has first identifier "def" /
/// "defp" / "defmacro" / "defmacrop".
///
/// Direct call sites: our identifier is the first identifier child of a `call`
/// node (not inside the def-pattern above). Dotted calls (`Mod.func`) have
/// parent `dot`.
fn classify_elixir_reference(node: &Node, parent: &Node, source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "call" => {
            // Is this the first identifier child of the call?
            if let Some(first_id) = first_named_child_of_kind(parent, &["identifier"]) {
                if node.id() == first_id.id() {
                    // Check if this call is inside `arguments` of a def-style call.
                    if is_elixir_def_call(parent, source) {
                        return ReferenceKind::Definition;
                    }
                    // Otherwise it's a plain function call.
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "dot" => {
            // Dotted call: App.greet — if the dot's parent is a call whose first
            // child is this dot, it's a Call.
            if let Some(grandparent) = parent.parent() {
                if grandparent.kind() == "call" {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            // Check we're the name side (usually the LAST identifier child of dot).
                            let mut last_id: Option<Node> = None;
                            for i in 0..parent.child_count() {
                                if let Some(child) = parent.child(i) {
                                    if child.kind() == "identifier" {
                                        last_id = Some(child);
                                    }
                                }
                            }
                            if let Some(id) = last_id {
                                if node.id() == id.id() {
                                    return ReferenceKind::Call;
                                }
                            }
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        _ => ReferenceKind::Read,
    }
}

/// Returns true if `call_node` is the inner `call` inside
/// `outer_call(def|defp|defmacro|defmacrop, arguments { call_node, ... })`.
fn is_elixir_def_call(call_node: &Node, source: &[u8]) -> bool {
    // call_node.parent() should be `arguments`.
    let args = match call_node.parent() {
        Some(p) if p.kind() == "arguments" => p,
        _ => return false,
    };
    let outer_call = match args.parent() {
        Some(p) if p.kind() == "call" => p,
        _ => return false,
    };
    // Find the first identifier child of outer_call.
    for i in 0..outer_call.child_count() {
        if let Some(child) = outer_call.child(i) {
            if child.kind() == "identifier" {
                let start = child.start_byte();
                let end = child.end_byte();
                if end <= source.len() {
                    let text = &source[start..end];
                    return matches!(text, b"def" | b"defp" | b"defmacro" | b"defmacrop");
                }
                return false;
            }
        }
    }
    false
}

/// Classify OCaml reference kind.
///
/// Grammar reference: tree-sitter-ocaml. The identifier-like node is a
/// `value_name`. At definition sites its parent is `let_binding` (or
/// `value_definition`). At call sites its parent is `value_path`, whose
/// parent is `application_expression` with this `value_path` as the first
/// child. Shadowing rebinding is another Definition (per OCaml semantics —
/// `let x = ... let x = ...` is two separate bindings, not a Write).
fn classify_ocaml_reference(node: &Node, parent: &Node, _source: &[u8]) -> ReferenceKind {
    let parent_kind = parent.kind();

    match parent_kind {
        "let_binding" | "value_definition" => {
            // The bound name is the first `value_name` child.
            if let Some(name) = parent.child_by_field_name("pattern") {
                if node.id() == name.id() || node_contains(node, &name) {
                    return ReferenceKind::Definition;
                }
            }
            if let Some(first_name) = first_named_child_of_kind(parent, &["value_name"]) {
                if node.id() == first_name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "value_path" => {
            // If this path is the callee of an application_expression, it's a Call.
            // Otherwise it's a Read.
            if let Some(grandparent) = parent.parent() {
                if matches!(grandparent.kind(), "application_expression" | "application") {
                    if let Some(first) = grandparent.child(0) {
                        if first.id() == parent.id() {
                            return ReferenceKind::Call;
                        }
                    }
                }
            }
            ReferenceKind::Read
        }

        "application_expression" | "application" => {
            // Bare value_name used as callee (no module qualifier).
            if let Some(first) = parent.child(0) {
                if node.id() == first.id() {
                    return ReferenceKind::Call;
                }
            }
            ReferenceKind::Read
        }

        "module_binding" | "module_definition" => {
            if let Some(first_name) = first_named_child_of_kind(parent, &["module_name"]) {
                if node.id() == first_name.id() {
                    return ReferenceKind::Definition;
                }
            }
            ReferenceKind::Read
        }

        "open_module" | "include_module" => ReferenceKind::Import,

        "type_constructor" | "type_constructor_path" => ReferenceKind::Type,

        _ => ReferenceKind::Read,
    }
}

/// Check if a node is contained within another node (by position)
fn node_contains(inner: &Node, outer: &Node) -> bool {
    inner.start_byte() >= outer.start_byte() && inner.end_byte() <= outer.end_byte()
}

/// Check if parent is in a type annotation context
fn is_type_context(node: &Node) -> bool {
    let mut current = Some(*node);
    while let Some(n) = current {
        let kind = n.kind();
        if matches!(
            kind,
            "type"
                | "annotation"
                | "type_annotation"
                | "return_type"
                | "parameter"
                | "typed_parameter"
                | "generic_type"
                | "type_arguments"
        ) {
            return true;
        }
        // Stop at statement boundaries
        if kind.ends_with("_statement") || kind.ends_with("_definition") {
            return false;
        }
        current = n.parent();
    }
    false
}

// =============================================================================
// Phase 11: Definition Finding and Cross-File Tracking
// =============================================================================

/// Find the definition location for a symbol
///
/// Searches for definition patterns in the AST across all files in the workspace:
/// - Python: function_definition, class_definition, assignment
/// - TypeScript: function_declaration, class_declaration, variable_declaration
/// - Go: function_declaration, type_declaration
/// - Rust: function_item, struct_item, enum_item
///
/// # Arguments
///
/// * `symbol` - The symbol name to find the definition for
/// * `root` - The root directory to search in
/// * `language` - Optional language filter
///
/// # Returns
///
/// `Some(Definition)` if found, `None` otherwise.
pub fn find_definition(
    symbol: &str,
    root: &Path,
    language: Option<&str>,
) -> TldrResult<Option<Definition>> {
    // Walk all source files
    for entry in walk_project(root)
        .filter(|e| e.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter(|e| is_source_file(e.path(), language))
    {
        // Try to find definition in this file
        if let Ok(Some(def)) = find_definition_in_file(symbol, entry.path(), language) {
            return Ok(Some(def));
        }
    }

    Ok(None)
}

/// Find definition of a symbol in a specific file
fn find_definition_in_file(
    symbol: &str,
    file_path: &Path,
    _language_str: Option<&str>,
) -> TldrResult<Option<Definition>> {
    let parsed = parse_file(file_path)?;
    let (tree, source, language) = parsed;
    let source_bytes = source.as_bytes();

    // Search recursively through the AST for definition nodes
    let root = tree.root_node();
    find_definition_in_node(&root, symbol, source_bytes, language, file_path)
}

/// Recursively search for a definition in an AST node
fn find_definition_in_node(
    node: &Node,
    symbol: &str,
    source: &[u8],
    language: Language,
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    // Check if this node is a definition matching our symbol
    if let Some(def) = check_definition_node(node, symbol, source, language, file_path)? {
        return Ok(Some(def));
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(def) = find_definition_in_node(&child, symbol, source, language, file_path)? {
            return Ok(Some(def));
        }
    }

    Ok(None)
}

/// Check if a node is a definition of the target symbol
fn check_definition_node(
    node: &Node,
    symbol: &str,
    source: &[u8],
    language: Language,
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    match language {
        Language::Python => check_python_definition(node, symbol, source, file_path),
        Language::TypeScript | Language::JavaScript => {
            check_ts_definition(node, symbol, source, file_path)
        }
        Language::Go => check_go_definition(node, symbol, source, file_path),
        Language::Rust => check_rust_definition(node, symbol, source, file_path),
        _ => Ok(None),
    }
}

/// Check if a Python node is a definition of the target symbol
fn check_python_definition(
    node: &Node,
    symbol: &str,
    source: &[u8],
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    let node_kind = node.kind();

    match node_kind {
        "function_definition" => {
            // def symbol(...):
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Python);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Function,
                        signature,
                    }));
                }
            }
        }
        "class_definition" => {
            // class Symbol:
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Python);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Class,
                        signature,
                    }));
                }
            }
        }
        "assignment" | "expression_statement" => {
            // symbol = value (module-level assignment)
            // Check if parent is module (top-level)
            if let Some(parent) = node.parent() {
                if parent.kind() == "module" || parent.kind() == "expression_statement" {
                    // Look for identifier on left side
                    let target_node = if node_kind == "assignment" {
                        node.child_by_field_name("left")
                    } else {
                        // expression_statement wraps assignment
                        node.child(0).and_then(|c| c.child_by_field_name("left"))
                    };

                    if let Some(left) = target_node {
                        if left.kind() == "identifier"
                            && left.utf8_text(source).unwrap_or("") == symbol
                        {
                            return Ok(Some(Definition {
                                file: file_path.to_path_buf(),
                                line: left.start_position().row + 1,
                                column: left.start_position().column + 1,
                                kind: DefinitionKind::Variable,
                                signature: None,
                            }));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(None)
}

/// Check if a TypeScript/JavaScript node is a definition of the target symbol
fn check_ts_definition(
    node: &Node,
    symbol: &str,
    source: &[u8],
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    let node_kind = node.kind();

    match node_kind {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::TypeScript);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Function,
                        signature,
                    }));
                }
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::TypeScript);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Class,
                        signature,
                    }));
                }
            }
        }
        "variable_declarator" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    // Determine if const (constant) or let/var (variable)
                    let kind = if let Some(parent) = node.parent() {
                        if let Some(gp) = parent.parent() {
                            let decl_text = gp.utf8_text(source).unwrap_or("");
                            if decl_text.starts_with("const") {
                                DefinitionKind::Constant
                            } else {
                                DefinitionKind::Variable
                            }
                        } else {
                            DefinitionKind::Variable
                        }
                    } else {
                        DefinitionKind::Variable
                    };

                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: name_node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind,
                        signature: None,
                    }));
                }
            }
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::TypeScript);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::TypeScript);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        _ => {}
    }

    Ok(None)
}

/// Check if a Go node is a definition of the target symbol
fn check_go_definition(
    node: &Node,
    symbol: &str,
    source: &[u8],
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    let node_kind = node.kind();

    match node_kind {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Go);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Function,
                        signature,
                    }));
                }
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Go);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Method,
                        signature,
                    }));
                }
            }
        }
        "type_declaration" | "type_spec" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Go);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        _ => {}
    }

    Ok(None)
}

/// Check if a Rust node is a definition of the target symbol
fn check_rust_definition(
    node: &Node,
    symbol: &str,
    source: &[u8],
    file_path: &Path,
) -> TldrResult<Option<Definition>> {
    let node_kind = node.kind();

    match node_kind {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Function,
                        signature,
                    }));
                }
            }
        }
        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Constant,
                        signature,
                    }));
                }
            }
        }
        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Variable,
                        signature,
                    }));
                }
            }
        }
        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if name_node.utf8_text(source).unwrap_or("") == symbol {
                    let signature = extract_signature(node, source, Language::Rust);
                    return Ok(Some(Definition {
                        file: file_path.to_path_buf(),
                        line: node.start_position().row + 1,
                        column: name_node.start_position().column + 1,
                        kind: DefinitionKind::Type,
                        signature,
                    }));
                }
            }
        }
        _ => {}
    }

    Ok(None)
}

/// Extract function/class signature from AST node
///
/// Returns the first line of the definition as the signature:
/// - Python: "def login(username, password):" or "class User:"
/// - TypeScript: "function login(username: string): boolean"
/// - Go: "func Login(username string) bool"
/// - Rust: "pub fn login(username: &str) -> bool"
fn extract_signature(node: &Node, source: &[u8], _language: Language) -> Option<String> {
    let node_text = node.utf8_text(source).ok()?;

    // Get the first line of the definition
    let first_line = node_text.lines().next()?;

    // Truncate if too long
    let signature = if first_line.len() > MAX_CONTEXT_LENGTH {
        format!("{}...", &first_line[..MAX_CONTEXT_LENGTH - 3])
    } else {
        first_line.to_string()
    };

    Some(signature.trim().to_string())
}

/// Main entry point for reference finding (text search + AST verification)
///
/// # Arguments
///
/// * `symbol` - The symbol name to search for
/// * `root` - The root directory to search in
/// * `options` - Configuration options for the search
///
/// # Returns
///
/// A ReferencesReport containing all found references with statistics.
///
/// # Phases
///
/// - Phase 9: Text search for candidates
/// - Phase 10: AST verification and kind classification
/// - Phase 11: Definition tracking and cross-file references
pub fn find_references(
    symbol: &str,
    root: &Path,
    options: &ReferencesOptions,
) -> TldrResult<ReferencesReport> {
    let start = std::time::Instant::now();

    let language = options.language.as_deref();

    // Phase 13: Determine search scope based on symbol visibility
    // If scope is explicitly set in options, use it; otherwise auto-detect
    let effective_scope = if options.scope != SearchScope::Workspace {
        // User explicitly set a non-default scope
        options.scope
    } else if let Some(lang) = language {
        // Auto-detect scope based on symbol naming conventions
        determine_search_scope(symbol, options.definition_file.as_deref(), lang)
    } else {
        SearchScope::Workspace
    };

    // Step 1: Text search for candidates (Phase 9)
    let candidates = find_text_candidates(symbol, root, language)?;
    let candidates_found = candidates.len();

    // Phase 13: Apply scope filter before AST verification (for performance)
    let scoped_candidates = apply_scope_filter(
        candidates,
        effective_scope,
        options.definition_file.as_deref(),
    );

    // Step 2: AST verification and kind classification (Phase 10)
    let verified = verify_candidates_with_ast(&scoped_candidates, symbol, language);

    // Step 3: Convert verified references to Reference structs
    let mut references: Vec<Reference> = verified
        .into_iter()
        .map(|(candidate, verified_ref)| Reference {
            file: candidate.file,
            line: candidate.line,
            column: candidate.column,
            kind: verified_ref.kind,
            context: truncate_context(candidate.line_text),
            confidence: Some(verified_ref.confidence),
            end_column: Some(candidate.end_column),
        })
        .collect();

    // Step 4: Find definition (Phase 11)
    let definition = find_definition(symbol, root, language)?;

    // Apply kind filter if specified (Phase 13)
    if let Some(ref kinds) = options.kinds {
        references.retain(|r| kinds.contains(&r.kind));
    }

    // Apply limit if specified
    if let Some(limit) = options.limit {
        references.truncate(limit);
    }

    let files_searched = count_source_files(root, language);
    let verified_references = references.len();

    let stats = ReferenceStats {
        files_searched,
        candidates_found,
        verified_references,
        search_time_ms: start.elapsed().as_millis() as u64,
    };

    Ok(ReferencesReport {
        symbol: symbol.to_string(),
        definition,
        references,
        total_references: verified_references,
        search_scope: effective_scope, // Use the effective scope (auto-detected or explicit)
        stats,
    })
}

// =============================================================================
// Phase 13: Advanced Features - Search Scope Optimization & Kind Filtering
// =============================================================================

/// Determine optimal search scope based on symbol visibility
///
/// This function infers the appropriate search scope based on:
/// - Python: `_prefix` = File scope, `__dunder__` = Workspace, normal = Workspace
/// - TypeScript: non-exported = File scope, exported = Workspace
/// - Go: lowercase = File (package-private), uppercase = Workspace
/// - Rust: pub = Workspace, pub(crate) = Workspace, private = File
///
/// # Arguments
///
/// * `symbol` - The symbol name to analyze
/// * `definition_file` - Optional path to the file containing the definition
/// * `language` - The programming language (e.g., "python", "typescript", "go", "rust")
///
/// # Returns
///
/// The inferred `SearchScope` for the symbol.
///
/// # Phase 13 Risks Addressed
///
/// - S7-R19: SearchScope Go package - handles Go visibility
/// - S7-R29: Private class methods - conservative: if unsure, use Workspace
/// - S7-R30: Rust pub(crate)/pub(super) - maps to appropriate scope
pub fn determine_search_scope(
    symbol: &str,
    definition_file: Option<&Path>,
    language: &str,
) -> SearchScope {
    match language.to_lowercase().as_str() {
        "python" => determine_python_scope(symbol, definition_file),
        "typescript" | "javascript" => determine_ts_scope(symbol, definition_file),
        "go" => determine_go_scope(symbol, definition_file),
        "rust" => determine_rust_scope(symbol, definition_file),
        _ => SearchScope::Workspace, // Conservative default
    }
}

/// Determine Python scope based on naming conventions
///
/// - `_single_underscore` = private by convention = File scope
/// - `__dunder__` = special methods = Workspace scope (implicit calls)
/// - `__name_mangled` (double underscore, no trailing) = File scope
/// - Other = Workspace scope
fn determine_python_scope(symbol: &str, _definition_file: Option<&Path>) -> SearchScope {
    // __dunder__ methods are special - they're called implicitly
    if symbol.starts_with("__") && symbol.ends_with("__") {
        return SearchScope::Workspace;
    }

    // __name_mangled (double underscore without trailing) = private
    if symbol.starts_with("__") && !symbol.ends_with("__") {
        return SearchScope::File;
    }

    // _single_underscore = private by convention
    if symbol.starts_with('_') && !symbol.starts_with("__") {
        return SearchScope::File;
    }

    // Default: public symbol
    SearchScope::Workspace
}

/// Determine TypeScript/JavaScript scope
///
/// Without parsing the file, we can't know if a symbol is exported.
/// Conservative approach: assume Workspace scope.
fn determine_ts_scope(_symbol: &str, _definition_file: Option<&Path>) -> SearchScope {
    // TODO: Parse definition_file to check for export keyword
    // For now, be conservative and search workspace
    SearchScope::Workspace
}

/// Determine Go scope based on capitalization
///
/// - Uppercase first letter = exported = Workspace
/// - Lowercase first letter = package-private = File (approximation)
fn determine_go_scope(symbol: &str, _definition_file: Option<&Path>) -> SearchScope {
    if let Some(first_char) = symbol.chars().next() {
        if first_char.is_uppercase() {
            return SearchScope::Workspace;
        }
        // Lowercase = package-private, approximate as File scope
        return SearchScope::File;
    }
    SearchScope::Workspace
}

/// Determine Rust scope based on naming (conservative)
///
/// Without parsing the file, we can't know visibility modifiers.
/// Conservative approach: assume Workspace scope.
fn determine_rust_scope(_symbol: &str, _definition_file: Option<&Path>) -> SearchScope {
    // TODO: Parse definition_file to check pub/pub(crate)/private
    // For now, be conservative and search workspace
    SearchScope::Workspace
}

/// Apply scope filtering to text search candidates
///
/// Filters candidates based on the search scope:
/// - `Local`: Only candidates in the same file (TODO: same function)
/// - `File`: Only candidates in the definition file
/// - `Workspace`: No filtering, return all candidates
///
/// # Arguments
///
/// * `candidates` - Vector of text search candidates
/// * `scope` - The search scope to apply
/// * `definition_file` - The file containing the symbol definition
///
/// # Returns
///
/// Filtered vector of candidates matching the scope.
pub fn apply_scope_filter(
    candidates: Vec<TextCandidate>,
    scope: SearchScope,
    definition_file: Option<&Path>,
) -> Vec<TextCandidate> {
    match scope {
        SearchScope::Workspace => candidates, // No filter
        SearchScope::File => {
            if let Some(def_file) = definition_file {
                candidates
                    .into_iter()
                    .filter(|c| c.file == def_file)
                    .collect()
            } else {
                candidates // Can't filter without definition file
            }
        }
        SearchScope::Local => {
            // Local scope: restrict to same file
            // TODO: Further restrict to same function/block
            if let Some(def_file) = definition_file {
                candidates
                    .into_iter()
                    .filter(|c| c.file == def_file)
                    .collect()
            } else {
                candidates
            }
        }
    }
}

/// Filter references by allowed kinds
///
/// Returns only references whose kind is in the allowed_kinds list.
///
/// # Arguments
///
/// * `references` - Vector of references to filter
/// * `allowed_kinds` - Slice of allowed ReferenceKind values
///
/// # Returns
///
/// Filtered vector containing only references with allowed kinds.
///
/// # Example
///
/// ```ignore
/// let filtered = filter_by_kinds(refs, &[ReferenceKind::Call, ReferenceKind::Import]);
/// ```
pub fn filter_by_kinds(
    references: Vec<Reference>,
    allowed_kinds: &[ReferenceKind],
) -> Vec<Reference> {
    references
        .into_iter()
        .filter(|r| allowed_kinds.contains(&r.kind))
        .collect()
}

/// Get incoming calls (who calls this function)
///
/// This is a basic call hierarchy feature that finds all locations
/// where the specified symbol is called.
///
/// # Arguments
///
/// * `symbol` - The function/method name to find callers for
/// * `root` - The root directory to search in
/// * `options` - Reference finding options
///
/// # Returns
///
/// Vector of References with kind == Call
pub fn get_incoming_calls(
    symbol: &str,
    root: &Path,
    options: &ReferencesOptions,
) -> TldrResult<Vec<Reference>> {
    let report = find_references(symbol, root, options)?;
    Ok(report
        .references
        .into_iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .collect())
}

/// Get outgoing calls (what this function calls)
///
/// This finds all function calls made within a specific function.
/// Uses the AST to find all call expressions within the function body.
///
/// # Arguments
///
/// * `file` - Path to the file containing the function
/// * `function` - Name of the function to analyze
///
/// # Returns
///
/// Vector of function names that are called by the specified function.
///
/// # Note
///
/// This is a simplified implementation. For full call graph analysis,
/// use the `tldr calls` command infrastructure.
pub fn get_outgoing_calls(file: &Path, function: &str) -> TldrResult<Vec<String>> {
    use crate::ast::parser::parse_file;

    let (tree, source, language) = parse_file(file)?;
    let source_bytes = source.as_bytes();

    // Find the function node and extract calls in one pass
    let root = tree.root_node();
    let calls = find_and_extract_calls(&root, function, source_bytes, language);
    Ok(calls)
}

/// Find a function by name and extract all calls from it
///
/// Combines function finding and call extraction to avoid lifetime issues.
fn find_and_extract_calls(
    node: &tree_sitter::Node,
    function_name: &str,
    source: &[u8],
    language: Language,
) -> Vec<String> {
    let node_kind = node.kind();

    // Check if this is a function definition with matching name
    let is_function = ast_utils::function_node_kinds(language).contains(&node_kind);

    if is_function {
        if let Some(name_node) = node.child_by_field_name("name") {
            if name_node.utf8_text(source).unwrap_or("") == function_name {
                // Found the function, extract all calls from it
                let mut calls = Vec::new();
                extract_calls_recursive(node, source, language, &mut calls);
                return calls;
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let calls = find_and_extract_calls(&child, function_name, source, language);
        if !calls.is_empty() {
            return calls;
        }
    }

    Vec::new()
}

/// Recursively extract call expressions from a node
fn extract_calls_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    language: Language,
    calls: &mut Vec<String>,
) {
    let node_kind = node.kind();

    // Check if this is a call expression
    let is_call = ast_utils::call_node_kinds(language).contains(&node_kind);

    if is_call {
        // Extract the function name being called
        if let Some(func_node) = node.child_by_field_name("function") {
            let func_text = func_node.utf8_text(source).unwrap_or("");
            // For simple identifiers, use as-is; for member access, extract the method name
            let call_name = if func_text.contains('.') {
                func_text.rsplit('.').next().unwrap_or(func_text)
            } else {
                func_text
            };
            if !call_name.is_empty() {
                calls.push(call_name.to_string());
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_calls_recursive(&child, source, language, calls);
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_references_report_default() {
        let report = ReferencesReport::default();
        assert!(report.symbol.is_empty());
        assert!(report.definition.is_none());
        assert!(report.references.is_empty());
        assert_eq!(report.total_references, 0);
        assert_eq!(report.search_scope, SearchScope::Workspace);
    }

    #[test]
    fn test_references_report_new() {
        let report = ReferencesReport::new("test_symbol".to_string());
        assert_eq!(report.symbol, "test_symbol");
        assert!(report.definition.is_none());
        assert!(report.references.is_empty());
    }

    #[test]
    fn test_definition_kind_serialization() {
        let kinds = vec![
            DefinitionKind::Function,
            DefinitionKind::Class,
            DefinitionKind::Variable,
            DefinitionKind::Constant,
            DefinitionKind::Type,
            DefinitionKind::Module,
            DefinitionKind::Method,
            DefinitionKind::Property,
            DefinitionKind::Other,
        ];

        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: DefinitionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn test_reference_kind_serialization() {
        let kinds = vec![
            ReferenceKind::Call,
            ReferenceKind::Read,
            ReferenceKind::Write,
            ReferenceKind::Import,
            ReferenceKind::Type,
            ReferenceKind::Definition,
            ReferenceKind::Other,
        ];

        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: ReferenceKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn test_search_scope_serialization() {
        let scopes = vec![
            SearchScope::Local,
            SearchScope::File,
            SearchScope::Workspace,
        ];

        for scope in scopes {
            let json = serde_json::to_string(&scope).unwrap();
            let parsed: SearchScope = serde_json::from_str(&json).unwrap();
            assert_eq!(scope, parsed);
        }
    }

    #[test]
    fn test_reference_kind_parse() {
        assert_eq!(ReferenceKind::parse("call"), Some(ReferenceKind::Call));
        assert_eq!(ReferenceKind::parse("CALL"), Some(ReferenceKind::Call));
        assert_eq!(ReferenceKind::parse("read"), Some(ReferenceKind::Read));
        assert_eq!(ReferenceKind::parse("invalid"), None);
    }

    #[test]
    fn test_search_scope_parse() {
        assert_eq!(SearchScope::parse("local"), Some(SearchScope::Local));
        assert_eq!(SearchScope::parse("FILE"), Some(SearchScope::File));
        assert_eq!(
            SearchScope::parse("workspace"),
            Some(SearchScope::Workspace)
        );
        assert_eq!(SearchScope::parse("invalid"), None);
    }

    #[test]
    fn test_truncate_context_short() {
        let short = "def login(): pass".to_string();
        let result = truncate_context(short.clone());
        assert_eq!(result, short);
    }

    #[test]
    fn test_truncate_context_long() {
        let long: String = "x".repeat(300);
        let result = truncate_context(long);
        assert!(result.len() <= MAX_CONTEXT_LENGTH);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_definition_new() {
        let def = Definition::new(
            PathBuf::from("src/auth.py"),
            42,
            5,
            DefinitionKind::Function,
        );
        assert_eq!(def.file, PathBuf::from("src/auth.py"));
        assert_eq!(def.line, 42);
        assert_eq!(def.column, 5);
        assert_eq!(def.kind, DefinitionKind::Function);
        assert!(def.signature.is_none());
    }

    #[test]
    fn test_definition_with_signature() {
        let def = Definition::with_signature(
            PathBuf::from("src/auth.py"),
            42,
            5,
            DefinitionKind::Function,
            "def login(username: str, password: str) -> bool:".to_string(),
        );
        assert!(def.signature.is_some());
        assert!(def.signature.as_ref().unwrap().contains("login"));
    }

    #[test]
    fn test_reference_new() {
        let ref_ = Reference::new(
            PathBuf::from("src/routes.py"),
            15,
            12,
            ReferenceKind::Call,
            "result = auth.login(username, password)".to_string(),
        );
        assert_eq!(ref_.file, PathBuf::from("src/routes.py"));
        assert_eq!(ref_.line, 15);
        assert_eq!(ref_.column, 12);
        assert_eq!(ref_.kind, ReferenceKind::Call);
        assert!(ref_.confidence.is_none());
    }

    #[test]
    fn test_reference_verified() {
        let ref_ = Reference::verified(
            PathBuf::from("src/routes.py"),
            15,
            12,
            ReferenceKind::Call,
            "login()".to_string(),
        );
        assert_eq!(ref_.confidence, Some(1.0));
    }

    #[test]
    fn test_reference_stats_default() {
        let stats = ReferenceStats::default();
        assert_eq!(stats.files_searched, 0);
        assert_eq!(stats.candidates_found, 0);
        assert_eq!(stats.verified_references, 0);
        assert_eq!(stats.search_time_ms, 0);
    }

    #[test]
    fn test_reference_stats_with_time() {
        let stats = ReferenceStats::new(10, 50, 25).with_time(127);
        assert_eq!(stats.files_searched, 10);
        assert_eq!(stats.candidates_found, 50);
        assert_eq!(stats.verified_references, 25);
        assert_eq!(stats.search_time_ms, 127);
    }

    #[test]
    fn test_references_options_builder() {
        let opts = ReferencesOptions::new()
            .with_definition()
            .with_kinds(vec![ReferenceKind::Call, ReferenceKind::Import])
            .with_scope(SearchScope::File)
            .with_limit(100)
            .with_context_lines(2);

        assert!(opts.include_definition);
        assert_eq!(opts.kinds.as_ref().unwrap().len(), 2);
        assert_eq!(opts.scope, SearchScope::File);
        assert_eq!(opts.limit, Some(100));
        assert_eq!(opts.context_lines, 2);
    }

    #[test]
    fn test_report_serialization() {
        let report = ReferencesReport {
            symbol: "login".to_string(),
            definition: Some(Definition::new(
                PathBuf::from("src/auth.py"),
                42,
                5,
                DefinitionKind::Function,
            )),
            references: vec![Reference::new(
                PathBuf::from("src/routes.py"),
                15,
                12,
                ReferenceKind::Call,
                "login()".to_string(),
            )],
            total_references: 1,
            search_scope: SearchScope::Workspace,
            stats: ReferenceStats::new(10, 5, 1).with_time(50),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        let parsed: ReferencesReport = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.symbol, "login");
        assert!(parsed.definition.is_some());
        assert_eq!(parsed.references.len(), 1);
        assert_eq!(parsed.total_references, 1);
        assert_eq!(parsed.search_scope, SearchScope::Workspace);
    }

    #[test]
    fn test_no_matches_report() {
        let report = ReferencesReport::no_matches(
            "nonexistent".to_string(),
            SearchScope::Workspace,
            ReferenceStats::new(50, 0, 0),
        );

        assert_eq!(report.symbol, "nonexistent");
        assert!(report.definition.is_none());
        assert!(report.references.is_empty());
        assert_eq!(report.total_references, 0);
        assert_eq!(report.stats.files_searched, 50);
    }

    // =========================================================================
    // Tier-2 classifier tests (VAL-Java..VAL-Ocaml)
    //
    // Each test parses a minimal fixture with one definition of `greet`
    // and two call sites. It walks the tree, classifies every identifier
    // whose text is `greet`, and asserts:
    //   1. at least one Definition
    //   2. at least two Calls
    //   3. NO Other for any occurrence of `greet`
    // =========================================================================

    /// Collect (kind, line) for every identifier-like node whose text equals `needle`.
    fn collect_reference_kinds_for(
        source: &str,
        lang: Language,
        needle: &str,
        identifier_kinds: &[&str],
    ) -> Vec<(ReferenceKind, usize)> {
        use crate::ast::parser;
        let tree = parser::parse(source, lang).expect("parse should succeed");
        let src_bytes = source.as_bytes();
        let mut results = Vec::new();

        fn walk<'a>(
            node: tree_sitter::Node<'a>,
            source: &'a [u8],
            src_str: &'a str,
            needle: &str,
            identifier_kinds: &[&str],
            language: Language,
            out: &mut Vec<(ReferenceKind, usize)>,
        ) {
            if identifier_kinds.contains(&node.kind()) {
                let start = node.start_byte();
                let end = node.end_byte();
                if end <= src_str.len() {
                    let text = &src_str[start..end];
                    if text == needle {
                        let kind = classify_reference_kind(&node, source, language);
                        let line = node.start_position().row + 1;
                        out.push((kind, line));
                    }
                }
            }
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    walk(
                        child,
                        source,
                        src_str,
                        needle,
                        identifier_kinds,
                        language,
                        out,
                    );
                }
            }
        }

        walk(
            tree.root_node(),
            src_bytes,
            source,
            needle,
            identifier_kinds,
            lang,
            &mut results,
        );
        results
    }

    /// Assert the classifier found at least one Definition and at least two Calls,
    /// and never returned Other for the needle identifier.
    fn assert_def_and_calls(results: &[(ReferenceKind, usize)], language_name: &str) {
        assert!(
            !results.is_empty(),
            "{}: no occurrences of `greet` matched by walker",
            language_name
        );
        let def_count = results
            .iter()
            .filter(|(k, _)| *k == ReferenceKind::Definition)
            .count();
        let call_count = results
            .iter()
            .filter(|(k, _)| *k == ReferenceKind::Call)
            .count();
        let other_count = results
            .iter()
            .filter(|(k, _)| *k == ReferenceKind::Other)
            .count();
        assert!(
            def_count >= 1,
            "{}: expected >=1 Definition, got {} (results={:?})",
            language_name,
            def_count,
            results
        );
        assert!(
            call_count >= 2,
            "{}: expected >=2 Calls, got {} (results={:?})",
            language_name,
            call_count,
            results
        );
        assert_eq!(
            other_count, 0,
            "{}: expected 0 Other, got {} (results={:?})",
            language_name, other_count, results
        );
    }

    #[test]
    fn test_java_classifier_emits_call_and_definition() {
        let src = r#"
class App {
    static String greet(String name) { return "Hello " + name; }
    public static void main(String[] args) {
        System.out.println(greet("World"));
        System.out.println(greet("Alice"));
    }
}
"#;
        let results = collect_reference_kinds_for(src, Language::Java, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Java");
    }

    #[test]
    fn test_c_classifier_emits_call_and_definition() {
        let src = r#"
#include <stdio.h>
void greet(const char *name) { printf("Hello %s\n", name); }
int main(void) {
    greet("World");
    greet("Alice");
    return 0;
}
"#;
        let results = collect_reference_kinds_for(src, Language::C, "greet", &["identifier"]);
        assert_def_and_calls(&results, "C");
    }

    #[test]
    fn test_cpp_classifier_emits_call_and_definition() {
        let src = r#"
#include <iostream>
void greet(const std::string &name) { std::cout << "Hello " << name; }
int main() {
    greet("World");
    greet("Alice");
    return 0;
}
"#;
        let results = collect_reference_kinds_for(src, Language::Cpp, "greet", &["identifier"]);
        assert_def_and_calls(&results, "C++");
    }

    #[test]
    fn test_csharp_classifier_emits_call_and_definition() {
        let src = r#"
class App {
    static string greet(string name) { return "Hello " + name; }
    static void Main() {
        System.Console.WriteLine(greet("World"));
        System.Console.WriteLine(greet("Alice"));
    }
}
"#;
        let results = collect_reference_kinds_for(src, Language::CSharp, "greet", &["identifier"]);
        assert_def_and_calls(&results, "C#");
    }

    #[test]
    fn test_kotlin_classifier_emits_call_and_definition() {
        let src = r#"
fun greet(name: String): String { return "Hello " + name }
fun main() {
    println(greet("World"))
    println(greet("Alice"))
}
"#;
        let results = collect_reference_kinds_for(src, Language::Kotlin, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Kotlin");
    }

    #[test]
    fn test_scala_classifier_emits_call_and_definition() {
        let src = r#"
object App {
  def greet(name: String): String = "Hello " + name
  def main(args: Array[String]): Unit = {
    println(greet("World"))
    println(greet("Alice"))
  }
}
"#;
        let results = collect_reference_kinds_for(
            src,
            Language::Scala,
            "greet",
            &["identifier", "type_identifier"],
        );
        assert_def_and_calls(&results, "Scala");
    }

    #[test]
    fn test_swift_classifier_emits_call_and_definition() {
        let src = r#"
func greet(name: String) -> String { return "Hello " + name }
func main() {
    print(greet(name: "World"))
    print(greet(name: "Alice"))
}
"#;
        let results =
            collect_reference_kinds_for(src, Language::Swift, "greet", &["simple_identifier"]);
        assert_def_and_calls(&results, "Swift");
    }

    #[test]
    fn test_php_classifier_emits_call_and_definition() {
        let src = r#"<?php
function greet($name) { return "Hello " . $name; }
echo greet("World");
echo greet("Alice");
"#;
        let results = collect_reference_kinds_for(src, Language::Php, "greet", &["name"]);
        assert_def_and_calls(&results, "PHP");
    }

    #[test]
    fn test_ruby_classifier_emits_call_and_definition() {
        let src = r#"
def greet(name)
  "Hello " + name
end
puts greet("World")
puts greet("Alice")
"#;
        let results = collect_reference_kinds_for(src, Language::Ruby, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Ruby");
    }

    #[test]
    fn test_lua_classifier_emits_call_and_definition() {
        let src = r#"
function greet(name)
  return "Hello " .. name
end
print(greet("World"))
print(greet("Alice"))
"#;
        let results = collect_reference_kinds_for(src, Language::Lua, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Lua");
    }

    #[test]
    fn test_luau_classifier_emits_call_and_definition() {
        let src = r#"
function greet(name: string): string
  return "Hello " .. name
end
print(greet("World"))
print(greet("Alice"))
"#;
        let results = collect_reference_kinds_for(src, Language::Luau, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Luau");
    }

    #[test]
    fn test_elixir_classifier_emits_call_and_definition() {
        let src = r#"
defmodule App do
  def greet(name) do
    "Hello " <> name
  end
end

IO.puts(App.greet("World"))
IO.puts(App.greet("Alice"))
"#;
        let results = collect_reference_kinds_for(src, Language::Elixir, "greet", &["identifier"]);
        assert_def_and_calls(&results, "Elixir");
    }

    #[test]
    fn test_ocaml_classifier_emits_call_and_definition() {
        let src = r#"
let greet name = "Hello " ^ name
let _ = print_string (greet "World")
let _ = print_string (greet "Alice")
"#;
        let results = collect_reference_kinds_for(src, Language::Ocaml, "greet", &["value_name"]);
        assert_def_and_calls(&results, "OCaml");
    }
}
