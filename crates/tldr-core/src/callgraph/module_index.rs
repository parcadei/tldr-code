//! Module Index for bidirectional module path <-> file path mapping.
//!
//! This module provides the `ModuleIndex` struct which maintains a mapping between
//! Python module paths (e.g., "myapp.utils") and their corresponding file paths
//! (e.g., "src/myapp/utils.py").
//!
//! # Overview
//!
//! The `ModuleIndex` is designed for:
//! - O(1) lookup of file paths from module names
//! - O(1) reverse lookup of module names from file paths
//! - Proper handling of Python packages (`__init__.py`)
//! - Support for namespace packages (PEP 420)
//! - Symlink resolution to canonical paths
//! - Platform-specific case sensitivity handling
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::callgraph::module_index::ModuleIndex;
//! use std::path::Path;
//!
//! let index = ModuleIndex::build(Path::new("src"), "python")?;
//!
//! // Forward lookup: module -> file
//! assert!(index.lookup("myapp.utils").is_some());
//!
//! // Reverse lookup: file -> module
//! assert_eq!(index.reverse_lookup(Path::new("src/myapp/utils.py")), Some("myapp.utils"));
//!
//! // Check if module is in project
//! assert!(index.is_project_module("myapp.utils"));
//! assert!(!index.is_project_module("os"));  // stdlib, not in project
//! ```
//!
//! # Spec Reference
//!
//! See `migration/spec/callgraph-spec.md` Section 4 for the full behavioral specification.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use lazy_static::lazy_static;
use regex::Regex;
use serde_json::Value as JsonValue;
use thiserror::Error;

/// Errors that can occur during module indexing.
#[derive(Debug, Error)]
pub enum ModuleIndexError {
    /// IO error during directory traversal
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Path is outside project root (security check)
    #[error("Path outside project root: {0}")]
    PathOutsideRoot(PathBuf),

    /// Invalid UTF-8 in path
    #[error("Invalid UTF-8 in path: {0}")]
    InvalidUtf8(PathBuf),
}

/// Module Index for bidirectional module path <-> file path mapping.
///
/// This struct maintains two HashMaps for O(1) lookups in both directions:
/// - `module_to_file`: module path -> file path
/// - `file_to_module`: file path -> module path
///
/// It also tracks namespace packages (directories with Python files but no `__init__.py`).
#[derive(Debug, Default)]
pub struct ModuleIndex {
    /// Project root directory (canonical path)
    project_root: PathBuf,
    /// Module name to file path mapping
    module_to_file: HashMap<String, PathBuf>,
    /// File path to module name mapping
    file_to_module: HashMap<PathBuf, String>,
    /// Set of namespace packages (PEP 420)
    namespace_packages: HashSet<String>,
    /// Language being indexed (affects module naming)
    language: String,
    /// Detected build metadata for improved module resolution
    metadata: ModuleIndexMetadata,
}

/// Build metadata used to refine module naming/aliasing.
#[derive(Debug, Default, Clone)]
struct ModuleIndexMetadata {
    python_src_root: Option<PathBuf>,
    ts_base_url: Option<PathBuf>,
    ts_paths: Vec<TsPathMapping>,
    js_package_name: Option<String>,
    go_module_path: Option<String>,
    rust_crate_name: Option<String>,
    php_psr4: Vec<(String, PathBuf)>,
}

#[derive(Debug, Clone, Default)]
struct TsPathMapping {
    alias_pattern: String,
    target_patterns: Vec<String>,
}

impl ModuleIndex {
    /// Creates a new empty ModuleIndex.
    pub fn new(project_root: PathBuf, language: &str) -> Self {
        Self::new_with_workspace_roots(project_root, language, &[])
    }

    /// Creates a new empty ModuleIndex, optionally using additional workspace
    /// roots for build-metadata detection (VAL-007).
    ///
    /// `extra_workspace_roots` is a list of directories in addition to
    /// `project_root` whose manifest files (`tsconfig.json`, `package.json`,
    /// ...) should contribute to the metadata. The root itself is implied and
    /// must NOT be duplicated here.
    pub fn new_with_workspace_roots(
        project_root: PathBuf,
        language: &str,
        extra_workspace_roots: &[PathBuf],
    ) -> Self {
        let metadata = ModuleIndexMetadata::detect_with_workspace_roots(
            &project_root,
            language,
            extra_workspace_roots,
        );
        Self {
            project_root,
            module_to_file: HashMap::new(),
            file_to_module: HashMap::new(),
            namespace_packages: HashSet::new(),
            language: language.to_lowercase(),
            metadata,
        }
    }

    /// Build index from project root for given language.
    ///
    /// Walks the directory tree, skipping common non-source directories
    /// (`__pycache__`, `.git`, `venv`, `node_modules`, etc.).
    ///
    /// # Arguments
    ///
    /// * `root` - Project root directory to scan
    /// * `language` - Programming language ("python", "typescript", "rust", "go")
    ///
    /// # Returns
    ///
    /// A populated `ModuleIndex` or an error.
    ///
    /// # Errors
    ///
    /// Returns `ModuleIndexError::Io` if the root directory cannot be accessed.
    pub fn build(root: &Path, language: &str) -> Result<Self, ModuleIndexError> {
        Self::build_with_ignore(root, language, true)
    }

    /// Build index with configurable gitignore handling.
    ///
    /// # Arguments
    ///
    /// * `root` - Project root directory to scan
    /// * `language` - Programming language
    /// * `respect_ignore` - Whether to respect `.gitignore` patterns
    pub fn build_with_ignore(
        root: &Path,
        language: &str,
        respect_ignore: bool,
    ) -> Result<Self, ModuleIndexError> {
        Self::build_with_workspace_roots(root, language, respect_ignore, &[])
    }

    /// Build index, optionally reading build metadata (tsconfig paths, etc.)
    /// from additional workspace roots in addition to the primary `root`
    /// (VAL-007).
    ///
    /// When called with an empty `extra_workspace_roots` slice, behaves
    /// identically to [`Self::build_with_ignore`].
    ///
    /// # Arguments
    ///
    /// * `root` - Project root directory to scan
    /// * `language` - Programming language
    /// * `respect_ignore` - Whether to respect `.gitignore` patterns
    /// * `extra_workspace_roots` - Additional directories (e.g. pnpm package
    ///   dirs) whose `tsconfig.json` / `package.json` should also be read
    pub fn build_with_workspace_roots(
        root: &Path,
        language: &str,
        respect_ignore: bool,
        extra_workspace_roots: &[PathBuf],
    ) -> Result<Self, ModuleIndexError> {
        // Resolve to canonical path for symlink handling
        let canonical_root = resolve_path(root, root)?;

        let mut index =
            Self::new_with_workspace_roots(canonical_root.clone(), language, extra_workspace_roots);

        // Track directories that contain Python files (for namespace package detection)
        let mut dirs_with_py_files: HashSet<PathBuf> = HashSet::new();
        // Track directories with __init__.py (regular packages)
        let mut dirs_with_init: HashSet<PathBuf> = HashSet::new();

        // Get language-specific file extension
        let extensions = get_language_extensions(language);

        // Build the walker with common exclusions
        let walker = WalkBuilder::new(&canonical_root)
            .hidden(true) // Skip hidden files/dirs
            .git_ignore(respect_ignore)
            .git_global(respect_ignore)
            .git_exclude(respect_ignore)
            .filter_entry(|entry| {
                // Skip common non-source directories
                let file_name = entry.file_name().to_string_lossy();
                !should_skip_directory(&file_name)
            })
            .build();

        // First pass: collect all relevant files
        let mut files: Vec<PathBuf> = Vec::new();

        for entry in walker.flatten() {
            let path = entry.path();

            // Skip if not a file
            if !path.is_file() {
                continue;
            }

            // Check extension
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase());

            let is_relevant = ext
                .as_ref()
                .map(|e| extensions.contains(&e.as_str()))
                .unwrap_or(false);

            if !is_relevant {
                continue;
            }

            // Resolve symlinks
            let canonical = match resolve_path(path, &canonical_root) {
                Ok(p) => p,
                Err(_) => continue, // Skip invalid paths
            };

            // Track directory for namespace package detection (Python)
            if language == "python" {
                if let Some(parent) = canonical.parent() {
                    dirs_with_py_files.insert(parent.to_path_buf());

                    // Check if this is an __init__.py
                    let file_name = canonical.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if file_name == "__init__.py" {
                        dirs_with_init.insert(parent.to_path_buf());
                    }
                }
            }

            files.push(canonical);
        }

        // Detect namespace packages (dirs with .py files but no __init__.py)
        if language == "python" {
            for dir in &dirs_with_py_files {
                if !dirs_with_init.contains(dir) {
                    let module = index.path_to_module(dir);
                    if !module.is_empty() {
                        index.namespace_packages.insert(module);
                    }
                }
            }
        }

        // Second pass: index files
        // Process packages (__init__.py) first, then modules
        // This ensures package wins over module when both exist
        let mut init_files: Vec<PathBuf> = Vec::new();
        let mut other_files: Vec<PathBuf> = Vec::new();

        for path in files {
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if language == "python" && file_name == "__init__.py" {
                init_files.push(path);
            } else {
                other_files.push(path);
            }
        }

        // Index packages first
        for path in init_files {
            index.index_file(&path)?;
        }

        // Then index modules (skipping if package already exists)
        for path in other_files {
            let module = index.compute_module_name(&path);

            // Check for package/module conflict
            // If a package with same name exists, skip the standalone module
            if language == "python" && !module.is_empty() {
                // For pkg.py when pkg/__init__.py exists, skip pkg.py
                if index.module_to_file.contains_key(&module) {
                    continue;
                }
            }

            index.index_file(&path)?;
        }

        Ok(index)
    }

    /// Index a single file.
    fn index_file(&mut self, path: &Path) -> Result<(), ModuleIndexError> {
        let module = self.compute_module_name(path);

        if module.is_empty() {
            return Ok(());
        }

        let normalized_module = normalize_module_key(&module);

        // Store mapping in both directions
        self.module_to_file
            .insert(normalized_module.clone(), path.to_path_buf());
        self.file_to_module
            .insert(path.to_path_buf(), normalized_module.clone());

        // Index alias keys for language-specific resolution (CROSSFILE parity)
        let mut aliases = self.compute_module_aliases(&module, path);
        if let Some(declared) = self.declared_package_alias(path) {
            aliases.push(declared);
        }

        for alias in aliases {
            let normalized_alias = normalize_module_key(&alias);
            self.module_to_file
                .entry(normalized_alias)
                .or_insert_with(|| path.to_path_buf());
        }

        Ok(())
    }

    /// Compute module name from file path.
    fn compute_module_name(&self, path: &Path) -> String {
        match self.language.as_str() {
            "python" => self.compute_python_module_name(path),
            "typescript" | "javascript" => self.compute_typescript_module_name(path),
            "rust" => self.compute_rust_module_name(path),
            "go" => self.compute_go_module_name(path),
            "java" => self.compute_java_module_name(path),
            "kotlin" => self.compute_kotlin_module_name(path),
            "scala" => self.compute_scala_module_name(path),
            "csharp" | "c#" => self.compute_csharp_module_name(path),
            "php" => self.compute_php_module_name(path),
            "ruby" => self.compute_ruby_module_name(path),
            "lua" => self.compute_lua_module_name(path),
            "luau" => self.compute_lua_module_name(path),
            "elixir" => self.compute_elixir_module_name(path),
            "swift" => self.compute_swift_module_name(path),
            "c" => self.compute_c_module_name(path),
            "cpp" | "c++" => self.compute_cpp_module_name(path),
            "ocaml" => self.compute_ocaml_module_name(path),
            _ => self.compute_python_module_name(path), // Fallback
        }
    }

    /// Compute alias module keys for language-specific resolution.
    fn compute_module_aliases(&self, module: &str, path: &Path) -> Vec<String> {
        let mut aliases = Vec::new();

        // Generic "simple name" alias (last segment)
        let simple = simple_module_name(module);
        if simple != module {
            aliases.push(simple.to_string());
        }

        match self.language.as_str() {
            "typescript" | "javascript" => {
                let module_no_dot = module.strip_prefix("./").unwrap_or(module);
                aliases.push(module_no_dot.to_string());

                let mut stripped_by_base_url: Option<String> = None;
                if let Some(base_url) = &self.metadata.ts_base_url {
                    if let Ok(rel) = base_url.strip_prefix(&self.project_root) {
                        let base = normalize_relative_str(rel);
                        let normalized = module.replace('\\', "/");
                        let candidates = [
                            format!("./{}/", base),
                            format!("{}/", base),
                            format!("./{}", base),
                            base.clone(),
                        ];
                        for prefix in candidates {
                            if normalized.starts_with(&prefix) {
                                let stripped = normalized[prefix.len()..].trim_start_matches('/');
                                if !stripped.is_empty() {
                                    aliases.push(stripped.to_string());
                                    stripped_by_base_url = Some(stripped.to_string());
                                }
                            }
                        }
                    }
                }
                if let Some(pkg) = &self.metadata.js_package_name {
                    let base = stripped_by_base_url
                        .as_deref()
                        .unwrap_or(module_no_dot)
                        .trim_start_matches('/');
                    if base.is_empty() {
                        aliases.push(pkg.to_string());
                    } else {
                        aliases.push(format!("{}/{}", pkg, base));
                    }
                }
                if is_ts_index_file(path) {
                    aliases.push(format!("{}/index", module));
                    if module_no_dot != module {
                        aliases.push(format!("{}/index", module_no_dot));
                    }
                }
                aliases.extend(ts_path_aliases_for_file(
                    path,
                    &self.project_root,
                    self.metadata.ts_base_url.as_ref(),
                    &self.metadata.ts_paths,
                ));
            }
            "rust" => {
                if let Some(stripped) = module.strip_prefix("crate::") {
                    aliases.push(stripped.to_string());
                }
                if let Some(crate_name) = &self.metadata.rust_crate_name {
                    if module == "crate" {
                        aliases.push(crate_name.to_string());
                    } else if let Some(stripped) = module.strip_prefix("crate::") {
                        aliases.push(format!("{}::{}", crate_name, stripped));
                    }
                }
            }
            "go" => {
                if let Some(prefix) = &self.metadata.go_module_path {
                    let base = module.trim_start_matches("./").trim_start_matches('/');
                    if base.is_empty() {
                        aliases.push(prefix.to_string());
                    } else {
                        aliases.push(format!("{}/{}", prefix.trim_end_matches('/'), base));
                    }
                }
            }
            "php" => {
                if module.contains('\\') {
                    aliases.push(module.replace('\\', "/"));
                }
                if module.contains('/') {
                    aliases.push(module.replace('/', "\\"));
                }
                if !module.starts_with('\\') {
                    aliases.push(format!("\\{}", module));
                }
                if !self.metadata.php_psr4.is_empty() {
                    for (prefix, dir) in &self.metadata.php_psr4 {
                        if let Ok(rel) = path.strip_prefix(dir) {
                            let rel_str = normalize_relative_str(rel);
                            let rel_str = strip_extension_any(&rel_str, &[".php"]);
                            if rel_str.is_empty() {
                                continue;
                            }
                            let ns_suffix = rel_str.replace('/', "\\");
                            let mut ns_prefix = prefix.clone();
                            if !ns_prefix.ends_with('\\') {
                                ns_prefix.push('\\');
                            }
                            aliases.push(format!("{}{}", ns_prefix, ns_suffix));
                        }
                    }
                }
            }
            "ruby" => {
                if let Some(camel) = ruby_module_alias_from_path(path, &self.project_root) {
                    aliases.push(camel);
                }
            }
            "lua" | "luau" => {
                if module.contains('.') {
                    aliases.push(module.replace('.', "/"));
                }
                if module.contains('/') {
                    aliases.push(module.replace('/', "."));
                }
                // VAL-011: Luau commonly uses Rojo-style relative requires
                // such as `require('./util')`. Index `./<module>` so the
                // import-resolver can find the file regardless of which
                // form the source uses.
                let leading = format!("./{}", module);
                aliases.push(leading);
                if module.contains('.') {
                    aliases.push(format!("./{}", module.replace('.', "/")));
                }
                if module.contains('/') {
                    aliases.push(format!("./{}", module.replace('/', ".")));
                }
            }
            "c" | "cpp" => {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    aliases.push(file_name.to_string());
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    aliases.push(stem.to_string());
                }
                if let Ok(rel) = path.strip_prefix(&self.project_root) {
                    let rel_str = normalize_relative_str(rel);
                    aliases.push(rel_str.clone());
                    let rel_no_ext = strip_extension_any(
                        &rel_str,
                        &[".c", ".h", ".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"],
                    );
                    if rel_no_ext != rel_str {
                        aliases.push(rel_no_ext.to_string());
                    }
                }
            }
            "swift" => {
                if let Some(root) = swift_module_root(path, &self.project_root) {
                    aliases.push(root);
                }
            }
            "elixir" => {
                if !module.starts_with("Elixir.") {
                    aliases.push(format!("Elixir.{}", module));
                }
            }
            "ocaml" => {
                // VAL-011: OCaml derives module name from file basename with
                // first letter capitalized. `util.ml` → module `Util`. Add
                // capitalized aliases so cross-file calls `Util.b_util()`
                // resolve through `module_imports["Util"] = "Util"` →
                // `func_index["Util"]["b_util"]`.
                let capitalized = module
                    .split('.')
                    .map(capitalize_first_ocaml_segment)
                    .collect::<Vec<_>>()
                    .join(".");
                if capitalized != module {
                    aliases.push(capitalized);
                }
            }
            _ => {}
        }

        aliases
    }

    fn declared_package_alias(&self, path: &Path) -> Option<String> {
        match self.language.as_str() {
            "java" | "kotlin" | "scala" => parse_java_like_package(path),
            "csharp" | "c#" => parse_csharp_namespace(path),
            _ => None,
        }
    }

    /// Compute Python module name from file path.
    ///
    /// Examples:
    /// - `src/myapp/__init__.py` -> `myapp`
    /// - `src/myapp/utils.py` -> `myapp.utils`
    /// - `src/myapp/subpkg/__init__.py` -> `myapp.subpkg`
    fn compute_python_module_name(&self, path: &Path) -> String {
        let relative = if let Some(src_root) = &self.metadata.python_src_root {
            if let Ok(r) = path.strip_prefix(src_root) {
                r
            } else {
                match path.strip_prefix(&self.project_root) {
                    Ok(r) => r,
                    Err(_) => return String::new(),
                }
            }
        } else {
            match path.strip_prefix(&self.project_root) {
                Ok(r) => r,
                Err(_) => return String::new(),
            }
        };

        let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Handle __init__.py -> package name (parent directory)
        if file_name == "__init__.py" {
            let parent = relative.parent().unwrap_or(Path::new(""));
            let parts: Vec<&str> = parent
                .iter()
                .filter_map(|s| s.to_str())
                .filter(|s| !s.is_empty())
                .collect();
            return parts.join(".");
        }

        // Regular module: strip extension
        let stem = relative.with_extension("");
        let parts: Vec<&str> = stem
            .iter()
            .filter_map(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .collect();

        parts.join(".")
    }

    /// Compute TypeScript/JavaScript module name from file path.
    ///
    /// Examples:
    /// - `src/utils/index.ts` -> `./utils`
    /// - `src/helpers.ts` -> `./helpers`
    fn compute_typescript_module_name(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };

        let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Handle index.ts/index.tsx -> parent directory
        if file_name == "index.ts" || file_name == "index.tsx" || file_name == "index.js" {
            let parent = relative.parent().unwrap_or(Path::new(""));
            return format!("./{}", parent.display());
        }

        // Regular module: strip extension
        let stem = relative.with_extension("");
        format!("./{}", stem.display())
    }

    /// Compute Rust module name from file path.
    ///
    /// Examples:
    /// - `src/lib.rs` -> `crate`
    /// - `src/utils/mod.rs` -> `crate::utils`
    /// - `src/utils/helpers.rs` -> `crate::utils::helpers`
    fn compute_rust_module_name(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };

        let file_name = relative.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Handle lib.rs/main.rs -> crate root
        if file_name == "lib.rs" || file_name == "main.rs" {
            return "crate".to_string();
        }

        // Handle mod.rs -> parent module
        if file_name == "mod.rs" {
            let parent = relative.parent().unwrap_or(Path::new(""));
            // Skip 'src' prefix if present
            let parts: Vec<&str> = parent
                .iter()
                .filter_map(|s| s.to_str())
                .filter(|s| *s != "src" && !s.is_empty())
                .collect();

            if parts.is_empty() {
                return "crate".to_string();
            }
            return format!("crate::{}", parts.join("::"));
        }

        // Regular module
        let stem = relative.with_extension("");
        let parts: Vec<&str> = stem
            .iter()
            .filter_map(|s| s.to_str())
            .filter(|s| *s != "src" && !s.is_empty())
            .collect();

        if parts.is_empty() {
            return "crate".to_string();
        }
        format!("crate::{}", parts.join("::"))
    }

    /// Compute Go module name from file path.
    ///
    /// In Go, the directory path is the package path.
    fn compute_go_module_name(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };

        // Go uses directory as package
        relative
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    /// Compute Java module name from file path (dot-separated package path).
    fn compute_java_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &JAVA_PREFIXES, &[".java"])
    }

    /// Compute Kotlin module name from file path (dot-separated package path).
    fn compute_kotlin_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &KOTLIN_PREFIXES, &[".kt", ".kts"])
    }

    /// Compute Scala module name from file path (dot-separated package path).
    fn compute_scala_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &SCALA_PREFIXES, &[".scala"])
    }

    /// Compute C# namespace module name from file path (dot-separated).
    fn compute_csharp_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &CSHARP_PREFIXES, &[".cs"])
    }

    /// Compute PHP module name from file path (backslash-separated namespace).
    fn compute_php_module_name(&self, path: &Path) -> String {
        compute_separator_module_name(path, &self.project_root, &PHP_PREFIXES, &[".php"], '\\')
    }

    /// Compute Ruby module name from file path (slash-separated require path).
    fn compute_ruby_module_name(&self, path: &Path) -> String {
        compute_separator_module_name(path, &self.project_root, &RUBY_PREFIXES, &[".rb"], '/')
    }

    /// Compute Lua/Luau module name from file path (dot-separated).
    fn compute_lua_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &LUA_PREFIXES, &[".lua", ".luau"])
    }

    /// Compute Elixir module name from file path (CamelCase segments).
    fn compute_elixir_module_name(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };
        let mut rel_str = normalize_relative_str(relative);

        let mut module_parts: Vec<String> = Vec::new();

        // Umbrella apps: apps/<app>/lib/<rest>
        if let Some(rest) = rel_str.strip_prefix("apps/") {
            let mut parts = rest.splitn(2, '/');
            if let Some(app) = parts.next() {
                if let Some(after_app) = parts.next() {
                    if let Some(after_lib) = after_app.strip_prefix("lib/") {
                        module_parts.push(snake_to_camel(app));
                        rel_str = after_lib.to_string();
                    }
                }
            }
        }

        // Standard: lib/<rest>
        if module_parts.is_empty() {
            if let Some(after_lib) = rel_str.strip_prefix("lib/") {
                rel_str = after_lib.to_string();
            }
        }

        let rel_str = strip_extension_any(&rel_str, &[".ex", ".exs"]);
        for segment in rel_str.split('/') {
            if segment.is_empty() {
                continue;
            }
            module_parts.push(snake_to_camel(segment));
        }

        module_parts.join(".")
    }

    /// Compute Swift module name from file path (SwiftPM-aware).
    fn compute_swift_module_name(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };
        let rel_str = normalize_relative_str(relative);

        if let Some(rest) = rel_str.strip_prefix("Sources/") {
            return swift_module_from_sources(rest);
        }
        if let Some(rest) = rel_str.strip_prefix("Tests/") {
            return swift_module_from_sources(rest);
        }

        // Fallback to dot path
        compute_dot_module_name(path, &self.project_root, &SWIFT_PREFIXES, &[".swift"])
    }

    /// Compute C module name from file path (relative path with extension).
    fn compute_c_module_name(&self, path: &Path) -> String {
        compute_path_module_name(path, &self.project_root)
    }

    /// Compute C++ module name from file path (relative path with extension).
    fn compute_cpp_module_name(&self, path: &Path) -> String {
        compute_path_module_name(path, &self.project_root)
    }

    /// Compute OCaml module name from file path (dot-separated).
    fn compute_ocaml_module_name(&self, path: &Path) -> String {
        compute_dot_module_name(path, &self.project_root, &OCAML_PREFIXES, &[".ml", ".mli"])
    }

    /// Convert a file path to module name (for reverse lookup building).
    fn path_to_module(&self, path: &Path) -> String {
        let relative = match path.strip_prefix(&self.project_root) {
            Ok(r) => r,
            Err(_) => return String::new(),
        };

        let parts: Vec<&str> = relative
            .iter()
            .filter_map(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .collect();

        parts.join(".")
    }

    /// Look up file path for a module name.
    ///
    /// # Arguments
    ///
    /// * `module` - Dotted module path (e.g., "myapp.utils")
    ///
    /// # Returns
    ///
    /// The file path if found, or `None` if the module is not in the index.
    ///
    /// # Complexity
    ///
    /// O(1) hash lookup.
    pub fn lookup(&self, module: &str) -> Option<&Path> {
        let normalized = normalize_module_key(module);
        self.module_to_file.get(&normalized).map(|p| p.as_path())
    }

    /// Reverse lookup: file path to module name.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to source file
    ///
    /// # Returns
    ///
    /// The module name if found, or `None` if the file is not in the index.
    ///
    /// # Complexity
    ///
    /// O(1) hash lookup. Symlinks are resolved before lookup.
    pub fn reverse_lookup(&self, path: &Path) -> Option<&str> {
        // Resolve symlinks before lookup
        let canonical = match resolve_path(path, &self.project_root) {
            Ok(p) => p,
            Err(_) => path.to_path_buf(),
        };

        self.file_to_module.get(&canonical).map(|s| s.as_str())
    }

    /// Check if module is part of this project (vs external/stdlib).
    ///
    /// # Arguments
    ///
    /// * `module` - Dotted module path
    ///
    /// # Returns
    ///
    /// `true` if the module is indexed in this project, `false` otherwise.
    pub fn is_project_module(&self, module: &str) -> bool {
        let normalized = normalize_module_key(module);

        // Direct lookup
        if self.module_to_file.contains_key(&normalized) {
            return true;
        }

        // Check if parent is a namespace package
        if let Some(dot_pos) = normalized.rfind('.') {
            let parent = &normalized[..dot_pos];
            if self.namespace_packages.contains(parent) {
                return true;
            }
        }

        // Check if this is a namespace package itself
        self.namespace_packages.contains(&normalized)
    }

    /// Check if a module is a namespace package.
    ///
    /// Namespace packages (PEP 420) are directories containing Python files
    /// but no `__init__.py`.
    pub fn is_namespace_package(&self, module: &str) -> bool {
        let normalized = normalize_module_key(module);
        self.namespace_packages.contains(&normalized)
    }

    /// Get all module names in the index.
    pub fn modules(&self) -> impl Iterator<Item = &str> {
        self.module_to_file.keys().map(|s| s.as_str())
    }

    /// Get the number of indexed modules.
    pub fn len(&self) -> usize {
        self.module_to_file.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.module_to_file.is_empty()
    }

    /// Get project root path.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Get the language this index was built for.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Iterate over all (module, path) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Path)> {
        self.module_to_file
            .iter()
            .map(|(m, p)| (m.as_str(), p.as_path()))
    }
}

/// Resolve a path to its canonical form, ensuring it stays within the project root.
///
/// # Security
///
/// This function ensures the resolved path is within the project root,
/// preventing directory traversal attacks via symlinks.
fn resolve_path(path: &Path, root: &Path) -> Result<PathBuf, ModuleIndexError> {
    // Use dunce to get canonical path without UNC prefix on Windows
    let canonical = dunce::canonicalize(path).map_err(ModuleIndexError::Io)?;

    // Security check: ensure path is under root
    // Skip this check if we're resolving the root itself
    if path != root {
        let canonical_root = dunce::canonicalize(root).map_err(ModuleIndexError::Io)?;
        if !canonical.starts_with(&canonical_root) {
            return Err(ModuleIndexError::PathOutsideRoot(canonical));
        }
    }

    Ok(canonical)
}

/// Normalize module key for case sensitivity.
///
/// On macOS and Windows, module lookups should be case-insensitive
/// to match the filesystem behavior.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn normalize_module_key(key: &str) -> String {
    key.to_lowercase()
}

/// Normalize module key for case sensitivity.
///
/// On Linux and other Unix systems, module lookups are case-sensitive.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn normalize_module_key(key: &str) -> String {
    key.to_string()
}

impl ModuleIndexMetadata {
    #[allow(dead_code)] // public API retained for callers that don't need multi-root
    fn detect(root: &Path, language: &str) -> Self {
        Self::detect_with_workspace_roots(root, language, &[])
    }

    /// Detect build metadata, optionally also consulting additional workspace
    /// roots' manifests (VAL-007).
    ///
    /// For TypeScript/JavaScript, this reads `tsconfig.json` from the primary
    /// `root` PLUS each entry in `extra_roots` and merges their path aliases.
    /// Duplicate alias patterns resolve with last-wins semantics (documented
    /// limitation; per-root scoped resolution is future work).
    fn detect_with_workspace_roots(root: &Path, language: &str, extra_roots: &[PathBuf]) -> Self {
        let lang = language.to_lowercase();
        let mut meta = ModuleIndexMetadata::default();

        if lang == "python" {
            meta.python_src_root = detect_python_src_root(root);
        }

        if lang == "typescript" || lang == "javascript" {
            meta.ts_base_url = detect_ts_base_url_multi(root, extra_roots);
            meta.ts_paths = detect_ts_paths_multi(root, extra_roots);
            meta.js_package_name = detect_js_package_name(root);
        }

        if lang == "go" {
            meta.go_module_path = detect_go_module_path(root);
        }

        if lang == "rust" {
            meta.rust_crate_name = detect_rust_crate_name(root);
        }

        if lang == "php" {
            meta.php_psr4 = detect_php_psr4(root);
        }

        meta
    }
}

// =============================================================================
// Build metadata detection helpers
// =============================================================================

fn detect_python_src_root(root: &Path) -> Option<PathBuf> {
    let candidate = root.join("src");
    if !candidate.is_dir() {
        return None;
    }
    // Look for at least one .py file under src/ to confirm layout.
    let walker = WalkBuilder::new(&candidate)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .max_depth(Some(6))
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("py") {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn detect_go_module_path(root: &Path) -> Option<String> {
    let path = root.join("go.mod");
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("module ") {
            let module = trimmed.trim_start_matches("module ").trim();
            if !module.is_empty() {
                return Some(module.to_string());
            }
        }
    }
    None
}

fn detect_rust_crate_name(root: &Path) -> Option<String> {
    let path = root.join("Cargo.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if trimmed.starts_with("name") {
            if let Some((_, value)) = trimmed.split_once('=') {
                let name = value.trim().trim_matches('"').trim_matches('\'');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn detect_js_package_name(root: &Path) -> Option<String> {
    let json = read_json_file(root.join("package.json"))?;
    json.get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[allow(dead_code)] // retained for callers not using multi-root discovery
fn detect_ts_base_url(root: &Path) -> Option<PathBuf> {
    detect_ts_base_url_multi(root, &[])
}

/// Detect the `baseUrl` at the primary `root`'s `tsconfig.json` (VAL-007).
///
/// We intentionally do NOT fall back to a sub-package's `baseUrl` when the
/// root has none: a sub-package's baseUrl (e.g. `apps/web/.`) is specific
/// to that package's compilation context, and promoting it to the whole
/// workspace would cause `ts_alias_for_pattern` to incorrectly prefix
/// every target pattern with `apps/web/` a second time (since target
/// patterns are already rewritten project-relative by
/// `detect_ts_paths_multi`). Per-package scoped baseUrl resolution is
/// documented as future work.
#[allow(clippy::ptr_arg)]
fn detect_ts_base_url_multi(root: &Path, _extra_roots: &[PathBuf]) -> Option<PathBuf> {
    detect_ts_base_url_from_root(root)
}

fn detect_ts_base_url_from_root(root: &Path) -> Option<PathBuf> {
    let configs = load_tsconfig_chain(root.join("tsconfig.json"));
    for config in configs.iter().rev() {
        let compiler = config.json.get("compilerOptions")?;
        let base_url = compiler.get("baseUrl")?.as_str()?;
        let base_url = base_url.trim();
        if base_url.is_empty() {
            continue;
        }
        let base_dir = config.path.parent().unwrap_or(root);
        return Some(base_dir.join(base_url));
    }
    None
}

#[allow(dead_code)] // retained for callers not using multi-root discovery
fn detect_ts_paths(root: &Path) -> Vec<TsPathMapping> {
    detect_ts_paths_multi(root, &[])
}

/// Merge `tsconfig.json` path mappings from `root` and every entry in
/// `extra_roots` (VAL-007). Duplicate alias patterns across roots resolve
/// with last-wins semantics: per-root scoped resolution (so `@/` from
/// `apps/web/tsconfig.json` only resolves for imports rooted under
/// `apps/web/**`) is future work.
fn detect_ts_paths_multi(root: &Path, extra_roots: &[PathBuf]) -> Vec<TsPathMapping> {
    let mut merged: HashMap<String, Vec<String>> = HashMap::new();

    // Primary root first. Its tsconfig (if any) declares paths at the top
    // level, so no prefix rewriting is needed.
    collect_ts_paths_from_root(root, root, &mut merged);
    // Each extra root's tsconfig needs its target patterns rewritten to be
    // relative to the PRIMARY project root, so `ts_alias_for_pattern`
    // (which operates in project-relative space) can match them against
    // scanned files.
    for extra in extra_roots {
        if extra == root {
            continue;
        }
        collect_ts_paths_from_root(extra, root, &mut merged);
    }

    let mut mappings: Vec<TsPathMapping> = merged
        .into_iter()
        .map(|(alias, targets)| TsPathMapping {
            alias_pattern: alias,
            target_patterns: targets,
        })
        .collect();
    mappings.sort_by(|a, b| a.alias_pattern.cmp(&b.alias_pattern));
    mappings
}

/// Populate `merged` with path aliases from the tsconfig chain rooted at
/// `package_root/tsconfig.json`, rewriting target patterns into the space
/// of `primary_root` (VAL-007).
///
/// If `package_root == primary_root`, patterns are stored as-written.
/// Otherwise, each relative target is prefixed with the subdirectory
/// (e.g. `src/*` from `apps/web/tsconfig.json` becomes `apps/web/src/*`)
/// so `ts_alias_for_pattern` — which matches against project-relative
/// file paths — can resolve imports to the right file.
fn collect_ts_paths_from_root(
    package_root: &Path,
    primary_root: &Path,
    merged: &mut HashMap<String, Vec<String>>,
) {
    let configs = load_tsconfig_chain(package_root.join("tsconfig.json"));
    for config in configs {
        let config_dir = config.path.parent().unwrap_or(package_root).to_path_buf();
        // Compute the offset from the primary project root so targets
        // can be rewritten into that space. If the tsconfig lives AT
        // `primary_root` (or outside it), the offset is empty and
        // targets stay as-written.
        let prefix = config_dir
            .strip_prefix(primary_root)
            .ok()
            .map(|r| r.to_path_buf())
            .filter(|p| !p.as_os_str().is_empty());
        if let Some(compiler) = config.json.get("compilerOptions") {
            if let Some(paths) = compiler.get("paths") {
                extract_ts_paths_with_prefix(paths, prefix.as_deref(), merged);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TsConfig {
    path: PathBuf,
    json: JsonValue,
}

fn load_tsconfig_chain(path: PathBuf) -> Vec<TsConfig> {
    let mut visited = HashSet::new();
    load_tsconfig_chain_inner(path, 0, &mut visited)
}

fn load_tsconfig_chain_inner(
    path: PathBuf,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
) -> Vec<TsConfig> {
    if depth > 5 {
        return Vec::new();
    }
    let canonical = dunce::canonicalize(&path).unwrap_or(path);
    if !visited.insert(canonical.clone()) {
        return Vec::new();
    }
    let json = match read_json_with_comments(canonical.clone()) {
        Some(j) => j,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    if let Some(extends) = json.get("extends").and_then(|v| v.as_str()) {
        if let Some(ext_path) = resolve_tsconfig_extends(&canonical, extends) {
            out.extend(load_tsconfig_chain_inner(ext_path, depth + 1, visited));
        }
    }
    out.push(TsConfig {
        path: canonical,
        json,
    });
    out
}

fn resolve_tsconfig_extends(base: &Path, extends: &str) -> Option<PathBuf> {
    let ext = extends.trim();
    if ext.is_empty() {
        return None;
    }
    if !(ext.starts_with('.') || ext.starts_with('/')) {
        // Package-based extends (e.g., @org/tsconfig) are out of scope.
        return None;
    }
    let base_dir = base.parent().unwrap_or(Path::new("."));
    let mut path = if ext.starts_with('/') {
        PathBuf::from(ext)
    } else {
        base_dir.join(ext)
    };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    Some(path)
}

#[allow(dead_code)] // retained for single-root callers; see extract_ts_paths_with_prefix
fn extract_ts_paths(value: &JsonValue, out: &mut HashMap<String, Vec<String>>) {
    extract_ts_paths_with_prefix(value, None, out);
}

/// Parse a tsconfig `paths` object and insert each alias -> target mapping
/// into `out`. When `subdir_prefix` is provided (e.g. `apps/web`), relative
/// target patterns are rewritten to be relative to the primary project
/// root — so `@/*` -> `src/*` declared in `apps/web/tsconfig.json`
/// becomes `@/* -> apps/web/src/*` in the merged table (VAL-007).
///
/// Patterns that are absolute OR start with `../` are left unchanged
/// (the former are user-authored absolute paths, the latter would escape
/// the workspace — both cases are out of scope for v1).
fn extract_ts_paths_with_prefix(
    value: &JsonValue,
    subdir_prefix: Option<&Path>,
    out: &mut HashMap<String, Vec<String>>,
) {
    let map = match value.as_object() {
        Some(m) => m,
        None => return,
    };
    for (alias, targets) in map {
        let mut patterns = Vec::new();
        if let Some(path) = targets.as_str() {
            patterns.push(rewrite_ts_path_target(path, subdir_prefix));
        } else if let Some(list) = targets.as_array() {
            for item in list {
                if let Some(path) = item.as_str() {
                    patterns.push(rewrite_ts_path_target(path, subdir_prefix));
                }
            }
        }
        if !patterns.is_empty() {
            out.insert(alias.to_string(), patterns);
        }
    }
}

/// Rewrite a tsconfig target pattern so it is relative to the primary
/// project root (instead of the tsconfig's own directory). Absolute
/// patterns and escaping `../` patterns are returned unchanged.
fn rewrite_ts_path_target(target: &str, subdir_prefix: Option<&Path>) -> String {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if Path::new(trimmed).is_absolute() {
        return trimmed.to_string();
    }
    // Strip leading `./` so we don't build `apps/web/./src/*`.
    let stripped = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if stripped.starts_with("../") {
        // Out of scope: parent-dir references from a per-package tsconfig.
        return stripped.to_string();
    }
    match subdir_prefix {
        Some(prefix) if !prefix.as_os_str().is_empty() => {
            let joined = prefix.join(stripped);
            normalize_relative_str(&joined)
        }
        _ => stripped.to_string(),
    }
}

fn detect_php_psr4(root: &Path) -> Vec<(String, PathBuf)> {
    let mut mappings = Vec::new();
    let json = match read_json_file(root.join("composer.json")) {
        Some(j) => j,
        None => return mappings,
    };
    for key in ["autoload", "autoload-dev"] {
        if let Some(section) = json.get(key) {
            if let Some(psr4) = section.get("psr-4") {
                extract_psr4_mappings(root, psr4, &mut mappings);
            }
        }
    }
    mappings
}

fn extract_psr4_mappings(root: &Path, value: &JsonValue, out: &mut Vec<(String, PathBuf)>) {
    let map = match value.as_object() {
        Some(m) => m,
        None => return,
    };
    for (prefix, paths) in map {
        if let Some(path) = paths.as_str() {
            out.push((prefix.to_string(), root.join(path)));
        } else if let Some(list) = paths.as_array() {
            for item in list {
                if let Some(path) = item.as_str() {
                    out.push((prefix.to_string(), root.join(path)));
                }
            }
        }
    }
}

fn read_json_file(path: PathBuf) -> Option<JsonValue> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

fn read_json_with_comments(path: PathBuf) -> Option<JsonValue> {
    let content = std::fs::read_to_string(path).ok()?;
    let stripped = strip_json_comments(&content);
    serde_json::from_str(&stripped).ok()
}

fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            out.push(ch);
            in_string = !in_string;
            continue;
        }
        if !in_string && ch == '/' {
            if let Some('/') = chars.peek().copied() {
                // line comment
                chars.next();
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
                continue;
            }
            if let Some('*') = chars.peek().copied() {
                // block comment
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '*' {
                        if let Some('/') = chars.peek().copied() {
                            chars.next();
                            break;
                        }
                    }
                }
                continue;
            }
        }
        out.push(ch);
    }
    out
}

// =============================================================================
// Language-specific module naming helpers
// =============================================================================

const JAVA_PREFIXES: [&str; 5] = ["src/main/java/", "src/test/java/", "src/", "lib/", "app/"];
const KOTLIN_PREFIXES: [&str; 5] = [
    "src/main/kotlin/",
    "src/test/kotlin/",
    "src/",
    "lib/",
    "app/",
];
const SCALA_PREFIXES: [&str; 5] = ["src/main/scala/", "src/test/scala/", "src/", "lib/", "app/"];
const CSHARP_PREFIXES: [&str; 3] = ["src/", "lib/", "app/"];
const PHP_PREFIXES: [&str; 5] = ["src/", "lib/", "app/", "public/", "includes/"];
const RUBY_PREFIXES: [&str; 3] = ["lib/", "src/", "app/"];
const LUA_PREFIXES: [&str; 3] = ["src/", "lib/", "scripts/"];
const SWIFT_PREFIXES: [&str; 2] = ["src/", "lib/"];
const OCAML_PREFIXES: [&str; 3] = ["src/", "lib/", "app/"];
const TS_EXTENSIONS: [&str; 6] = [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

fn normalize_relative_str(path: &Path) -> String {
    let mut rel = path.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = rel.strip_prefix("./") {
        rel = stripped.to_string();
    }
    rel.trim_start_matches('/').to_string()
}

fn is_ts_index_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(
        file_name,
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "index.mjs" | "index.cjs"
    )
}

fn ts_path_aliases_for_file(
    path: &Path,
    root: &Path,
    base_url: Option<&PathBuf>,
    mappings: &[TsPathMapping],
) -> Vec<String> {
    if mappings.is_empty() {
        return Vec::new();
    }
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let rel_str = normalize_relative_str(relative);
    let rel_no_ext = strip_extension_any(&rel_str, &TS_EXTENSIONS);
    let mut candidates = Vec::new();
    if !rel_no_ext.is_empty() {
        candidates.push(rel_no_ext.to_string());
    }
    if is_ts_index_file(path) {
        if let Some(parent) = Path::new(rel_no_ext).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                candidates.push(parent_str);
            }
        }
    }

    let base_prefix = base_url
        .and_then(|p| p.strip_prefix(root).ok())
        .map(normalize_relative_str)
        .filter(|s| !s.is_empty());

    let mut aliases = Vec::new();
    for candidate in candidates {
        for mapping in mappings {
            for target_pattern in &mapping.target_patterns {
                if let Some(alias) = ts_alias_for_pattern(
                    &candidate,
                    target_pattern,
                    base_prefix.as_deref(),
                    &mapping.alias_pattern,
                ) {
                    aliases.push(alias);
                }
            }
        }
    }
    aliases
}

fn ts_alias_for_pattern(
    candidate: &str,
    target_pattern: &str,
    base_prefix: Option<&str>,
    alias_pattern: &str,
) -> Option<String> {
    let mut pattern = target_pattern.trim().replace('\\', "/");
    if let Some(stripped) = pattern.strip_prefix("./") {
        pattern = stripped.to_string();
    }
    if let Some(base) = base_prefix {
        if !pattern.starts_with("../") && !pattern.starts_with('/') {
            let base = base.trim_end_matches('/');
            if !base.is_empty() {
                pattern = format!("{}/{}", base, pattern);
            }
        }
    }
    let pattern = strip_extension_any(&pattern, &TS_EXTENSIONS);

    let capture = match_ts_path_pattern(candidate, pattern)?;
    let mut alias = alias_pattern.replace('*', &capture);
    if alias.ends_with('/') {
        alias = alias.trim_end_matches('/').to_string();
    }
    if alias.is_empty() {
        None
    } else {
        Some(alias)
    }
}

fn match_ts_path_pattern(candidate: &str, pattern: &str) -> Option<String> {
    if let Some(star_pos) = pattern.find('*') {
        let (prefix, rest) = pattern.split_at(star_pos);
        let suffix = &rest[1..];
        if candidate.starts_with(prefix) && candidate.ends_with(suffix) {
            let mid_end = candidate.len().saturating_sub(suffix.len());
            let mid = &candidate[prefix.len()..mid_end];
            return Some(mid.to_string());
        }
        return None;
    }
    if candidate == pattern {
        return Some(String::new());
    }
    None
}

fn strip_known_prefixes<'a>(path: &'a str, prefixes: &[&str]) -> &'a str {
    let mut best_end: Option<usize> = None;
    let mut best_prefix_len: usize = 0;
    for prefix in prefixes {
        if let Some(pos) = path.find(prefix) {
            // Only match at start of path or after a '/' boundary
            if (pos == 0 || path.as_bytes()[pos - 1] == b'/') && prefix.len() > best_prefix_len {
                best_prefix_len = prefix.len();
                best_end = Some(pos + prefix.len());
            }
        }
    }
    if let Some(end) = best_end {
        &path[end..]
    } else {
        path
    }
}

fn strip_extension_any<'a>(path: &'a str, extensions: &[&str]) -> &'a str {
    for ext in extensions {
        if let Some(stripped) = path.strip_suffix(ext) {
            return stripped;
        }
    }
    path
}

fn compute_dot_module_name(
    path: &Path,
    root: &Path,
    prefixes: &[&str],
    extensions: &[&str],
) -> String {
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let rel_str = normalize_relative_str(relative);
    let rel_str = strip_known_prefixes(&rel_str, prefixes);
    let rel_str = strip_extension_any(rel_str, extensions);
    if rel_str.is_empty() {
        return String::new();
    }
    rel_str
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn compute_separator_module_name(
    path: &Path,
    root: &Path,
    prefixes: &[&str],
    extensions: &[&str],
    separator: char,
) -> String {
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let rel_str = normalize_relative_str(relative);
    let rel_str = strip_known_prefixes(&rel_str, prefixes);
    let rel_str = strip_extension_any(rel_str, extensions);
    if rel_str.is_empty() {
        return String::new();
    }
    if separator == '/' {
        rel_str.to_string()
    } else {
        rel_str.replace('/', &separator.to_string())
    }
}

fn compute_path_module_name(path: &Path, root: &Path) -> String {
    let relative = match path.strip_prefix(root) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    normalize_relative_str(relative)
}

fn snake_to_camel(segment: &str) -> String {
    segment
        .split(['_', '-'])
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = String::new();
                    out.push(first.to_ascii_uppercase());
                    out.push_str(chars.as_str());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Capitalize the first letter of an OCaml module segment.
///
/// OCaml derives the module name from the file basename by capitalizing
/// the first letter only — underscores stay underscores. So `util.ml`
/// becomes module `Util`, and `my_helper.ml` becomes `My_helper` (NOT
/// `MyHelper`, which would be the snake_to_camel transform). This is the
/// conversion used by both `dune` and `ocamlfind`.
fn capitalize_first_ocaml_segment(segment: &str) -> String {
    let mut chars = segment.chars();
    match chars.next() {
        Some(first) => {
            let mut out = String::new();
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn swift_module_from_sources(rest: &str) -> String {
    let rest = strip_extension_any(rest, &[".swift"]);
    let mut parts = rest.split('/').filter(|s| !s.is_empty());
    let module = parts.next().unwrap_or("");
    if module.is_empty() {
        return String::new();
    }
    let remainder: Vec<&str> = parts.collect();
    if remainder.is_empty() {
        module.to_string()
    } else {
        format!("{}.{}", module, remainder.join("."))
    }
}

fn swift_module_root(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let rel_str = normalize_relative_str(relative);
    if let Some(rest) = rel_str.strip_prefix("Sources/") {
        return rest.split('/').next().map(|s| s.to_string());
    }
    if let Some(rest) = rel_str.strip_prefix("Tests/") {
        return rest.split('/').next().map(|s| s.to_string());
    }
    None
}

fn ruby_module_alias_from_path(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let rel_str = normalize_relative_str(relative);
    let rel_str = strip_known_prefixes(&rel_str, &RUBY_PREFIXES);
    let rel_str = strip_extension_any(rel_str, &[".rb"]);
    if rel_str.is_empty() {
        return None;
    }
    let parts: Vec<String> = rel_str
        .split('/')
        .filter(|s| !s.is_empty())
        .map(snake_to_camel)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("::"))
    }
}

fn parse_java_like_package(path: &Path) -> Option<String> {
    let source = fs::read_to_string(path).ok()?;
    lazy_static! {
        static ref RE_PACKAGE: Regex =
            Regex::new(r"(?m)^\\s*package\\s+([A-Za-z_][\\w\\.]*)\\s*;?").unwrap();
    }
    RE_PACKAGE.captures(&source).map(|caps| caps[1].to_string())
}

fn parse_csharp_namespace(path: &Path) -> Option<String> {
    let source = fs::read_to_string(path).ok()?;
    lazy_static! {
        static ref RE_NAMESPACE: Regex =
            Regex::new(r"(?m)^\\s*namespace\\s+([A-Za-z_][\\w\\.]*)").unwrap();
    }
    RE_NAMESPACE
        .captures(&source)
        .map(|caps| caps[1].to_string())
}

fn simple_module_name(module: &str) -> &str {
    module.rsplit(['.', '/', '\\']).next().unwrap_or(module)
}

/// Check if a directory should be skipped during traversal.
fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        "__pycache__"
            | ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "venv"
            | ".venv"
            | "env"
            | ".env"
            | ".tox"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | "__pypackages__"
            | "target"
            | "build"
            | "dist"
            | ".idea"
            | ".vscode"
    )
}

/// Get file extensions for a language.
fn get_language_extensions(language: &str) -> Vec<&'static str> {
    match language.to_lowercase().as_str() {
        "python" => vec!["py"],
        "typescript" => vec!["ts", "tsx"],
        "javascript" => vec!["js", "jsx", "mjs", "cjs"],
        "rust" => vec!["rs"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        "c" => vec!["c", "h"],
        "cpp" | "c++" => vec!["cpp", "cc", "cxx", "hpp", "hh", "hxx", "h"],
        "ruby" => vec!["rb"],
        "php" => vec!["php"],
        "kotlin" => vec!["kt", "kts"],
        "scala" => vec!["scala"],
        "swift" => vec!["swift"],
        "csharp" | "c#" => vec!["cs"],
        "lua" => vec!["lua"],
        "luau" => vec!["lua", "luau"],
        "elixir" => vec!["ex", "exs"],
        "ocaml" => vec!["ml", "mli"],
        _ => vec!["py"], // Default to Python
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // =============================================================================
    // build() tests
    // =============================================================================

    #[test]
    fn test_build_simple_package() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/core.py"), "def foo(): pass").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.lookup("pkg").is_some());
        assert!(index.lookup("pkg.core").is_some());
    }

    #[test]
    fn test_build_indexes_init_as_package() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        let pkg_path = index.lookup("pkg");
        assert!(pkg_path.is_some());
        assert!(pkg_path.unwrap().ends_with("__init__.py"));
    }

    #[test]
    fn test_build_package_wins_over_module() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "# package").unwrap();
        fs::write(dir.path().join("pkg.py"), "# module").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        let pkg_path = index.lookup("pkg");
        assert!(pkg_path.is_some());
        // Package wins - should point to __init__.py
        assert!(pkg_path.unwrap().ends_with("__init__.py"));
    }

    #[test]
    fn test_build_namespace_package() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/module.py"), "").unwrap();
        // No __init__.py - namespace package

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.is_namespace_package("pkg"));
        assert!(index.lookup("pkg.module").is_some());
    }

    #[test]
    fn test_build_skips_pycache() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
        fs::write(dir.path().join("__pycache__/module.cpython-311.pyc"), "").unwrap();
        fs::write(dir.path().join("module.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.lookup("module").is_some());
        // No __pycache__ entries
        for (module, _) in index.iter() {
            assert!(!module.contains("__pycache__"));
        }
    }

    #[test]
    fn test_build_deeply_nested_package() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b/c/d/e")).unwrap();
        for pkg in ["a", "a/b", "a/b/c", "a/b/c/d", "a/b/c/d/e"] {
            fs::write(dir.path().join(format!("{}/__init__.py", pkg)), "").unwrap();
        }
        fs::write(dir.path().join("a/b/c/d/e/f.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.lookup("a.b.c.d.e.f").is_some());
        assert!(index.lookup("a.b.c").is_some());
    }

    // =============================================================================
    // lookup() tests
    // =============================================================================

    #[test]
    fn test_lookup_returns_file_path() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/core.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        let path = index.lookup("pkg.core");
        assert!(path.is_some());
        assert!(path.unwrap().ends_with("core.py"));
    }

    #[test]
    fn test_lookup_returns_none_for_nonexistent() {
        let dir = tempdir().unwrap();
        let index = ModuleIndex::build(dir.path(), "python").unwrap();
        assert!(index.lookup("nonexistent.module").is_none());
    }

    // =============================================================================
    // reverse_lookup() tests
    // =============================================================================

    #[test]
    fn test_reverse_lookup_returns_module_path() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/core.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        let module = index.reverse_lookup(&dir.path().join("pkg/core.py"));
        assert_eq!(module, Some("pkg.core"));
    }

    #[test]
    fn test_reverse_lookup_returns_none_for_unknown() {
        let dir = tempdir().unwrap();
        let index = ModuleIndex::build(dir.path(), "python").unwrap();
        assert!(index
            .reverse_lookup(Path::new("/unknown/path.py"))
            .is_none());
    }

    // =============================================================================
    // is_project_module() tests
    // =============================================================================

    #[test]
    fn test_is_project_module_returns_true_for_indexed() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/core.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.is_project_module("pkg"));
        assert!(index.is_project_module("pkg.core"));
    }

    #[test]
    fn test_is_project_module_returns_false_for_stdlib() {
        let dir = tempdir().unwrap();
        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(!index.is_project_module("os"));
        assert!(!index.is_project_module("sys"));
        assert!(!index.is_project_module("json.decoder"));
    }

    // =============================================================================
    // Edge cases
    // =============================================================================

    #[test]
    fn test_empty_directory() {
        let dir = tempdir().unwrap();
        let index = ModuleIndex::build(dir.path(), "python").unwrap();
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn test_single_file_no_package() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("script.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();
        assert!(index.lookup("script").is_some());
    }

    #[test]
    fn test_mixed_extensions() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("module.py"), "").unwrap();
        fs::write(dir.path().join("config.json"), "").unwrap();
        fs::write(dir.path().join("README.md"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.lookup("module").is_some());
        assert_eq!(index.len(), 1); // Only .py file
    }

    #[test]
    fn test_dunder_names() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/__main__.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        assert!(index.lookup("pkg.__main__").is_some());
    }

    #[test]
    fn test_private_modules() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg/_internal")).unwrap();
        fs::write(dir.path().join("pkg/__init__.py"), "").unwrap();
        fs::write(dir.path().join("pkg/_private.py"), "").unwrap();
        fs::write(dir.path().join("pkg/_internal/__init__.py"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "python").unwrap();

        // Private modules should still be indexed
        assert!(index.lookup("pkg._private").is_some());
        assert!(index.lookup("pkg._internal").is_some());
    }

    // =============================================================================
    // TypeScript tests
    // =============================================================================

    #[test]
    fn test_typescript_index_file() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("utils")).unwrap();
        fs::write(dir.path().join("utils/index.ts"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "typescript").unwrap();

        assert!(index.lookup("./utils").is_some());
    }

    // =============================================================================
    // Rust tests
    // =============================================================================

    #[test]
    fn test_rust_lib_rs() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "rust").unwrap();

        assert!(index.lookup("crate").is_some());
    }

    #[test]
    fn test_rust_mod_rs() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src/utils")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        fs::write(dir.path().join("src/utils/mod.rs"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "rust").unwrap();

        assert!(index.lookup("crate::utils").is_some());
    }

    // =============================================================================
    // Go tests
    // =============================================================================

    #[test]
    fn test_go_package() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("pkg/utils")).unwrap();
        fs::write(dir.path().join("pkg/utils/helpers.go"), "").unwrap();

        let index = ModuleIndex::build(dir.path(), "go").unwrap();

        assert!(index.lookup("pkg/utils").is_some());
    }
}
