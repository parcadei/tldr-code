//! Foundational types for Builder V2 call graph construction.
//!
//! This is the leaf module in the builder_v2 dependency graph -- it has no
//! dependencies on other new modules (scanner, var_types, module_path, imports,
//! resolution). Contains config, error, diagnostics, index types, and parser
//! utilities.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;
use tree_sitter::{Parser, Tree};

use super::cross_file_types::CallGraphIR;

// =============================================================================
// Python Built-in Types (Phase 2: Parity Fix)
// =============================================================================

/// Python built-in types to skip when resolving cross-file calls.
/// These are not actual cross-file dependencies and inflate edge counts.
pub(crate) const PYTHON_BUILTINS: &[&str] = &[
    // Exceptions
    "Exception",
    "ValueError",
    "TypeError",
    "KeyError",
    "IndexError",
    "AttributeError",
    "RuntimeError",
    "StopIteration",
    "OSError",
    "FileNotFoundError",
    "ImportError",
    "ModuleNotFoundError",
    // Built-in types
    "int",
    "str",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "bytes",
    "bytearray",
    "frozenset",
    "object",
    "type",
    // Built-in functions that look like constructors
    "super",
    "classmethod",
    "staticmethod",
    "property",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sorted",
    "reversed",
    "len",
    "print",
    "open",
    "iter",
    "next",
    "isinstance",
    "issubclass",
    "getattr",
    "setattr",
    "hasattr",
    "delattr",
];

// =============================================================================
// BuildConfig (Spec Section 14.3)
// =============================================================================

/// Configuration for call graph building.
///
/// # Defaults
/// - `language`: Empty string (must be set by caller)
/// - `use_workspace_config`: false
/// - `workspace_roots`: empty
/// - `use_type_resolution`: false
/// - `respect_ignore`: true (respect .tldrignore patterns)
/// - `parallelism`: 0 (auto-detect based on CPU cores)
/// - `verbose`: false
#[derive(Clone, Debug)]
pub struct BuildConfig {
    /// Language to analyze (e.g., "python", "typescript")
    pub language: String,

    /// Enable workspace config filtering (monorepo support)
    pub use_workspace_config: bool,

    /// Workspace roots to include when workspace filtering is enabled.
    /// Paths are relative to project root unless absolute.
    pub workspace_roots: Vec<PathBuf>,

    /// Enable type-aware method resolution
    pub use_type_resolution: bool,

    /// Respect .tldrignore patterns
    pub respect_ignore: bool,

    /// Maximum parallel threads (0 = auto-detect)
    pub parallelism: usize,

    /// Enable verbose logging
    pub verbose: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            language: String::new(),
            use_workspace_config: false,
            workspace_roots: Vec::new(),
            use_type_resolution: false,
            respect_ignore: true,
            parallelism: 0, // Auto-detect
            verbose: false,
        }
    }
}

// =============================================================================
// BuildError (Spec Section 14.10)
// =============================================================================

/// Errors during call graph building.
#[derive(Debug, Error)]
pub enum BuildError {
    /// Project root directory not found
    #[error("Project root not found: {0}")]
    RootNotFound(PathBuf),

    /// Requested language is not supported
    #[error("Unsupported language: {0}")]
    UnsupportedLanguage(String),

    /// Error reading or parsing workspace configuration
    #[error("Workspace config error: {0}")]
    WorkspaceConfig(String),

    /// I/O error during file operations
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Error parsing a source file
    #[error("Parse error in {file}: {message}")]
    ParseError {
        /// Path to the file that failed to parse
        file: PathBuf,
        /// Description of the parse error
        message: String,
    },

    /// Error in thread pool operations
    #[error("Thread pool error: {0}")]
    ThreadPool(String),

    /// Feature not enabled at compile time (M3.7 mitigation)
    ///
    /// Returned when `use_experimental=true` is passed to `build_call_graph`
    /// but the `experimental_callgraph` feature is not enabled.
    #[error("Feature not enabled: {feature}. {message}")]
    FeatureNotEnabled {
        /// The feature that was requested but not enabled
        feature: String,
        /// Instructions or additional context
        message: String,
    },
}

// =============================================================================
// BuildResult and BuildDiagnostics (Mitigation M2.1)
// =============================================================================

/// Result of a call graph build operation.
///
/// Contains both the built graph and any diagnostics (warnings, errors)
/// collected during the build process.
#[derive(Debug)]
pub struct BuildResult {
    /// The constructed call graph IR
    pub graph: CallGraphIR,

    /// Diagnostics collected during build
    pub diagnostics: BuildDiagnostics,
}

/// Diagnostics collected during call graph building.
///
/// Implements error aggregation per Mitigation M2.1:
/// "Implement error aggregation with configurable strategy.
/// Return both the graph AND a list of warnings/errors."
#[derive(Debug, Default)]
pub struct BuildDiagnostics {
    /// Files that failed to parse
    pub parse_errors: Vec<ParseDiagnostic>,

    /// Warnings during import/call resolution
    pub resolution_warnings: Vec<ResolutionWarning>,

    /// Files that were skipped (with reason)
    pub skipped_files: Vec<(PathBuf, SkipReason)>,
}

impl BuildDiagnostics {
    /// Create empty diagnostics
    pub fn new() -> Self {
        Self::default()
    }

    /// Sort diagnostics for deterministic output.
    ///
    /// Per Mitigation M2.12: "Sort diagnostics by file path and line number
    /// before returning" to ensure deterministic output regardless of
    /// parallel execution order.
    pub fn sort(&mut self) {
        self.parse_errors.sort();
        self.resolution_warnings.sort();
        self.skipped_files.sort_by(|a, b| a.0.cmp(&b.0));
    }

    /// Check if there are any errors (not just warnings)
    pub fn has_errors(&self) -> bool {
        !self.parse_errors.is_empty()
    }

    /// Total number of diagnostic messages
    pub fn count(&self) -> usize {
        self.parse_errors.len() + self.resolution_warnings.len() + self.skipped_files.len()
    }
}

/// Diagnostic for a parse error in a specific file.
#[derive(Debug, Clone)]
pub struct ParseDiagnostic {
    /// Path to the file that failed
    pub file: PathBuf,

    /// Line number where error occurred (0 if unknown)
    pub line: u32,

    /// Error message
    pub message: String,
}

// Implement Ord for deterministic sorting (M2.12)
impl PartialEq for ParseDiagnostic {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file && self.line == other.line && self.message == other.message
    }
}

impl Eq for ParseDiagnostic {}

impl PartialOrd for ParseDiagnostic {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ParseDiagnostic {
    fn cmp(&self, other: &Self) -> Ordering {
        self.file
            .cmp(&other.file)
            .then(self.line.cmp(&other.line))
            .then(self.message.cmp(&other.message))
    }
}

/// Warning during import or call resolution.
#[derive(Debug, Clone)]
pub struct ResolutionWarning {
    /// Path to the file where warning occurred
    pub file: PathBuf,

    /// Line number (0 if unknown)
    pub line: u32,

    /// The import or call that couldn't be resolved
    pub target: String,

    /// Reason for the warning
    pub reason: String,
}

// Implement Ord for deterministic sorting (M2.12)
impl PartialEq for ResolutionWarning {
    fn eq(&self, other: &Self) -> bool {
        self.file == other.file
            && self.line == other.line
            && self.target == other.target
            && self.reason == other.reason
    }
}

impl Eq for ResolutionWarning {}

impl PartialOrd for ResolutionWarning {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ResolutionWarning {
    fn cmp(&self, other: &Self) -> Ordering {
        self.file
            .cmp(&other.file)
            .then(self.line.cmp(&other.line))
            .then(self.target.cmp(&other.target))
            .then(self.reason.cmp(&other.reason))
    }
}

/// Reason why a file was skipped during processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// File encoding could not be determined
    EncodingError,

    /// File matched ignore pattern
    Ignored,

    /// File is outside workspace scope
    OutOfScope,

    /// File is a symlink that would cause a cycle
    SymlinkCycle,

    /// Other reason with description
    Other(String),
}

// =============================================================================
// Phase 14c: Index Types (Spec Section 14.4 Steps 3-4)
// =============================================================================

/// Entry in the function index.
///
/// Stores metadata about a function definition for cross-file resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncEntry {
    /// Path to the file containing this function (relative to project root).
    pub file_path: PathBuf,

    /// Line number where the function is defined (1-indexed).
    pub line: u32,

    /// End line of the function (1-indexed).
    pub end_line: u32,

    /// Whether this function is a method of a class.
    pub is_method: bool,

    /// Containing class name if `is_method` is true.
    pub class_name: Option<String>,
}

impl FuncEntry {
    /// Creates a new FuncEntry for a standalone function.
    pub fn function(file_path: PathBuf, line: u32, end_line: u32) -> Self {
        Self {
            file_path,
            line,
            end_line,
            is_method: false,
            class_name: None,
        }
    }

    /// Creates a new FuncEntry for a method.
    pub fn method(file_path: PathBuf, line: u32, end_line: u32, class_name: String) -> Self {
        Self {
            file_path,
            line,
            end_line,
            is_method: true,
            class_name: Some(class_name),
        }
    }
}

/// Entry in the class index.
///
/// Stores metadata about a class definition for cross-file resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassEntry {
    /// Path to the file containing this class (relative to project root).
    pub file_path: PathBuf,

    /// Line number where the class is defined (1-indexed).
    pub line: u32,

    /// End line of the class (1-indexed).
    pub end_line: u32,

    /// Method names defined in this class.
    pub methods: Vec<String>,

    /// Base class names (for inheritance tracking).
    pub bases: Vec<String>,
}

impl ClassEntry {
    /// Creates a new ClassEntry.
    pub fn new(
        file_path: PathBuf,
        line: u32,
        end_line: u32,
        methods: Vec<String>,
        bases: Vec<String>,
    ) -> Self {
        Self {
            file_path,
            line,
            end_line,
            methods,
            bases,
        }
    }
}

/// Function index for O(1) lookup of function definitions.
///
/// Maps (module_path, func_name) to FuncEntry for quick cross-file resolution.
///
/// # Thread Safety
///
/// This type is not `Sync` - it's built in parallel but only accessed
/// after the build is complete.
///
/// # Example
/// ```rust,ignore
/// let index = FuncIndex::new();
/// index.insert("mymodule", "process", entry);
/// if let Some(entry) = index.get("mymodule", "process") {
///     println!("Found {} at line {}", entry.file_path.display(), entry.line);
/// }
/// ```
#[derive(Debug, Default)]
pub struct FuncIndex {
    /// Maps (module_path, func_name) -> FuncEntry
    entries: HashMap<(String, String), FuncEntry>,
}

impl FuncIndex {
    /// Creates a new empty function index.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Creates a function index with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
        }
    }

    /// Inserts a function entry.
    pub fn insert(
        &mut self,
        module: impl Into<String>,
        func_name: impl Into<String>,
        entry: FuncEntry,
    ) {
        self.entries
            .insert((module.into(), func_name.into()), entry);
    }

    /// Looks up a function by module and name.
    pub fn get(&self, module: &str, func_name: &str) -> Option<&FuncEntry> {
        self.entries
            .get(&(module.to_string(), func_name.to_string()))
    }

    /// Returns the number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merges another FuncIndex into this one.
    ///
    /// Used to combine results from parallel processing.
    pub fn merge(&mut self, other: FuncIndex) {
        self.entries.extend(other.entries);
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = ((&str, &str), &FuncEntry)> {
        self.entries
            .iter()
            .map(|((m, f), e)| ((m.as_str(), f.as_str()), e))
    }

    /// Finds all entries matching a given function name across all modules.
    /// Used for fallback resolution when the module/receiver cannot be determined.
    pub fn find_by_name<'a>(
        &'a self,
        func_name: &'a str,
    ) -> impl Iterator<Item = &'a FuncEntry> + 'a {
        self.entries
            .iter()
            .filter(move |((_m, f), _)| f.as_str() == func_name)
            .map(|(_, e)| e)
    }

    /// Convert to path map for TypeAwareCallResolver compatibility.
    pub fn to_path_map(&self) -> std::collections::HashMap<(String, String), std::path::PathBuf> {
        self.entries
            .iter()
            .map(|((m, f), e)| ((m.clone(), f.clone()), e.file_path.clone()))
            .collect()
    }
}

/// Class index for O(1) lookup of class definitions.
///
/// Maps class_name to ClassEntry for quick cross-file resolution.
///
/// # Example
/// ```rust,ignore
/// let index = ClassIndex::new();
/// index.insert("User", entry);
/// if let Some(entry) = index.get("User") {
///     println!("Found User at line {}", entry.line);
/// }
/// ```
#[derive(Debug, Default)]
pub struct ClassIndex {
    /// Maps class_name -> ClassEntry
    entries: HashMap<String, ClassEntry>,
}

impl ClassIndex {
    /// Creates a new empty class index.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Creates a class index with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
        }
    }

    /// Inserts a class entry.
    pub fn insert(&mut self, class_name: impl Into<String>, entry: ClassEntry) {
        self.entries.insert(class_name.into(), entry);
    }

    /// Looks up a class by name.
    pub fn get(&self, class_name: &str) -> Option<&ClassEntry> {
        self.entries.get(class_name)
    }

    /// Looks up a mutable class entry by name.
    pub fn get_mut(&mut self, class_name: &str) -> Option<&mut ClassEntry> {
        self.entries.get_mut(class_name)
    }

    /// Returns the number of entries in the index.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merges another ClassIndex into this one.
    ///
    /// Used to combine results from parallel processing.
    pub fn merge(&mut self, other: ClassIndex) {
        self.entries.extend(other.entries);
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ClassEntry)> {
        self.entries.iter().map(|(n, e)| (n.as_str(), e))
    }

    /// Convert to path map for TypeAwareCallResolver compatibility.
    /// Note: ClassIndex uses single-key (class_name), but TypeAwareCallResolver expects (module, class).
    /// We use ("", class_name) as the key since we don't track module per class.
    pub fn to_path_map(&self) -> std::collections::HashMap<(String, String), std::path::PathBuf> {
        self.entries
            .iter()
            .map(|(name, e)| (("".to_string(), name.clone()), e.file_path.clone()))
            .collect()
    }
}

// =============================================================================
// Utility Functions (FM-1 and FM-2 mitigations)
// =============================================================================

/// Capitalizes the first character of a string.
/// Used for Java/Kotlin/C# convention: variable "owner" -> class "Owner".
pub(crate) fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// =============================================================================
// Thread-Local Parsers (Mitigation M1.4)
// =============================================================================

// NOTE: Tree-sitter Parser is !Send, so we cannot share it across threads.
// The get_thread_local_parser function creates a new parser per call,
// which is safe for parallel execution via rayon. Each thread gets its own
// parser instance on the stack.
//
// For future optimization, we could use thread_local! to cache parsers
// per language per thread, but the current approach is correct and simpler.

/// Gets or creates a thread-local parser for the specified language.
///
/// Per Mitigation M1.4: Tree-sitter Parser is !Send, so we use thread_local!
/// to ensure each thread has its own parser instance.
pub(crate) fn get_thread_local_parser(language: &str) -> Result<Parser, BuildError> {
    let mut parser = Parser::new();

    let ts_language = match language.to_lowercase().as_str() {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "typescript" | "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "js" => tree_sitter_typescript::LANGUAGE_TSX.into(), // JS/JSX via TSX grammar
        "go" => tree_sitter_go::LANGUAGE.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "scala" => tree_sitter_scala::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "lua" => tree_sitter_lua::LANGUAGE.into(),
        "luau" => tree_sitter_luau::LANGUAGE.into(),
        "elixir" => tree_sitter_elixir::LANGUAGE.into(),
        "ocaml" => tree_sitter_ocaml::LANGUAGE_OCAML.into(),
        _ => return Err(BuildError::UnsupportedLanguage(language.to_string())),
    };

    parser
        .set_language(&ts_language)
        .map_err(|e| BuildError::ParseError {
            file: PathBuf::new(),
            message: format!("Failed to set language {}: {}", language, e),
        })?;

    Ok(parser)
}

/// Parse source code and return the tree.
///
/// Uses thread-local parser storage per M1.4 mitigation.
pub(crate) fn parse_source(source: &str, language: &str) -> Result<Tree, BuildError> {
    let mut parser = get_thread_local_parser(language)?;

    parser
        .parse(source, None)
        .ok_or_else(|| BuildError::ParseError {
            file: PathBuf::new(),
            message: "Parser returned None".to_string(),
        })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_build_config_defaults() {
        let config = BuildConfig::default();

        assert!(config.language.is_empty());
        assert!(!config.use_workspace_config);
        assert!(config.workspace_roots.is_empty());
        assert!(!config.use_type_resolution);
        assert!(config.respect_ignore);
        assert_eq!(config.parallelism, 0);
        assert!(!config.verbose);
    }

    #[test]
    fn test_build_error_display() {
        let err = BuildError::RootNotFound(PathBuf::from("/foo/bar"));
        assert!(err.to_string().contains("/foo/bar"));

        let err = BuildError::UnsupportedLanguage("brainfuck".to_string());
        assert!(err.to_string().contains("brainfuck"));
    }

    #[test]
    fn test_build_diagnostics_sort() {
        let mut diag = BuildDiagnostics::new();

        diag.parse_errors.push(ParseDiagnostic {
            file: PathBuf::from("z.py"),
            line: 10,
            message: "error".to_string(),
        });
        diag.parse_errors.push(ParseDiagnostic {
            file: PathBuf::from("a.py"),
            line: 5,
            message: "error".to_string(),
        });

        diag.sort();

        assert_eq!(diag.parse_errors[0].file, PathBuf::from("a.py"));
        assert_eq!(diag.parse_errors[1].file, PathBuf::from("z.py"));
    }

    /// Test: BuildDiagnostics methods
    #[test]
    fn test_build_diagnostics_methods() {
        let mut diag = BuildDiagnostics::new();

        assert!(!diag.has_errors());
        assert_eq!(diag.count(), 0);

        diag.parse_errors.push(ParseDiagnostic {
            file: PathBuf::from("test.py"),
            line: 1,
            message: "test error".to_string(),
        });

        assert!(diag.has_errors());
        assert_eq!(diag.count(), 1);

        diag.resolution_warnings.push(ResolutionWarning {
            file: PathBuf::from("test.py"),
            line: 2,
            target: "some_import".to_string(),
            reason: "not found".to_string(),
        });

        assert_eq!(diag.count(), 2);
    }

    /// Test: SkipReason variants exist
    #[test]
    fn test_skip_reason_variants() {
        let reasons = vec![
            SkipReason::EncodingError,
            SkipReason::Ignored,
            SkipReason::OutOfScope,
            SkipReason::SymlinkCycle,
            SkipReason::Other("custom reason".to_string()),
        ];

        for reason in reasons {
            // Just verify they can be created and compared
            assert_eq!(reason.clone(), reason);
        }
    }
}
