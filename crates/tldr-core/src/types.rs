//! Common types for TLDR operations
//!
//! This module defines all shared types used across the TLDR codebase.
//! All types derive Serialize/Deserialize with consistent field ordering
//! to address M5 (JSON Serialization Consistency).
//!
//! ## Submodules
//!
//! - `inheritance` - Types for class hierarchy extraction (Phase 7-9, A9)
//! - `patterns` - Types for design pattern mining (Phase 4-6, A10)
//! - `arch_rules` - Types for architecture rules and violations (Phase 3, A11)

// =============================================================================
// Submodules for Architecture Commands (Phase 1: Types Foundation)
// =============================================================================

pub mod arch_rules;
pub mod inheritance;
pub mod patterns;

// Re-export submodule types for convenience
pub use arch_rules::*;
pub use inheritance::*;
pub use patterns::*;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// =============================================================================
// Language Support
// =============================================================================

/// Supported programming languages (17 variants as per spec Section 1.2)
///
/// Priority levels:
/// - P0: Python, TypeScript, JavaScript, Go (full support)
/// - P1: Rust, Java (full support)
/// - P2: C, C++, Ruby, Kotlin, Swift, C#, Scala, PHP, Lua, Luau, Elixir (basic support)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Python (.py)
    Python,
    /// TypeScript (.ts, .tsx)
    TypeScript,
    /// JavaScript (.js, .jsx, .mjs, .cjs)
    JavaScript,
    /// Go (.go)
    Go,
    /// Rust (.rs)
    Rust,
    /// Java (.java)
    Java,
    /// C (.c, .h)
    C,
    /// C++ (.cpp, .cc, .cxx, .hpp)
    Cpp,
    /// Ruby (.rb)
    Ruby,
    /// Kotlin (.kt, .kts)
    Kotlin,
    /// Swift (.swift)
    Swift,
    /// C# (.cs)
    CSharp,
    /// Scala (.scala)
    Scala,
    /// PHP (.php)
    Php,
    /// Lua (.lua)
    Lua,
    /// Luau (.luau)
    Luau,
    /// Elixir (.ex, .exs)
    Elixir,
    /// OCaml (.ml, .mli)
    Ocaml,
}

impl Language {
    /// Get file extensions for this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            Language::Python => &[".py"],
            Language::TypeScript => &[".ts", ".tsx"],
            Language::JavaScript => &[".js", ".jsx", ".mjs", ".cjs"],
            Language::Go => &[".go"],
            Language::Rust => &[".rs"],
            Language::Java => &[".java"],
            Language::C => &[".c", ".h"],
            Language::Cpp => &[".cpp", ".cc", ".cxx", ".hpp"],
            Language::Ruby => &[".rb"],
            Language::Kotlin => &[".kt", ".kts"],
            Language::Swift => &[".swift"],
            Language::CSharp => &[".cs"],
            Language::Scala => &[".scala"],
            Language::Php => &[".php"],
            Language::Lua => &[".lua"],
            Language::Luau => &[".luau"],
            Language::Elixir => &[".ex", ".exs"],
            Language::Ocaml => &[".ml", ".mli"],
        }
    }

    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        // Normalize extension to lowercase with leading dot
        let ext = if ext.starts_with('.') {
            ext.to_lowercase()
        } else {
            format!(".{}", ext.to_lowercase())
        };

        match ext.as_str() {
            ".py" => Some(Language::Python),
            ".ts" | ".tsx" => Some(Language::TypeScript),
            ".js" | ".jsx" | ".mjs" | ".cjs" => Some(Language::JavaScript),
            ".go" => Some(Language::Go),
            ".rs" => Some(Language::Rust),
            ".java" => Some(Language::Java),
            ".c" | ".h" => Some(Language::C),
            ".cpp" | ".cc" | ".cxx" | ".hpp" => Some(Language::Cpp),
            ".rb" => Some(Language::Ruby),
            ".kt" | ".kts" => Some(Language::Kotlin),
            ".swift" => Some(Language::Swift),
            ".cs" => Some(Language::CSharp),
            ".scala" => Some(Language::Scala),
            ".php" => Some(Language::Php),
            ".lua" => Some(Language::Lua),
            ".luau" => Some(Language::Luau),
            ".ex" | ".exs" => Some(Language::Elixir),
            ".ml" | ".mli" => Some(Language::Ocaml),
            _ => None,
        }
    }

    /// Detect language from file path
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(|ext| Self::from_extension(&format!(".{}", ext)))
    }

    /// Detect dominant language from files in a directory
    ///
    /// Uses recursive traversal via walkdir to find source files at any depth.
    /// This is important for projects with deep directory structures like
    /// Java (src/main/java/...) or C# (src/...).
    pub fn from_directory(path: &std::path::Path) -> Option<Self> {
        use std::collections::HashMap;
        use walkdir::WalkDir;

        let mut counts: HashMap<Language, usize> = HashMap::new();

        // Recursively walk the directory, skipping hidden directories
        for entry in WalkDir::new(path)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                // Skip hidden directories (starting with .)
                e.file_name()
                    .to_str()
                    .map(|s| !s.starts_with('.'))
                    .unwrap_or(true)
            })
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.is_file() {
                if let Some(lang) = Self::from_path(p) {
                    *counts.entry(lang).or_insert(0) += 1;
                }
            }
        }

        counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(lang, _)| lang)
    }

    /// Get the language name as it appears in JSON output
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Go => "go",
            Language::Rust => "rust",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Ruby => "ruby",
            Language::Kotlin => "kotlin",
            Language::Swift => "swift",
            Language::CSharp => "csharp",
            Language::Scala => "scala",
            Language::Php => "php",
            Language::Lua => "lua",
            Language::Luau => "luau",
            Language::Elixir => "elixir",
            Language::Ocaml => "ocaml",
        }
    }

    /// Check if this is a P0 (highest priority) language
    pub fn is_p0(&self) -> bool {
        matches!(
            self,
            Language::Python | Language::TypeScript | Language::JavaScript | Language::Go
        )
    }

    /// Check if this is a P1 (high priority) language
    pub fn is_p1(&self) -> bool {
        matches!(self, Language::Rust | Language::Java)
    }

    /// Get all supported languages
    pub fn all() -> &'static [Language] {
        &[
            Language::Python,
            Language::TypeScript,
            Language::JavaScript,
            Language::Go,
            Language::Rust,
            Language::Java,
            Language::C,
            Language::Cpp,
            Language::Ruby,
            Language::Kotlin,
            Language::Swift,
            Language::CSharp,
            Language::Scala,
            Language::Php,
            Language::Lua,
            Language::Luau,
            Language::Elixir,
            Language::Ocaml,
        ]
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "python" | "py" => Ok(Language::Python),
            "typescript" | "ts" => Ok(Language::TypeScript),
            "javascript" | "js" => Ok(Language::JavaScript),
            "go" | "golang" => Ok(Language::Go),
            "rust" | "rs" => Ok(Language::Rust),
            "java" => Ok(Language::Java),
            "c" => Ok(Language::C),
            "cpp" | "c++" | "cxx" => Ok(Language::Cpp),
            "ruby" | "rb" => Ok(Language::Ruby),
            "kotlin" | "kt" => Ok(Language::Kotlin),
            "swift" => Ok(Language::Swift),
            "csharp" | "c#" | "cs" => Ok(Language::CSharp),
            "scala" => Ok(Language::Scala),
            "php" => Ok(Language::Php),
            "lua" => Ok(Language::Lua),
            "luau" => Ok(Language::Luau),
            "elixir" | "ex" => Ok(Language::Elixir),
            "ocaml" | "ml" => Ok(Language::Ocaml),
            _ => Err(format!("Unknown language: {}", s)),
        }
    }
}

// =============================================================================
// File System Types
// =============================================================================

/// File tree node type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    /// Directory node
    Dir,
    /// File node
    File,
}

/// File tree structure (spec Section 2.1.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTree {
    /// Display name of the file or directory
    pub name: String,
    /// Whether this node is a file or directory
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// Absolute path to the file (None for directory nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Child nodes (only populated for directory nodes)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FileTree>,
}

impl FileTree {
    /// Create a new file node
    pub fn file(name: impl Into<String>, path: PathBuf) -> Self {
        Self {
            name: name.into(),
            node_type: NodeType::File,
            path: Some(path),
            children: Vec::new(),
        }
    }

    /// Create a new directory node
    pub fn dir(name: impl Into<String>, children: Vec<FileTree>) -> Self {
        Self {
            name: name.into(),
            node_type: NodeType::Dir,
            path: None,
            children,
        }
    }
}

/// File entry for flat file lists
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path to the file
    pub path: PathBuf,
    /// Detected programming language, if any
    pub language: Option<Language>,
    /// File size in bytes
    pub size_bytes: u64,
}

/// Ignore specification (gitignore-style patterns)
#[derive(Debug, Clone, Default)]
pub struct IgnoreSpec {
    /// Glob patterns for files and directories to ignore
    pub patterns: Vec<String>,
}

impl IgnoreSpec {
    /// Create a new ignore spec from patterns
    pub fn new(patterns: Vec<String>) -> Self {
        Self { patterns }
    }

    /// Load from a file (like .tldrignore or .gitignore)
    pub fn from_file(_path: &std::path::Path) -> std::io::Result<Self> {
        // TODO: Implement in Phase 2
        Ok(Self::default())
    }

    /// Check if a path should be ignored
    pub fn is_ignored(&self, _path: &std::path::Path) -> bool {
        // TODO: Implement pattern matching in Phase 2
        false
    }
}

// =============================================================================
// AST Types (Layer 1)
// =============================================================================

/// Code structure for a project (spec Section 2.1.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeStructure {
    /// Root directory of the analyzed project
    pub root: PathBuf,
    /// Primary programming language of the project
    pub language: Language,
    /// Structural information for each source file
    pub files: Vec<FileStructure>,
}

/// Definition-level information with line ranges and signatures.
/// Extracted from tree-sitter AST, suitable for caching.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionInfo {
    /// Symbol name
    pub name: String,
    /// Kind: "function", "method", "class", "struct"
    pub kind: String,
    /// Start line (1-indexed)
    pub line_start: u32,
    /// End line (1-indexed, inclusive)
    pub line_end: u32,
    /// Signature line (e.g., "pub fn foo(x: i32) -> bool")
    pub signature: String,
}

/// Structure of a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStructure {
    /// Path to the source file
    pub path: PathBuf,
    /// Names of top-level functions defined in this file
    pub functions: Vec<String>,
    /// Names of classes or structs defined in this file
    pub classes: Vec<String>,
    /// Names of methods (functions inside classes) in this file
    pub methods: Vec<String>,
    /// Import statements found in this file
    pub imports: Vec<ImportInfo>,
    /// Detailed definition information with line ranges and signatures
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definitions: Vec<DefinitionInfo>,
}

/// Import statement information (spec Section 2.1.4)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportInfo {
    /// Module or package being imported
    pub module: String,
    /// Specific names imported from the module (e.g., `from X import a, b`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Whether this is a `from` import (e.g., `from module import name`)
    #[serde(default)]
    pub is_from: bool,
    /// Import alias (e.g., `import X as Y`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Complete module information (spec Section 2.1.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// Path to the source file for this module
    pub file_path: PathBuf,
    /// Programming language of the module
    pub language: Language,
    /// Module-level docstring, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Import statements in this module
    pub imports: Vec<ImportInfo>,
    /// Top-level functions defined in this module
    pub functions: Vec<FunctionInfo>,
    /// Classes or structs defined in this module
    pub classes: Vec<ClassInfo>,
    /// Module-level constants (Gap 3)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constants: Vec<FieldInfo>,
    /// Intra-file call graph showing function call relationships within this module
    pub call_graph: IntraFileCallGraph,
}

/// Function information with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    /// Name of the function
    pub name: String,
    /// Parameter names (and optional type annotations)
    pub params: Vec<String>,
    /// Return type annotation, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_type: Option<String>,
    /// Docstring or doc comment for this function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Whether this function is a method (defined inside a class/struct)
    #[serde(default)]
    pub is_method: bool,
    /// Whether this function is declared as async
    #[serde(default)]
    pub is_async: bool,
    /// Decorator or annotation names applied to this function
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorators: Vec<String>,
    /// Line number where this function is defined (1-indexed)
    pub line_number: u32,
}

/// Class information with full details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    /// Name of the class or struct
    pub name: String,
    /// Base classes or parent types this class extends
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bases: Vec<String>,
    /// Docstring or doc comment for this class
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Methods defined in this class
    pub methods: Vec<FunctionInfo>,
    /// Fields/properties of the class (Gap 3)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldInfo>,
    /// Decorator or annotation names applied to this class
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorators: Vec<String>,
    /// Line number where this class is defined (1-indexed)
    pub line_number: u32,
}

/// Field or constant information (Gap 3)
///
/// Represents:
/// - Class/struct fields (instance variables, properties)
/// - Module-level constants
/// - Static class variables
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    /// Field name
    pub name: String,
    /// Field type annotation (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    /// Default value (if present)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    /// Whether this is a static/class variable
    #[serde(default)]
    pub is_static: bool,
    /// Whether this is a constant (immutable, UPPER_CASE by convention)
    #[serde(default)]
    pub is_constant: bool,
    /// Visibility modifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Line number where field is defined (1-indexed)
    pub line_number: u32,
}

/// Intra-file call graph
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntraFileCallGraph {
    /// Map from function name to the list of functions it calls
    pub calls: HashMap<String, Vec<String>>,
    /// Reverse map from function name to the list of functions that call it
    pub called_by: HashMap<String, Vec<String>>,
}

// =============================================================================
// Call Graph Types (Layer 2)
// =============================================================================

/// Helper for serde skip_serializing_if on u32 fields.
fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

/// Reference to a function in the codebase, used in call graphs and dead code analysis.
///
/// Equality and hashing are based only on `file` and `name`, so metadata
/// fields do not affect `HashSet`/`HashMap` lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Name of the function
    pub name: String,
    /// Line number where the function starts (1-based, 0 = unknown)
    #[serde(default)]
    pub line: u32,
    /// Function signature (e.g. "def my_func(x, y) -> int")
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// Reference count: how many times this identifier appears across the codebase.
    /// 1 = only the definition, 0 = unknown/not computed.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub ref_count: u32,
    /// Whether this function is public/exported (pub, export, uppercase Go, etc.)
    #[serde(default)]
    pub is_public: bool,
    /// Whether this function is a test function (in test file or test function)
    #[serde(default)]
    pub is_test: bool,
    /// Whether this function is inside a trait/interface/protocol/abstract class
    #[serde(default)]
    pub is_trait_method: bool,
    /// Whether this function has any decorator/annotation
    #[serde(default)]
    pub has_decorator: bool,
    /// Names of decorators/annotations on this function
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decorator_names: Vec<String>,
}

// Equality based on file + name only (metadata is for analysis, not identity)
impl PartialEq for FunctionRef {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.name == other.name
    }
}

impl Eq for FunctionRef {}

// Hash based on file + name only (must match PartialEq)
impl std::hash::Hash for FunctionRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.file.hash(state);
        self.name.hash(state);
    }
}

impl FunctionRef {
    /// Create a new function reference with default (unenriched) metadata.
    ///
    /// All metadata fields default to false/empty, meaning the function
    /// is treated as private with no special attributes. This preserves
    /// backward compatibility with existing call sites.
    pub fn new(file: PathBuf, name: impl Into<String>) -> Self {
        Self {
            file,
            name: name.into(),
            line: 0,
            signature: String::new(),
            ref_count: 0,
            is_public: false,
            is_test: false,
            is_trait_method: false,
            has_decorator: false,
            decorator_names: Vec::new(),
        }
    }
}

impl std::fmt::Display for FunctionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file.display(), self.name)
    }
}

/// Workspace configuration for multi-root projects
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    /// Root directories of the workspace
    pub roots: Vec<PathBuf>,
}

/// Project-wide call graph (spec Section 2.2.1)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectCallGraph {
    edges: HashSet<CallEdge>,
}

/// Edge in the call graph
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallEdge {
    /// Path to the file containing the calling function
    pub src_file: PathBuf,
    /// Name of the calling function
    pub src_func: String,
    /// Path to the file containing the called function
    pub dst_file: PathBuf,
    /// Name of the called function
    pub dst_func: String,
}

// =============================================================================
// Type-Aware Call Graph Types (Phase 7-8: Type Resolution)
// =============================================================================

/// Confidence level for type resolution
///
/// Indicates how confident we are in the type resolution:
/// - High: Explicit annotation or constructor call
/// - Medium: Return type inference or union type
/// - Low: No type info available (fallback to variable name)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    /// Explicit annotation, constructor, or self/this reference
    High,
    /// Return type inference, union type, or interface
    Medium,
    /// Unknown type, fallback to variable name
    #[default]
    Low,
}

impl std::fmt::Display for Confidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Confidence::High => write!(f, "HIGH"),
            Confidence::Medium => write!(f, "MEDIUM"),
            Confidence::Low => write!(f, "LOW"),
        }
    }
}

/// Extended call edge with type resolution metadata
///
/// Used when --type-aware flag is enabled to track:
/// - The resolved receiver type (e.g., "User" instead of "user")
/// - Confidence level of the resolution
/// - Line number of the call site
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypedCallEdge {
    /// Path to the file containing the calling function
    pub src_file: PathBuf,
    /// Name of the calling function
    pub src_func: String,
    /// Path to the file containing the called function
    pub dst_file: PathBuf,
    /// Name of the called function
    pub dst_func: String,
    /// Resolved receiver type (e.g., "User" for user.save())
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
    /// Confidence level of the type resolution
    pub confidence: Confidence,
    /// Line number of the call site
    pub call_site_line: u32,
}

impl TypedCallEdge {
    /// Create a new typed call edge from a basic CallEdge
    pub fn from_call_edge(edge: &CallEdge, line: u32) -> Self {
        Self {
            src_file: edge.src_file.clone(),
            src_func: edge.src_func.clone(),
            dst_file: edge.dst_file.clone(),
            dst_func: edge.dst_func.clone(),
            receiver_type: None,
            confidence: Confidence::Low,
            call_site_line: line,
        }
    }

    /// Create a high-confidence typed call edge
    pub fn high_confidence(
        src_file: PathBuf,
        src_func: String,
        dst_file: PathBuf,
        dst_func: String,
        receiver_type: String,
        line: u32,
    ) -> Self {
        Self {
            src_file,
            src_func,
            dst_file,
            dst_func,
            receiver_type: Some(receiver_type),
            confidence: Confidence::High,
            call_site_line: line,
        }
    }

    /// Create a medium-confidence typed call edge
    pub fn medium_confidence(
        src_file: PathBuf,
        src_func: String,
        dst_file: PathBuf,
        dst_func: String,
        receiver_type: String,
        line: u32,
    ) -> Self {
        Self {
            src_file,
            src_func,
            dst_file,
            dst_func,
            receiver_type: Some(receiver_type),
            confidence: Confidence::Medium,
            call_site_line: line,
        }
    }

    /// Convert to basic CallEdge (loses type info)
    pub fn to_call_edge(&self) -> CallEdge {
        CallEdge {
            src_file: self.src_file.clone(),
            src_func: self.src_func.clone(),
            dst_file: self.dst_file.clone(),
            dst_func: self.dst_func.clone(),
        }
    }
}

/// Statistics on type resolution (T17 mitigation)
///
/// Provides observability into how well type resolution worked,
/// helping users understand if --type-aware is useful for their codebase.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeResolutionStats {
    /// Whether type-aware analysis was enabled
    pub enabled: bool,
    /// Number of calls resolved with HIGH confidence
    pub resolved_high_confidence: usize,
    /// Number of calls resolved with MEDIUM confidence
    pub resolved_medium_confidence: usize,
    /// Number of calls that fell back to variable name (LOW confidence)
    pub fallback_used: usize,
    /// Total number of call sites analyzed
    pub total_call_sites: usize,
}

impl TypeResolutionStats {
    /// Create stats with type-aware enabled
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Record a high-confidence resolution
    pub fn record_high(&mut self) {
        self.resolved_high_confidence += 1;
        self.total_call_sites += 1;
    }

    /// Record a medium-confidence resolution
    pub fn record_medium(&mut self) {
        self.resolved_medium_confidence += 1;
        self.total_call_sites += 1;
    }

    /// Record a fallback (low confidence)
    pub fn record_fallback(&mut self) {
        self.fallback_used += 1;
        self.total_call_sites += 1;
    }

    /// Get the percentage of successfully resolved calls (HIGH + MEDIUM)
    pub fn resolution_rate(&self) -> f64 {
        if self.total_call_sites == 0 {
            return 0.0;
        }
        let resolved = self.resolved_high_confidence + self.resolved_medium_confidence;
        (resolved as f64 / self.total_call_sites as f64) * 100.0
    }

    /// Format as human-readable summary
    pub fn summary(&self) -> String {
        if !self.enabled {
            return "Type resolution: disabled".to_string();
        }
        let resolved = self.resolved_high_confidence + self.resolved_medium_confidence;
        format!(
            "Type-aware resolution: {}/{} calls resolved ({} high, {} medium confidence)",
            resolved,
            self.total_call_sites,
            self.resolved_high_confidence,
            self.resolved_medium_confidence
        )
    }
}

impl ProjectCallGraph {
    /// Create a new empty call graph
    pub fn new() -> Self {
        Self {
            edges: HashSet::new(),
        }
    }

    /// Iterate over all edges
    pub fn edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.edges.iter()
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: CallEdge) {
        self.edges.insert(edge);
    }

    /// Check if the graph contains an edge
    pub fn contains(&self, edge: &CallEdge) -> bool {
        self.edges.contains(edge)
    }

    /// Get the number of edges
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Check if graph is empty
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

// =============================================================================
// Impact Analysis Types (spec Section 2.2.2)
// =============================================================================

/// Impact analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    /// Map from target function name to its caller tree
    pub targets: HashMap<String, CallerTree>,
    /// Total number of target functions analyzed
    pub total_targets: usize,
    /// Type resolution statistics (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_resolution: Option<TypeResolutionStats>,
}

/// Tree of callers for impact analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallerTree {
    /// Name of the function at this node
    pub function: String,
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Number of direct callers of this function
    pub caller_count: usize,
    /// Recursive tree of callers (callers of callers)
    pub callers: Vec<CallerTree>,
    /// Whether the caller tree was truncated due to depth limits
    #[serde(default)]
    pub truncated: bool,
    /// Optional note about this node (e.g., truncation reason)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Confidence of type resolution for this caller (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Resolved receiver type (when --type-aware is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_type: Option<String>,
}

// =============================================================================
// Dead Code Types (spec Section 2.2.3)
// =============================================================================

/// Dead code analysis report
///
/// Functions are classified into two tiers:
/// - `dead_functions`: Definitely dead (private/unenriched + uncalled + no special metadata)
/// - `possibly_dead`: Public/exported but uncalled (may be API surface)
///
/// The `dead_percentage` is calculated from `dead_functions` only (definitely dead).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeReport {
    /// Functions that are definitely dead (private and uncalled)
    pub dead_functions: Vec<FunctionRef>,
    /// Public/exported functions that are uncalled (may be intentional API surface)
    #[serde(default)]
    pub possibly_dead: Vec<FunctionRef>,
    /// Map from file path to names of dead functions in that file
    pub by_file: HashMap<PathBuf, Vec<String>>,
    /// Count of definitely-dead functions
    pub total_dead: usize,
    /// Number of possibly-dead (public but uncalled) functions
    #[serde(default)]
    pub total_possibly_dead: usize,
    /// Total number of functions in the analyzed codebase
    pub total_functions: usize,
    /// Percentage of definitely-dead functions (excludes possibly_dead)
    pub dead_percentage: f64,
}

// =============================================================================
// Importers Types (spec Section 2.2.4)
// =============================================================================

/// Report of files importing a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportersReport {
    /// Name of the module being queried
    pub module: String,
    /// Files that import this module
    pub importers: Vec<ImporterInfo>,
    /// Total number of importers found
    pub total: usize,
}

/// Information about a file that imports a module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImporterInfo {
    /// Path to the file that contains the import
    pub file: PathBuf,
    /// Line number of the import statement (1-indexed)
    pub line: u32,
    /// Full text of the import statement
    pub import_statement: String,
}

// =============================================================================
// Architecture Types (spec Section 2.2.5)
// =============================================================================

/// Architecture analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureReport {
    /// Functions in the entry layer (called by external consumers, call others)
    pub entry_layer: Vec<FunctionRef>,
    /// Functions in the middle/service layer (called by entry, call leaf)
    pub middle_layer: Vec<FunctionRef>,
    /// Functions in the leaf/utility layer (called by others, call nothing external)
    pub leaf_layer: Vec<FunctionRef>,
    /// Per-directory statistics (function counts, call directions)
    pub directories: HashMap<PathBuf, DirStats>,
    /// Detected circular dependencies between directories
    pub circular_dependencies: Vec<CircularDep>,
    /// Inferred architectural layer for each directory
    pub inferred_layers: HashMap<PathBuf, LayerType>,
}

/// Directory statistics for architecture analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirStats {
    /// Names of functions defined in this directory
    pub functions: Vec<String>,
    /// Number of outgoing calls from this directory to other directories
    pub calls_out: usize,
    /// Number of incoming calls from other directories into this directory
    pub calls_in: usize,
}

/// Circular dependency between directories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircularDep {
    /// First directory in the circular dependency
    pub a: PathBuf,
    /// Second directory in the circular dependency
    pub b: PathBuf,
}

/// Inferred layer type for a directory
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerType {
    /// Entry point layer (API handlers, CLI commands, main functions)
    Entry,
    /// Service/business logic layer (orchestrates utilities)
    Service,
    /// Utility/leaf layer (pure helpers, no external dependencies)
    Utility,
    /// Dynamic dispatch layer (virtual calls, trait objects, callbacks)
    DynamicDispatch,
}

// =============================================================================
// CFG Types (Layer 3, spec Section 2.3)
// =============================================================================

/// Control flow graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgInfo {
    /// Name of the function this CFG represents
    pub function: String,
    /// Basic blocks in the control flow graph
    pub blocks: Vec<CfgBlock>,
    /// Edges connecting basic blocks
    pub edges: Vec<CfgEdge>,
    /// ID of the entry basic block
    pub entry_block: usize,
    /// IDs of exit basic blocks (return/end points)
    pub exit_blocks: Vec<usize>,
    /// Cyclomatic complexity of this function
    pub cyclomatic_complexity: u32,
    /// CFGs for nested/inner functions defined within this function
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub nested_functions: HashMap<String, CfgInfo>,
}

/// Basic block in CFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgBlock {
    /// Unique identifier for this basic block
    pub id: usize,
    /// Classification of this basic block (entry, branch, loop, etc.)
    pub block_type: BlockType,
    /// Line range covered by this block (start_line, end_line), 1-indexed
    pub lines: (u32, u32),
    /// Function calls made within this basic block
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
}

/// Type of basic block
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    /// Function entry point
    Entry,
    /// Conditional branch (if/else, match)
    Branch,
    /// Loop condition check (for, while header)
    LoopHeader,
    /// Loop body statements
    LoopBody,
    /// Return statement
    Return,
    /// Function exit point
    Exit,
    /// Sequential statement block
    Body,
}

/// Edge in CFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgEdge {
    /// ID of the source basic block
    pub from: usize,
    /// ID of the target basic block
    pub to: usize,
    /// Classification of this edge (true branch, false branch, unconditional, etc.)
    pub edge_type: EdgeType,
    /// Condition expression for conditional edges (e.g., `x > 0`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Type of CFG edge
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// True branch of a conditional
    True,
    /// False branch of a conditional
    False,
    /// Unconditional flow (fallthrough, goto)
    Unconditional,
    /// Back edge to a loop header
    BackEdge,
    /// Break out of a loop
    Break,
    /// Continue to next loop iteration
    Continue,
}

/// Complexity metrics (spec Section 2.3.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    /// Name of the function being measured
    pub function: String,
    /// Cyclomatic complexity (number of independent paths)
    pub cyclomatic: u32,
    /// Cognitive complexity (how hard the function is to understand)
    pub cognitive: u32,
    /// Maximum nesting depth of control structures
    pub nesting_depth: u32,
    /// Number of lines of code in the function
    pub lines_of_code: u32,
}

// =============================================================================
// DFG Types (Layer 4, spec Section 2.4)
// =============================================================================

/// Data flow graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfgInfo {
    /// Name of the function this data flow graph represents
    pub function: String,
    /// All variable references (definitions, updates, uses) in the function
    pub refs: Vec<VarRef>,
    /// Data flow edges (def-use chains) connecting definitions to their uses
    pub edges: Vec<DataflowEdge>,
    /// Names of all variables tracked in this function
    pub variables: Vec<String>,
}

/// Variable reference in DFG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarRef {
    /// Name of the variable being referenced
    pub name: String,
    /// Whether this is a definition, update, or use of the variable
    pub ref_type: RefType,
    /// Line number of this reference (1-indexed)
    pub line: u32,
    /// Column number of this reference (0-indexed)
    pub column: u32,
    /// Language-specific construct context (e.g., "augmented_assignment", "destructuring")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<VarRefContext>,
    /// Statement group ID for parallel assignments (e.g., a, b = b, a)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<u32>,
}

/// Context for language-specific variable reference patterns
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VarRefContext {
    // Python-specific
    /// x += 1: both use and def in same statement
    AugmentedAssignment,
    /// a, b = b, a: parallel semantics (RHS evaluated before LHS)
    MultipleAssignment,
    /// n := expr: walrus operator, def in expression context
    WalrusOperator,
    /// [x for x in ...]: x is scoped to comprehension
    ComprehensionScope,
    /// match case (x, y): pattern binding
    MatchBinding,
    /// global x / nonlocal x: external scope reference
    GlobalNonlocal,

    // TypeScript/JavaScript-specific
    /// const {a, b} = obj: destructuring creates multiple defs
    Destructuring,
    /// Closure captures variable by reference
    ClosureCapture,
    /// Optional chaining (?.) short-circuit
    OptionalChain,

    // Go-specific
    /// x := 1: short declaration (may be new var or redefinition)
    ShortDeclaration,
    /// a, b := f(): multiple return values
    MultipleReturn,
    /// _ = x: blank identifier (not a real definition)
    BlankIdentifier,
    /// defer log(x): captured at defer point
    DeferCapture,

    // Rust-specific
    /// let x = 1; let x = 2: shadowing creates NEW variable
    Shadowing,
    /// let (a, b) = tuple: pattern binding
    PatternBinding,
    /// let b = a: ownership move ends a's liveness
    OwnershipMove,
    /// match x { Some(v) => ... }: binding scoped to arm
    MatchArmBinding,
}

/// Type of variable reference
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RefType {
    /// Variable definition (first assignment)
    Definition,
    /// Variable update (reassignment or mutation)
    Update,
    /// Variable use (read)
    Use,
}

/// Data flow edge (def-use chain)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataflowEdge {
    /// Name of the variable flowing from definition to use
    pub var: String,
    /// Line number where the variable is defined (1-indexed)
    pub def_line: u32,
    /// Line number where the variable is used (1-indexed)
    pub use_line: u32,
    /// Full variable reference at the definition site
    pub def_ref: VarRef,
    /// Full variable reference at the use site
    pub use_ref: VarRef,
}

// =============================================================================
// PDG Types (Layer 5, spec Section 2.5)
// =============================================================================

/// Program dependence graph information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgInfo {
    /// Name of the function this PDG represents
    pub function: String,
    /// Control flow graph for this function
    pub cfg: CfgInfo,
    /// Data flow graph for this function
    pub dfg: DfgInfo,
    /// Nodes in the program dependence graph
    pub nodes: Vec<PdgNode>,
    /// Dependence edges (control and data) between PDG nodes
    pub edges: Vec<PdgEdge>,
}

/// Node in PDG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgNode {
    /// Unique identifier for this PDG node
    pub id: usize,
    /// Type of statement at this node (e.g., "assignment", "branch", "call")
    pub node_type: String,
    /// Line range covered by this node (start_line, end_line), 1-indexed
    pub lines: (u32, u32),
    /// Variables defined at this node
    pub definitions: Vec<String>,
    /// Variables used at this node
    pub uses: Vec<String>,
}

/// Edge in PDG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdgEdge {
    /// ID of the source PDG node
    pub source_id: usize,
    /// ID of the target PDG node
    pub target_id: usize,
    /// Whether this is a control or data dependence
    pub dep_type: DependenceType,
    /// Human-readable label describing the dependence (e.g., variable name)
    pub label: String,
}

/// Type of dependence in PDG
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DependenceType {
    /// Control dependence (execution of target depends on a branch decision)
    Control,
    /// Data dependence (target uses a value defined by source)
    Data,
}

/// Slice direction for program slicing (spec Section 2.5.2)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SliceDirection {
    /// Backward slice: find all statements that affect the slicing criterion
    Backward,
    /// Forward slice: find all statements affected by the slicing criterion
    Forward,
}

impl std::str::FromStr for SliceDirection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "backward" | "back" | "b" => Ok(SliceDirection::Backward),
            "forward" | "fwd" | "f" => Ok(SliceDirection::Forward),
            _ => Err(format!(
                "Invalid direction: {}. Expected 'backward' or 'forward'",
                s
            )),
        }
    }
}

/// Thin slice result (spec Section 2.5.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinSliceResult {
    /// Line numbers in the thin (data-only) slice
    pub lines: HashSet<u32>,
    /// Line numbers in the full (data + control) slice for comparison
    pub full_slice_lines: HashSet<u32>,
    /// Percentage reduction from full slice to thin slice
    pub reduction_pct: f64,
}

// =============================================================================
// Search Types (spec Section 2.6)
// =============================================================================

/// Search match result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    /// Path to the file containing the match
    pub file: PathBuf,
    /// Line number of the match (1-indexed)
    pub line: u32,
    /// Content of the matching line
    pub content: String,
    /// Surrounding context lines (before and after the match)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<String>>,
}

/// BM25 search result (spec Section 2.6.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Result {
    /// Path to the file containing the result
    pub file_path: PathBuf,
    /// BM25 relevance score
    pub score: f64,
    /// Start line of the matching snippet (1-indexed)
    pub line_start: u32,
    /// End line of the matching snippet (1-indexed)
    pub line_end: u32,
    /// Text snippet containing the match
    pub snippet: String,
    /// Query terms that matched in this result
    pub matched_terms: Vec<String>,
}

/// Embedding client placeholder for hybrid search
pub struct EmbeddingClient;

/// Hybrid search result (spec Section 2.6.3)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridResult {
    /// Path to the file containing the result
    pub file_path: PathBuf,
    /// Reciprocal Rank Fusion score combining BM25 and dense retrieval
    pub rrf_score: f64,
    /// Rank from the BM25 retriever, if this result appeared in BM25 results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_rank: Option<usize>,
    /// Rank from the dense (embedding) retriever, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_rank: Option<usize>,
    /// Raw BM25 score, if this result appeared in BM25 results
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bm25_score: Option<f64>,
    /// Raw dense (cosine similarity) score, if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dense_score: Option<f64>,
    /// Text snippet containing the match
    pub snippet: String,
    /// Query terms that matched in this result
    pub matched_terms: Vec<String>,
}

/// Hybrid search report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchReport {
    /// Ranked search results after reciprocal rank fusion
    pub results: Vec<HybridResult>,
    /// Original search query string
    pub query: String,
    /// Total number of candidate results before ranking
    pub total_candidates: usize,
    /// Number of results found only by BM25 (not dense retrieval)
    pub bm25_only: usize,
    /// Number of results found only by dense retrieval (not BM25)
    pub dense_only: usize,
    /// Number of results found by both retrievers
    pub overlap: usize,
    /// Fallback mode used when dense retrieval is unavailable (e.g., "bm25_only")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_mode: Option<String>,
}

// =============================================================================
// Context Types (spec Section 2.7)
// =============================================================================

/// Relevant context for LLM (spec Section 2.7.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantContext {
    /// Name of the entry point function for context gathering
    pub entry_point: String,
    /// Maximum call depth traversed to gather context
    pub depth: usize,
    /// Functions reachable from the entry point within the specified depth
    pub functions: Vec<FunctionContext>,
}

/// Function context for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionContext {
    /// Name of the function
    pub name: String,
    /// Path to the file containing this function
    pub file: PathBuf,
    /// Line number where the function is defined (1-indexed)
    pub line: u32,
    /// Full function signature
    pub signature: String,
    /// Docstring or doc comment, if present
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docstring: Option<String>,
    /// Names of functions called by this function
    pub calls: Vec<String>,
    /// Number of basic blocks in the function's CFG
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocks: Option<usize>,
    /// Cyclomatic complexity of the function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cyclomatic: Option<u32>,
}

impl RelevantContext {
    /// Format for LLM consumption
    pub fn to_llm_string(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Context for: {}\n\n", self.entry_point));
        for func in &self.functions {
            output.push_str(&format!("## {}\n", func.name));
            output.push_str(&format!("File: {}:{}\n", func.file.display(), func.line));
            output.push_str(&format!("Signature: {}\n", func.signature));
            if let Some(doc) = &func.docstring {
                output.push_str(&format!("Doc: {}\n", doc));
            }
            if !func.calls.is_empty() {
                output.push_str(&format!("Calls: {}\n", func.calls.join(", ")));
            }
            output.push('\n');
        }
        output
    }
}

// =============================================================================
// Change Impact Types (spec Section 2.7.2)
// =============================================================================

/// Change impact report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeImpactReport {
    /// Files that were changed (from git diff or explicit input)
    pub changed_files: Vec<PathBuf>,
    /// Test files potentially affected by the changes
    pub affected_tests: Vec<PathBuf>,
    /// Functions transitively affected by the changes
    pub affected_functions: Vec<FunctionRef>,
    /// Method used to detect impacts (e.g., "call_graph", "import_graph")
    pub detection_method: String,
}

// =============================================================================
// Quality Types (spec Section 2.8)
// =============================================================================

/// Threshold preset for quality checks
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ThresholdPreset {
    /// Strict thresholds (lower tolerance for smells and complexity)
    Strict,
    /// Default thresholds (balanced tolerance)
    #[default]
    Default,
    /// Relaxed thresholds (higher tolerance, fewer warnings)
    Relaxed,
}

/// Code smell type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SmellType {
    /// Class that does too much (high number of methods, fields, or responsibilities)
    GodClass,
    /// Method with too many lines of code or excessive complexity
    LongMethod,
    /// Method that uses another class's data more than its own
    FeatureEnvy,
    /// Groups of fields that frequently appear together across classes
    DataClumps,
    /// Function with too many parameters
    LongParameterList,
}

/// Code smells report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellsReport {
    /// Individual code smell findings
    pub smells: Vec<SmellFinding>,
    /// Number of files analyzed for code smells
    pub files_analyzed: usize,
    /// Total number of code smells found
    pub total_smells: usize,
}

/// Individual smell finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmellFinding {
    /// Path to the file containing the smell
    pub file: PathBuf,
    /// Line number where the smell occurs (1-indexed)
    pub line: u32,
    /// Classification of the code smell
    pub smell_type: SmellType,
    /// Human-readable description of the smell
    pub description: String,
    /// Suggested fix or refactoring
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Maintainability report (spec Section 2.8.2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaintainabilityReport {
    /// Per-file maintainability index results
    pub files: Vec<FileMI>,
    /// Aggregate summary of maintainability across all files
    pub summary: MISummary,
}

/// File maintainability index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMI {
    /// Path to the source file
    pub path: PathBuf,
    /// Maintainability Index score (0-100, higher is better)
    pub mi: f64,
    /// Letter grade (A, B, or C) derived from the MI score
    pub grade: char,
    /// Halstead metrics used in MI calculation, if computed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halstead: Option<HalsteadMetrics>,
}

/// MI summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MISummary {
    /// Average Maintainability Index across all files
    pub average_mi: f64,
    /// Lowest Maintainability Index (worst file)
    pub min_mi: f64,
    /// Highest Maintainability Index (best file)
    pub max_mi: f64,
    /// Number of files included in the summary
    pub files_analyzed: usize,
}

/// Halstead metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HalsteadMetrics {
    /// Number of distinct operators and operands (n = n1 + n2)
    pub vocabulary: u32,
    /// Total number of operators and operands (N = N1 + N2)
    pub length: u32,
    /// Volume: N * log2(n), measures information content
    pub volume: f64,
    /// Difficulty: (n1/2) * (N2/n2), measures error-proneness
    pub difficulty: f64,
    /// Effort: D * V, measures cognitive effort to understand
    pub effort: f64,
}

// =============================================================================
// Security Types (spec Section 2.9)
// =============================================================================

/// Severity level for security findings
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Low severity (informational, minor risk)
    Low,
    /// Medium severity (moderate risk, should be addressed)
    Medium,
    /// High severity (significant risk, needs prompt attention)
    High,
    /// Critical severity (immediate risk, must be fixed urgently)
    Critical,
}

/// Secrets report (spec Section 2.9.1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsReport {
    /// Individual secret findings (hardcoded keys, tokens, passwords)
    pub findings: Vec<SecretFinding>,
    /// Number of files scanned for secrets
    pub files_scanned: usize,
    /// Number of secret patterns checked
    pub patterns_checked: usize,
    /// Aggregate summary of findings by severity
    pub summary: SecretsSummary,
}

/// Secret finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    /// Path to the file containing the secret
    pub file: PathBuf,
    /// Line number where the secret was found (1-indexed)
    pub line: u32,
    /// Name of the pattern that matched (e.g., "AWS_ACCESS_KEY")
    pub pattern: String,
    /// Severity of the finding
    pub severity: Severity,
    /// Partially masked value showing the secret type without exposing it
    pub masked_value: String,
}

/// Secrets summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretsSummary {
    /// Total number of secret findings
    pub total_findings: usize,
    /// Breakdown of findings by severity level
    pub by_severity: HashMap<String, usize>,
}

/// Vulnerability type (spec Section 2.9.2)
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VulnType {
    /// SQL injection via unsanitized user input in queries
    SqlInjection,
    /// Cross-site scripting via unescaped output
    Xss,
    /// OS command injection via unsanitized shell arguments
    CommandInjection,
    /// Path traversal via unvalidated file paths
    PathTraversal,
    /// Server-side request forgery via user-controlled URLs
    Ssrf,
    /// Unsafe deserialization of untrusted data
    Deserialization,
}

/// Vulnerability report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnReport {
    /// Individual vulnerability findings
    pub findings: Vec<VulnFinding>,
    /// Number of files scanned for vulnerabilities
    pub files_scanned: usize,
    /// Aggregate summary by type and severity
    pub summary: VulnSummary,
}

/// Vulnerability finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnFinding {
    /// Path to the file containing the vulnerability
    pub file: PathBuf,
    /// Line number where the vulnerability occurs (1-indexed)
    pub line: u32,
    /// Classification of the vulnerability
    pub vuln_type: VulnType,
    /// Severity of the vulnerability
    pub severity: Severity,
    /// Human-readable description of the vulnerability
    pub description: String,
    /// Taint source (where untrusted data enters), if identified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Taint sink (where untrusted data is consumed unsafely), if identified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sink: Option<String>,
}

/// Vulnerability summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnSummary {
    /// Total number of vulnerability findings
    pub total_findings: usize,
    /// Breakdown of findings by vulnerability type
    pub by_type: HashMap<String, usize>,
    /// Breakdown of findings by severity level
    pub by_severity: HashMap<String, usize>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_from_extension() {
        assert_eq!(Language::from_extension(".py"), Some(Language::Python));
        assert_eq!(Language::from_extension(".ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension(".tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension(".js"), Some(Language::JavaScript));
        assert_eq!(Language::from_extension(".go"), Some(Language::Go));
        assert_eq!(Language::from_extension(".rs"), Some(Language::Rust));
        assert_eq!(Language::from_extension(".java"), Some(Language::Java));
        assert_eq!(Language::from_extension(".unknown"), None);
    }

    #[test]
    fn test_language_from_extension_without_dot() {
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_extension_case_insensitive() {
        assert_eq!(Language::from_extension(".PY"), Some(Language::Python));
        assert_eq!(Language::from_extension(".Ts"), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_serde_roundtrip() {
        for lang in Language::all() {
            let json = serde_json::to_string(lang).unwrap();
            let parsed: Language = serde_json::from_str(&json).unwrap();
            assert_eq!(*lang, parsed);
        }
    }

    #[test]
    fn test_language_all_18_variants() {
        assert_eq!(Language::all().len(), 18);
    }

    #[test]
    fn test_language_from_str() {
        assert_eq!("python".parse::<Language>().unwrap(), Language::Python);
        assert_eq!("py".parse::<Language>().unwrap(), Language::Python);
        assert_eq!(
            "typescript".parse::<Language>().unwrap(),
            Language::TypeScript
        );
        assert_eq!("ts".parse::<Language>().unwrap(), Language::TypeScript);
        assert_eq!("golang".parse::<Language>().unwrap(), Language::Go);
        assert!("unknown".parse::<Language>().is_err());
    }

    #[test]
    fn test_function_ref_equality() {
        let ref1 = FunctionRef::new(PathBuf::from("test.py"), "func");
        let ref2 = FunctionRef::new(PathBuf::from("test.py"), "func");
        let ref3 = FunctionRef::new(PathBuf::from("test.py"), "other");

        assert_eq!(ref1, ref2);
        assert_ne!(ref1, ref3);
    }

    #[test]
    fn test_function_ref_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(FunctionRef::new(PathBuf::from("test.py"), "func"));
        set.insert(FunctionRef::new(PathBuf::from("test.py"), "func"));

        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_project_call_graph() {
        let mut graph = ProjectCallGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.edge_count(), 0);

        let edge = CallEdge {
            src_file: PathBuf::from("a.py"),
            src_func: "foo".to_string(),
            dst_file: PathBuf::from("b.py"),
            dst_func: "bar".to_string(),
        };

        graph.add_edge(edge.clone());
        assert!(!graph.is_empty());
        assert_eq!(graph.edge_count(), 1);
        assert!(graph.contains(&edge));
    }

    #[test]
    fn test_slice_direction_from_str() {
        assert_eq!(
            "backward".parse::<SliceDirection>().unwrap(),
            SliceDirection::Backward
        );
        assert_eq!(
            "forward".parse::<SliceDirection>().unwrap(),
            SliceDirection::Forward
        );
        assert_eq!(
            "back".parse::<SliceDirection>().unwrap(),
            SliceDirection::Backward
        );
        assert_eq!(
            "fwd".parse::<SliceDirection>().unwrap(),
            SliceDirection::Forward
        );
        assert!("invalid".parse::<SliceDirection>().is_err());
    }

    #[test]
    fn test_relevant_context_to_llm_string() {
        let ctx = RelevantContext {
            entry_point: "main".to_string(),
            depth: 2,
            functions: vec![FunctionContext {
                name: "main".to_string(),
                file: PathBuf::from("app.py"),
                line: 10,
                signature: "def main() -> None".to_string(),
                docstring: Some("Entry point".to_string()),
                calls: vec!["helper".to_string()],
                blocks: Some(3),
                cyclomatic: Some(2),
            }],
        };

        let output = ctx.to_llm_string();
        assert!(output.contains("Context for: main"));
        assert!(output.contains("app.py:10"));
        assert!(output.contains("def main() -> None"));
        assert!(output.contains("Entry point"));
        assert!(output.contains("Calls: helper"));
    }

    #[test]
    fn test_language_from_path_typescript() {
        let path = std::path::Path::new("src/app.ts");
        assert_eq!(Language::from_path(path), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_path_tsx() {
        let path = std::path::Path::new("components/Button.tsx");
        assert_eq!(Language::from_path(path), Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_path_go() {
        let path = std::path::Path::new("main.go");
        assert_eq!(Language::from_path(path), Some(Language::Go));
    }

    #[test]
    fn test_language_from_path_python() {
        let path = std::path::Path::new("app.py");
        assert_eq!(Language::from_path(path), Some(Language::Python));
    }

    #[test]
    fn test_language_from_path_rust() {
        let path = std::path::Path::new("lib.rs");
        assert_eq!(Language::from_path(path), Some(Language::Rust));
    }

    #[test]
    fn test_language_from_path_ocaml() {
        let path = std::path::Path::new("lib.ml");
        assert_eq!(Language::from_path(path), Some(Language::Ocaml));
    }

    #[test]
    fn test_language_from_path_unknown() {
        let path = std::path::Path::new("readme.txt");
        assert_eq!(Language::from_path(path), None);
    }

    #[test]
    fn test_language_from_path_no_extension() {
        let path = std::path::Path::new("Makefile");
        assert_eq!(Language::from_path(path), None);
    }

    #[test]
    fn test_language_from_directory_detects_majority() {
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_majority");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        // Create 3 TypeScript files and 1 Python file
        fs::write(tmp.join("a.ts"), "").unwrap();
        fs::write(tmp.join("b.ts"), "").unwrap();
        fs::write(tmp.join("c.tsx"), "").unwrap();
        fs::write(tmp.join("d.py"), "").unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, Some(Language::TypeScript));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_empty_returns_none() {
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_empty");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, None);

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_checks_subdirs() {
        use std::fs;
        let tmp = std::env::temp_dir().join("tldr_test_from_dir_subdirs");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();

        // No files at top level, only in subdirectory
        fs::write(tmp.join("src/main.go"), "").unwrap();
        fs::write(tmp.join("src/util.go"), "").unwrap();

        let detected = Language::from_directory(&tmp);
        assert_eq!(detected, Some(Language::Go));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_language_from_directory_nonexistent_returns_none() {
        let path = std::path::Path::new("/tmp/tldr_nonexistent_dir_xyz");
        let detected = Language::from_directory(path);
        assert_eq!(detected, None);
    }
}
