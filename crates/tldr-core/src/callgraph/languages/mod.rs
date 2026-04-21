//! Language Handler Trait and Registry for call graph analysis.
//!
//! This module provides the `CallGraphLanguageSupport` trait that all language handlers
//! implement, plus a registry for managing and dispatching to handlers by language name
//! or file extension.
//!
//! # Architecture
//!
//! The language handler system uses traits for internal extensibility while providing
//! functional dispatch in the public API (premortem mitigation #6.1):
//!
//! ```text
//! Public API (functional)             Internal (trait-based)
//! ----------------------              ----------------------
//! extract_calls_for_language()   -->  LanguageRegistry.get(lang)
//!                                         --> dyn CallGraphLanguageSupport
//!                                             --> handler.extract_calls()
//! ```
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 8 for the full specification.

pub mod base;
pub mod c;
pub mod c_common;
pub mod common;
pub mod cpp;
pub mod csharp;
pub mod go;
pub mod java;
pub mod kotlin;
pub mod php;
pub mod python;
pub mod ruby;
pub mod rust_lang;
pub mod scala;
pub mod swift;
pub mod typescript;

// Tier 4 languages (Phase 12)
pub mod elixir;
pub mod lua;
pub mod luau;
pub mod ocaml;

// Re-export handlers for convenience
pub use c::CHandler;
pub use cpp::CppHandler;
pub use csharp::CsharpHandler;
pub use go::GoHandler;
pub use java::JavaHandler;
pub use kotlin::KotlinHandler;
pub use php::PhpHandler;
pub use python::PythonHandler;
pub use ruby::RubyHandler;
pub use rust_lang::RustLangHandler;
pub use scala::ScalaHandler;
pub use swift::SwiftHandler;
pub use typescript::TypeScriptHandler;

// Tier 4 re-exports
pub use elixir::ElixirHandler;
pub use lua::LuaHandler;
pub use luau::LuauHandler;
pub use ocaml::OcamlHandler;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tree_sitter::Tree;

use super::builder_v2::{
    build_project_call_graph_v2, BuildConfig as V2BuildConfig, BuildError as V2BuildError,
};
use super::cross_file_types::{CallSite, ClassDef, FuncDef, ImportDef, ProjectCallGraphV2};
use super::module_index::ModuleIndex;

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during language-specific parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ParseError {
    /// Language is not supported
    #[error("Unsupported language: {0}")]
    UnsupportedLanguage(String),

    /// Failed to parse source file
    #[error("Parse failed for {file}: {message}")]
    ParseFailed {
        /// Path to the source file that could not be parsed.
        file: std::path::PathBuf,
        /// Description of the parse failure.
        message: String,
    },

    /// Failed to read source file
    #[error("IO error reading {file}: {message}")]
    IoError {
        /// Path to the source file that caused the I/O error.
        file: std::path::PathBuf,
        /// Description of the I/O error encountered.
        message: String,
    },

    /// Invalid UTF-8 in source (after latin-1 fallback also failed)
    #[error("Encoding error in {file}: could not decode as UTF-8 or Latin-1")]
    EncodingError {
        /// Path to the source file with unsupported encoding.
        file: std::path::PathBuf,
    },
}

/// Errors that can occur during call graph building.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildError {
    /// Parse error during build
    #[error(transparent)]
    Parse(#[from] ParseError),

    /// Index is missing or incomplete
    #[error("Module index not available")]
    MissingIndex,

    /// Other build errors
    #[error("Build failed: {0}")]
    Other(String),
}

// =============================================================================
// Section 8.1: CallGraphLanguageSupport Trait
// =============================================================================

/// Trait for language-specific call graph support.
///
/// Each supported language implements this trait to provide:
/// - Import parsing from source code
/// - Function call extraction
/// - Call graph building
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to support parallel processing
/// in the builder (Phase 14).
///
/// # Example
///
/// ```rust,ignore
/// struct PythonHandler;
///
/// impl CallGraphLanguageSupport for PythonHandler {
///     fn name(&self) -> &str { "python" }
///     fn extensions(&self) -> &[&str] { &[".py", ".pyi"] }
///     // ... other methods
/// }
/// ```
pub trait CallGraphLanguageSupport: Send + Sync {
    /// Language name (e.g., "python", "typescript", "rust").
    ///
    /// This is used for registry lookup and should match the language
    /// identifier used throughout the codebase.
    fn name(&self) -> &str;

    /// File extensions handled by this language (e.g., `[".py", ".pyi"]`).
    ///
    /// Extensions should include the leading dot.
    fn extensions(&self) -> &[&str];

    /// Parse imports from source code.
    ///
    /// # Arguments
    ///
    /// * `source` - The source code content
    /// * `path` - Path to the file (for error messages and relative import resolution)
    ///
    /// # Returns
    ///
    /// A vector of `ImportDef` representing all import statements in the file.
    fn parse_imports(&self, source: &str, path: &Path) -> Result<Vec<ImportDef>, ParseError>;

    /// Extract function calls from a file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the source file
    /// * `source` - The source code content
    /// * `tree` - Pre-parsed tree-sitter AST
    ///
    /// # Returns
    ///
    /// A HashMap where keys are function names (callers) and values are
    /// vectors of `CallSite` objects representing calls made from that function.
    fn extract_calls(
        &self,
        path: &Path,
        source: &str,
        tree: &Tree,
    ) -> Result<HashMap<String, Vec<CallSite>>, ParseError>;

    /// Extract function and class definitions from source code.
    ///
    /// # Arguments
    ///
    /// * `source` - The source code content
    /// * `path` - Path to the source file
    /// * `tree` - Pre-parsed tree-sitter AST
    ///
    /// # Returns
    ///
    /// A tuple of (Vec<FuncDef>, Vec<ClassDef>) for all definitions in the file.
    /// Default implementation returns empty vecs (for languages that haven't implemented this yet).
    fn extract_definitions(
        &self,
        source: &str,
        path: &Path,
        tree: &Tree,
    ) -> Result<(Vec<FuncDef>, Vec<ClassDef>), ParseError> {
        let _ = (source, path, tree);
        Ok((Vec::new(), Vec::new()))
    }

    /// Build call graph for a project.
    ///
    /// This method processes all files in a project and builds the cross-file
    /// call graph by:
    /// 1. Parsing imports for each file
    /// 2. Extracting calls
    /// 3. Resolving cross-file edges using the module index
    ///
    /// # Arguments
    ///
    /// * `root` - Project root directory
    /// * `index` - Module index for resolution
    /// * `graph` - Mutable reference to the call graph being built
    fn build_call_graph(
        &self,
        root: &Path,
        _index: &ModuleIndex,
        graph: &mut ProjectCallGraphV2,
    ) -> Result<(), BuildError> {
        build_call_graph_with_v2(self.name(), root, graph)
    }

    /// Check if this handler supports the given language.
    ///
    /// Default implementation compares against `self.name()`.
    fn supports(&self, language: &str) -> bool {
        self.name().eq_ignore_ascii_case(language)
    }

    /// Check if this handler supports the given file extension.
    ///
    /// Default implementation checks if extension is in `self.extensions()`.
    fn supports_extension(&self, ext: &str) -> bool {
        self.extensions()
            .iter()
            .any(|e| e.eq_ignore_ascii_case(ext))
    }
}

/// Shared build_call_graph implementation that delegates to the V2 builder.
///
/// This keeps language handlers focused on parsing while using the canonical
/// cross-file pipeline for resolution.
pub(crate) fn build_call_graph_with_v2(
    language: &str,
    root: &Path,
    graph: &mut ProjectCallGraphV2,
) -> Result<(), BuildError> {
    let mut config = V2BuildConfig {
        language: language.to_string(),
        ..Default::default()
    };
    config.use_type_resolution = true;

    let ir = build_project_call_graph_v2(root, config).map_err(map_v2_build_error)?;
    for edge in ir.edges {
        graph.add_edge(edge);
    }
    Ok(())
}

fn map_v2_build_error(err: V2BuildError) -> BuildError {
    match err {
        V2BuildError::ParseError { file, message } => {
            BuildError::Parse(ParseError::ParseFailed { file, message })
        }
        other => BuildError::Other(other.to_string()),
    }
}

// =============================================================================
// Section 8.2: Language Registry
// =============================================================================

/// Registry of language handlers.
///
/// Provides centralized management of language handlers with lookup by
/// language name or file extension.
///
/// # Thread Safety
///
/// The registry is designed for concurrent read access. Handlers are
/// stored as `Arc<dyn CallGraphLanguageSupport>` to allow sharing
/// across threads.
///
/// # Example
///
/// ```rust,ignore
/// let registry = LanguageRegistry::with_defaults();
///
/// // Lookup by language name
/// if let Some(handler) = registry.get("python") {
///     let imports = handler.parse_imports(source, path)?;
/// }
///
/// // Lookup by file extension
/// if let Some(handler) = registry.get_by_extension(".py") {
///     // ...
/// }
/// ```
#[derive(Default)]
pub struct LanguageRegistry {
    handlers: HashMap<String, Arc<dyn CallGraphLanguageSupport>>,
    extension_map: HashMap<String, String>, // extension -> language name
}

impl LanguageRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            extension_map: HashMap::new(),
        }
    }

    /// Register a language handler.
    ///
    /// The handler will be indexed by its name and all its extensions.
    pub fn register(&mut self, handler: Arc<dyn CallGraphLanguageSupport>) {
        let name = handler.name().to_lowercase();

        // Register extensions
        for ext in handler.extensions() {
            self.extension_map.insert(ext.to_lowercase(), name.clone());
        }

        // Register handler
        self.handlers.insert(name, handler);
    }

    /// Get handler for a language by name.
    ///
    /// Lookup is case-insensitive.
    pub fn get(&self, language: &str) -> Option<Arc<dyn CallGraphLanguageSupport>> {
        let key = language.to_lowercase();
        if let Some(handler) = self.handlers.get(&key) {
            return Some(Arc::clone(handler));
        }
        // JS aliases to the TypeScript handler to keep parsing consistent.
        if key == "javascript" || key == "js" {
            return self.handlers.get("typescript").cloned();
        }
        None
    }

    /// Get handler for a file extension.
    ///
    /// Extension should include the leading dot (e.g., ".py").
    /// Lookup is case-insensitive.
    pub fn get_by_extension(&self, ext: &str) -> Option<Arc<dyn CallGraphLanguageSupport>> {
        let lang = self.extension_map.get(&ext.to_lowercase())?;
        self.handlers.get(lang).cloned()
    }

    /// List all supported language names.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(|s| s.as_str())
    }

    /// List all supported file extensions.
    pub fn extensions(&self) -> impl Iterator<Item = &str> {
        self.extension_map.keys().map(|s| s.as_str())
    }

    /// Returns the number of registered handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns true if no handlers are registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Create registry with all built-in handlers.
    ///
    /// Registers the Tier 1 language handlers (Phase 9):
    /// - Python (.py, .pyi)
    /// - Go (.go)
    /// - C (.c, .h)
    ///
    /// Tier 2 languages (Phase 10):
    /// - TypeScript/JavaScript (.ts, .tsx, .js, .jsx)
    /// - Rust (.rs)
    /// - Ruby (.rb, .rake)
    /// - Java (.java)
    ///
    /// Tier 3 languages (Phase 11):
    /// - C# (.cs)
    /// - Kotlin (.kt, .kts)
    ///
    /// Future phases will add more languages:
    /// - Phase 12: Scala, PHP, Lua, Elixir
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();

        // Tier 1 languages (Phase 9)
        registry.register(Arc::new(PythonHandler::new()));
        registry.register(Arc::new(GoHandler::new()));
        registry.register(Arc::new(CHandler::new()));
        registry.register(Arc::new(CppHandler::new()));

        // Tier 2 languages (Phase 10)
        registry.register(Arc::new(TypeScriptHandler::new()));
        registry.register(Arc::new(RustLangHandler::new()));
        registry.register(Arc::new(RubyHandler::new()));
        registry.register(Arc::new(JavaHandler::new()));

        // Tier 3 languages (Phase 11)
        registry.register(Arc::new(CsharpHandler::new()));
        registry.register(Arc::new(KotlinHandler::new()));
        registry.register(Arc::new(SwiftHandler::new()));

        // Tier 4 languages (Phase 12)
        registry.register(Arc::new(ScalaHandler::new()));
        registry.register(Arc::new(PhpHandler::new()));
        registry.register(Arc::new(LuaHandler::new()));
        registry.register(Arc::new(LuauHandler::new()));
        registry.register(Arc::new(ElixirHandler::new()));
        registry.register(Arc::new(OcamlHandler::new()));

        registry
    }
}

impl std::fmt::Debug for LanguageRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageRegistry")
            .field("languages", &self.handlers.keys().collect::<Vec<_>>())
            .field("extensions", &self.extension_map.keys().collect::<Vec<_>>())
            .finish()
    }
}

// =============================================================================
// Section 8.3: Public API (Functional Dispatch)
// =============================================================================

/// Public API - functional dispatch for extracting calls.
///
/// This function provides a simple functional interface that routes to the
/// appropriate language handler internally. This follows the premortem
/// mitigation #6.1: "Use trait internally for extensibility, keep functional
/// dispatch in public API".
///
/// # Arguments
///
/// * `language` - Language name (e.g., "python", "typescript")
/// * `path` - Path to the source file
/// * `source` - Source code content
///
/// # Returns
///
/// A HashMap of function name -> CallSites, or an error if the language
/// is not supported or parsing fails.
///
/// # Example
///
/// ```rust,ignore
/// let calls = extract_calls_for_language(
///     "python",
///     Path::new("src/main.py"),
///     source_code,
/// )?;
///
/// for (func_name, call_sites) in &calls {
///     println!("{} makes {} calls", func_name, call_sites.len());
/// }
/// ```
pub fn extract_calls_for_language(
    language: &str,
    path: &Path,
    source: &str,
) -> Result<HashMap<String, Vec<CallSite>>, ParseError> {
    let registry = LanguageRegistry::with_defaults();
    let handler = registry
        .get(language)
        .ok_or_else(|| ParseError::UnsupportedLanguage(language.to_string()))?;

    let tree = parse_file_for_language(language, source)?;
    handler.extract_calls(path, source, &tree)
}

/// Public API - functional dispatch for parsing imports.
///
/// # Arguments
///
/// * `language` - Language name
/// * `source` - Source code content
/// * `path` - Path to the source file
///
/// # Returns
///
/// A vector of ImportDef, or an error if parsing fails.
pub fn parse_imports_for_language(
    language: &str,
    source: &str,
    path: &Path,
) -> Result<Vec<ImportDef>, ParseError> {
    let registry = LanguageRegistry::with_defaults();
    let handler = registry
        .get(language)
        .ok_or_else(|| ParseError::UnsupportedLanguage(language.to_string()))?;

    handler.parse_imports(source, path)
}

/// Parse a source file using the appropriate tree-sitter parser.
///
/// This is a helper function used internally by the public API functions.
fn parse_file_for_language(language: &str, source: &str) -> Result<Tree, ParseError> {
    use tree_sitter::Parser;

    let mut parser = Parser::new();

    // Set the language based on the language name
    let ts_language = match language.to_lowercase().as_str() {
        "python" => tree_sitter_python::LANGUAGE,
        "typescript" | "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        "javascript" | "js" => tree_sitter_typescript::LANGUAGE_TSX,
        "go" => tree_sitter_go::LANGUAGE,
        "rust" => tree_sitter_rust::LANGUAGE,
        "java" => tree_sitter_java::LANGUAGE,
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE,
        "c" => tree_sitter_c::LANGUAGE,
        "cpp" | "c++" => tree_sitter_cpp::LANGUAGE,
        "ruby" => tree_sitter_ruby::LANGUAGE,
        "csharp" | "c#" => tree_sitter_c_sharp::LANGUAGE,
        "scala" => tree_sitter_scala::LANGUAGE,
        "php" => tree_sitter_php::LANGUAGE_PHP,
        "lua" => tree_sitter_lua::LANGUAGE,
        "luau" => tree_sitter_luau::LANGUAGE,
        "elixir" => tree_sitter_elixir::LANGUAGE,
        "swift" => tree_sitter_swift::LANGUAGE,
        _ => {
            return Err(ParseError::UnsupportedLanguage(language.to_string()));
        }
    };

    parser
        .set_language(&ts_language.into())
        .map_err(|e| ParseError::ParseFailed {
            file: std::path::PathBuf::new(),
            message: format!("Failed to set language: {}", e),
        })?;

    parser
        .parse(source, None)
        .ok_or_else(|| ParseError::ParseFailed {
            file: std::path::PathBuf::new(),
            message: "Parser returned None".to_string(),
        })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Trait Tests
    // -------------------------------------------------------------------------

    mod trait_tests {
        use super::*;

        /// Dummy handler for testing the trait
        struct DummyHandler {
            name: &'static str,
            extensions: Vec<&'static str>,
        }

        impl DummyHandler {
            fn new(name: &'static str, extensions: Vec<&'static str>) -> Self {
                Self { name, extensions }
            }
        }

        impl CallGraphLanguageSupport for DummyHandler {
            fn name(&self) -> &str {
                self.name
            }

            fn extensions(&self) -> &[&str] {
                &self.extensions
            }

            fn parse_imports(
                &self,
                _source: &str,
                _path: &Path,
            ) -> Result<Vec<ImportDef>, ParseError> {
                Ok(vec![])
            }

            fn extract_calls(
                &self,
                _path: &Path,
                _source: &str,
                _tree: &Tree,
            ) -> Result<HashMap<String, Vec<CallSite>>, ParseError> {
                Ok(HashMap::new())
            }
        }

        #[test]
        fn test_trait_methods_exist() {
            let handler = DummyHandler::new("test", vec![".test"]);

            // Verify trait methods exist and return expected types
            assert_eq!(handler.name(), "test");
            assert_eq!(handler.extensions(), &[".test"]);
            assert!(handler.supports("test"));
            assert!(handler.supports("TEST")); // case-insensitive
            assert!(!handler.supports("other"));
            assert!(handler.supports_extension(".test"));
            assert!(handler.supports_extension(".TEST")); // case-insensitive
            assert!(!handler.supports_extension(".other"));
        }

        #[test]
        fn test_trait_is_send_sync() {
            // This test verifies the trait bounds compile
            fn assert_send_sync<T: Send + Sync>() {}
            assert_send_sync::<DummyHandler>();
        }
    }

    // -------------------------------------------------------------------------
    // Registry Tests
    // -------------------------------------------------------------------------

    mod registry_tests {
        use super::*;

        struct MockHandler {
            name: String,
            extensions: Vec<&'static str>,
        }

        impl MockHandler {
            fn new(name: &str, extensions: Vec<&'static str>) -> Self {
                Self {
                    name: name.to_string(),
                    extensions,
                }
            }
        }

        impl CallGraphLanguageSupport for MockHandler {
            fn name(&self) -> &str {
                &self.name
            }

            fn extensions(&self) -> &[&str] {
                &self.extensions
            }

            fn parse_imports(
                &self,
                _source: &str,
                _path: &Path,
            ) -> Result<Vec<ImportDef>, ParseError> {
                Ok(vec![])
            }

            fn extract_calls(
                &self,
                _path: &Path,
                _source: &str,
                _tree: &Tree,
            ) -> Result<HashMap<String, Vec<CallSite>>, ParseError> {
                Ok(HashMap::new())
            }
        }

        #[test]
        fn test_registry_new_is_empty() {
            let registry = LanguageRegistry::new();
            assert!(registry.is_empty());
            assert_eq!(registry.len(), 0);
        }

        #[test]
        fn test_registry_register_and_get() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py", ".pyi"])));

            assert_eq!(registry.len(), 1);
            assert!(!registry.is_empty());

            // Get by name
            let handler = registry.get("python");
            assert!(handler.is_some());
            assert_eq!(handler.unwrap().name(), "python");

            // Get non-existent
            assert!(registry.get("nonexistent").is_none());
        }

        #[test]
        fn test_registry_get_case_insensitive() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("Python", vec![".py"])));

            // All these should find the handler
            assert!(registry.get("python").is_some());
            assert!(registry.get("Python").is_some());
            assert!(registry.get("PYTHON").is_some());
        }

        #[test]
        fn test_registry_get_by_extension() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py", ".pyi"])));
            registry.register(Arc::new(MockHandler::new(
                "typescript",
                vec![".ts", ".tsx"],
            )));

            // Get by extension
            let handler = registry.get_by_extension(".py");
            assert!(handler.is_some());
            assert_eq!(handler.unwrap().name(), "python");

            let handler = registry.get_by_extension(".pyi");
            assert!(handler.is_some());
            assert_eq!(handler.unwrap().name(), "python");

            let handler = registry.get_by_extension(".ts");
            assert!(handler.is_some());
            assert_eq!(handler.unwrap().name(), "typescript");

            // Non-existent extension
            assert!(registry.get_by_extension(".xyz").is_none());
        }

        #[test]
        fn test_registry_get_by_extension_case_insensitive() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py"])));

            assert!(registry.get_by_extension(".py").is_some());
            assert!(registry.get_by_extension(".PY").is_some());
            assert!(registry.get_by_extension(".Py").is_some());
        }

        #[test]
        fn test_registry_languages_iterator() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py"])));
            registry.register(Arc::new(MockHandler::new("go", vec![".go"])));

            let languages: Vec<&str> = registry.languages().collect();
            assert_eq!(languages.len(), 2);
            assert!(languages.contains(&"python"));
            assert!(languages.contains(&"go"));
        }

        #[test]
        fn test_registry_extensions_iterator() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py", ".pyi"])));

            let extensions: Vec<&str> = registry.extensions().collect();
            assert_eq!(extensions.len(), 2);
            assert!(extensions.contains(&".py"));
            assert!(extensions.contains(&".pyi"));
        }

        #[test]
        fn test_registry_with_defaults() {
            // Tier 1: Python, Go, C, C++
            // Tier 2: TypeScript, Rust, Ruby, Java
            // Tier 3: C#, Kotlin, Swift
            // Tier 4: Scala, PHP (added more recently)
            let registry = LanguageRegistry::with_defaults();
            assert!(!registry.is_empty());
            assert_eq!(registry.len(), 17); // All registered handlers (13 + 4 Tier 4)

            // Verify Tier 1 handlers are registered
            assert!(registry.get("python").is_some());
            assert!(registry.get("go").is_some());
            assert!(registry.get("c").is_some());
            assert!(registry.get("cpp").is_some());

            // Verify Tier 2 handlers are registered
            assert!(registry.get("typescript").is_some());
            assert!(registry.get("rust").is_some());
            assert!(registry.get("ruby").is_some());
            assert!(registry.get("java").is_some());

            // Verify Tier 3 handlers are registered
            assert!(registry.get("csharp").is_some());
            assert!(registry.get("kotlin").is_some());
            assert!(registry.get("swift").is_some());

            // Verify extension lookups work
            assert!(registry.get_by_extension(".py").is_some());
            assert!(registry.get_by_extension(".pyi").is_some());
            assert!(registry.get_by_extension(".go").is_some());
            assert!(registry.get_by_extension(".c").is_some());
            assert!(registry.get_by_extension(".h").is_some());
            assert!(registry.get_by_extension(".ts").is_some());
            assert!(registry.get_by_extension(".tsx").is_some());
            assert!(registry.get_by_extension(".rs").is_some());
            assert!(registry.get_by_extension(".rb").is_some());
            assert!(registry.get_by_extension(".rake").is_some());
            assert!(registry.get_by_extension(".java").is_some());
            assert!(registry.get_by_extension(".swift").is_some());
        }

        #[test]
        fn test_registry_debug_impl() {
            let mut registry = LanguageRegistry::new();
            registry.register(Arc::new(MockHandler::new("python", vec![".py"])));

            let debug_str = format!("{:?}", registry);
            assert!(debug_str.contains("LanguageRegistry"));
            assert!(debug_str.contains("python"));
        }
    }

    // -------------------------------------------------------------------------
    // Error Tests
    // -------------------------------------------------------------------------

    mod error_tests {
        use super::*;

        #[test]
        fn test_parse_error_unsupported_language() {
            let err = ParseError::UnsupportedLanguage("brainfuck".to_string());
            assert!(err.to_string().contains("Unsupported language"));
            assert!(err.to_string().contains("brainfuck"));
        }

        #[test]
        fn test_parse_error_parse_failed() {
            let err = ParseError::ParseFailed {
                file: std::path::PathBuf::from("test.py"),
                message: "syntax error".to_string(),
            };
            assert!(err.to_string().contains("test.py"));
            assert!(err.to_string().contains("syntax error"));
        }

        #[test]
        fn test_build_error_from_parse_error() {
            let parse_err = ParseError::UnsupportedLanguage("test".to_string());
            let build_err: BuildError = parse_err.into();
            assert!(matches!(build_err, BuildError::Parse(_)));
        }
    }

    // -------------------------------------------------------------------------
    // Parse File Tests
    // -------------------------------------------------------------------------

    mod parse_tests {
        use super::*;

        #[test]
        fn test_parse_file_python() {
            let source = "def hello(): pass";
            let result = parse_file_for_language("python", source);
            assert!(result.is_ok());
        }

        #[test]
        fn test_parse_file_unsupported() {
            let result = parse_file_for_language("brainfuck", "");
            assert!(matches!(result, Err(ParseError::UnsupportedLanguage(_))));
        }

        #[test]
        fn test_parse_file_case_insensitive() {
            let source = "def hello(): pass";
            assert!(parse_file_for_language("Python", source).is_ok());
            assert!(parse_file_for_language("PYTHON", source).is_ok());
        }
    }

    // -------------------------------------------------------------------------
    // Public API Tests
    // -------------------------------------------------------------------------

    mod public_api_tests {
        use super::*;

        #[test]
        fn test_extract_calls_unsupported_language() {
            let result = extract_calls_for_language("brainfuck", Path::new("test.bf"), "");
            assert!(matches!(result, Err(ParseError::UnsupportedLanguage(_))));
        }

        #[test]
        fn test_parse_imports_unsupported_language() {
            let result = parse_imports_for_language("brainfuck", "", Path::new("test.bf"));
            assert!(matches!(result, Err(ParseError::UnsupportedLanguage(_))));
        }
    }
}
