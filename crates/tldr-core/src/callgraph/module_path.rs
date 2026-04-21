//! Module path conversion and definition extraction.
//!
//! This module handles converting file paths to language-specific module names
//! and dispatching definition extraction to the appropriate language handler.
//!
//! Extracted from builder_v2.rs as part of Phase 4 modularization.

use std::path::{Path, PathBuf};

use super::languages::LanguageRegistry;
use super::types::parse_source;
use super::var_types::{
    extract_go_var_types, extract_java_var_types, extract_kotlin_var_types, extract_php_var_types,
    extract_python_definitions, extract_rust_var_types, extract_ts_var_types, FileParseResult,
};

/// Extract functions, classes, imports, and calls from a source file based on language.
///
/// For Python, uses the dedicated built-in extractor which handles funcs, classes,
/// imports, and calls in a single pass (preserving existing behavior and call type
/// classification).
///
/// For all other languages, uses the language handler registry to dispatch to the
/// appropriate handler for imports and calls extraction. Function/class definition
/// extraction is not yet in the trait, so those are empty for non-Python languages.
/// The calls+imports are the critical path for cross-file call graph construction.
pub(crate) fn extract_definitions(
    source: &str,
    file_path: &Path,
    language: &str,
) -> FileParseResult {
    // Python: use the dedicated built-in extractor (preserves existing behavior)
    if language.to_lowercase() == "python" {
        return extract_python_definitions(source, file_path);
    }

    // All other languages: use the language handler registry
    let registry = LanguageRegistry::with_defaults();
    if let Some(handler) = registry.get(language) {
        // Parse imports via the language handler
        let imports = handler.parse_imports(source, file_path).unwrap_or_default();

        // Parse the tree using the builder's own parser (handles all languages)
        let tree = match parse_source(source, language) {
            Ok(t) => t,
            Err(e) => {
                return FileParseResult {
                    error: Some(format!("Parse failed: {}", e)),
                    ..Default::default()
                };
            }
        };

        // Extract calls via the language handler
        let calls = handler
            .extract_calls(file_path, source, &tree)
            .unwrap_or_default();

        // Extract function and class definitions via the language handler
        let (funcs, classes_defs) = handler
            .extract_definitions(source, file_path, &tree)
            .unwrap_or_default();

        // Extract VarType information for supported languages
        let var_types = match language.to_lowercase().as_str() {
            "go" => extract_go_var_types(&tree, source.as_bytes()),
            "typescript" | "javascript" => extract_ts_var_types(&tree, source.as_bytes()),
            "java" => extract_java_var_types(&tree, source.as_bytes()),
            "rust" => extract_rust_var_types(&tree, source.as_bytes()),
            "kotlin" => extract_kotlin_var_types(&tree, source.as_bytes()),
            "php" => extract_php_var_types(&tree, source.as_bytes()),
            _ => Vec::new(),
        };

        // Explicitly drop the tree to free memory (per spec M1.4)
        drop(tree);

        FileParseResult {
            funcs,
            classes: classes_defs,
            imports,
            calls,
            var_types,
            error: None,
        }
    } else {
        // Fallback for truly unknown languages not in the registry
        FileParseResult::default()
    }
}

/// Normalize a file path to be relative to the project root.
///
/// This function handles various edge cases:
/// - Tries canonical paths first (resolves symlinks, `.`, `..`)
/// - Falls back to string manipulation if canonicalization fails
/// - Removes leading `../` sequences by matching against root's basename
/// - Converts backslashes to forward slashes
///
/// # P1 Implementation (parity-fix-plan.yaml)
/// Matches Python V2 behavior: `str(py_path.relative_to(root))`
///
pub(crate) fn normalize_path_relative_to_root(
    file_path: &Path,
    root: &Path,
    canonical_root: Option<&Path>,
) -> PathBuf {
    // Try canonicalization approach first
    if let (Some(canon_root), Ok(canon_file)) = (canonical_root, file_path.canonicalize()) {
        if let Ok(relative) = canon_file.strip_prefix(canon_root) {
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            return PathBuf::from(rel_str);
        }
    }

    // Try direct strip_prefix
    if let Ok(relative) = file_path.strip_prefix(root) {
        let rel_str = relative.to_string_lossy().replace('\\', "/");
        return PathBuf::from(rel_str);
    }

    // Fallback: Handle cases like `../tldr/foo.py` with root `../tldr`
    // Extract the base name from root and remove it from file_path
    let file_str = file_path.to_string_lossy();
    let root_str = root.to_string_lossy();

    // If file_path starts with root_str, strip it
    if let Some(stripped) = file_str.strip_prefix(root_str.as_ref()) {
        let cleaned = stripped.trim_start_matches('/').trim_start_matches('\\');
        return PathBuf::from(cleaned.replace('\\', "/"));
    }

    // Last resort: use the file_path as-is but normalize slashes
    PathBuf::from(file_str.replace('\\', "/"))
}

/// Converts a file path to a module name, using language-specific conventions.
///
/// The module name format must match what `ModuleIndex::compute_module_name()` produces
/// so that func_index keys align with import_map keys during call resolution.
///
/// Language conventions:
/// - **Python**: dot-separated, no prefix: `pkg.module` (strips `src/`, `lib/`)
/// - **TypeScript/JavaScript**: `./` prefix, slash-separated: `./utils`, `./v4/core/errors`
/// - **Go**: directory path, slash-separated, no prefix: `pkg/utils`
/// - **Rust**: `crate::` prefix, `::` separated: `crate::utils::helpers`
/// - **Other**: defaults to Python-style (dot-separated)
///
/// # Examples
/// ```text
/// path_to_module("src/pkg/module.py", "python")    -> "pkg.module"
/// path_to_module("errors.ts", "typescript")         -> "./errors"
/// path_to_module("pkg/utils/helpers.go", "go")      -> "pkg/utils"
/// path_to_module("src/utils/helpers.rs", "rust")    -> "crate::utils::helpers"
/// ```
pub fn path_to_module(path: &Path, language: &str) -> String {
    let lang = language.to_lowercase();
    match lang.as_str() {
        "typescript" | "javascript" => path_to_module_typescript(path),
        "go" => path_to_module_go(path),
        "rust" => path_to_module_rust(path),
        "java" => path_to_module_java(path),
        "kotlin" => path_to_module_kotlin(path),
        "scala" => path_to_module_scala(path),
        "csharp" | "c#" => path_to_module_csharp(path),
        "php" => path_to_module_php(path),
        "ruby" => path_to_module_ruby(path),
        "lua" | "luau" => path_to_module_lua(path),
        "elixir" => path_to_module_elixir(path),
        "swift" => path_to_module_swift(path),
        "c" => path_to_module_c(path),
        "cpp" | "c++" => path_to_module_cpp(path),
        "ocaml" => path_to_module_ocaml(path),
        _ => path_to_module_python(path),
    }
}

/// Python module name: dot-separated, strips src/lib prefix, handles __init__.py.
fn path_to_module_python(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");

    // Remove common prefixes
    let path_str = path_str
        .strip_prefix("src/")
        .or_else(|| path_str.strip_prefix("lib/"))
        .unwrap_or(&path_str);

    // Remove extension
    let path_str = path_str
        .strip_suffix(".py")
        .or_else(|| path_str.strip_suffix(".rs"))
        .or_else(|| path_str.strip_suffix(".ts"))
        .or_else(|| path_str.strip_suffix(".tsx"))
        .or_else(|| path_str.strip_suffix(".js"))
        .or_else(|| path_str.strip_suffix(".jsx"))
        .or_else(|| path_str.strip_suffix(".go"))
        .unwrap_or(path_str);

    // Handle __init__.py -> package name
    let path_str = path_str.strip_suffix("/__init__").unwrap_or(path_str);

    // Convert slashes to dots
    path_str.replace('/', ".")
}

/// TypeScript/JavaScript module name: ./ prefix, slash-separated, handles index files.
/// Matches `ModuleIndex::compute_typescript_module_name()`.
fn path_to_module_typescript(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Handle index.ts/index.tsx/index.js -> parent directory
    if file_name == "index.ts" || file_name == "index.tsx" || file_name == "index.js" {
        let parent = path.parent().unwrap_or(Path::new(""));
        let parent_str = parent.to_string_lossy().replace('\\', "/");
        return format!("./{}", parent_str);
    }

    // Strip extension
    let stripped = path_str
        .strip_suffix(".ts")
        .or_else(|| path_str.strip_suffix(".tsx"))
        .or_else(|| path_str.strip_suffix(".js"))
        .or_else(|| path_str.strip_suffix(".jsx"))
        .or_else(|| path_str.strip_suffix(".mjs"))
        .or_else(|| path_str.strip_suffix(".cjs"))
        .unwrap_or(&path_str);

    format!("./{}", stripped)
}

/// Go module name: directory path (slash-separated, no prefix).
/// Matches `ModuleIndex::compute_go_module_name()`.
fn path_to_module_go(path: &Path) -> String {
    // Go uses directory as package
    path.parent()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

/// Rust module name: crate:: prefix, :: separated, handles lib.rs/main.rs/mod.rs.
/// Matches `ModuleIndex::compute_rust_module_name()`.
fn path_to_module_rust(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Handle lib.rs/main.rs -> crate root
    if file_name == "lib.rs" || file_name == "main.rs" {
        return "crate".to_string();
    }

    // Handle mod.rs -> parent module
    if file_name == "mod.rs" {
        let parent = path.parent().unwrap_or(Path::new(""));
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

    // Regular module: strip extension, skip src/
    let stripped = path_str.strip_suffix(".rs").unwrap_or(&path_str);

    let parts: Vec<&str> = stripped
        .split('/')
        .filter(|s| *s != "src" && !s.is_empty())
        .collect();

    if parts.is_empty() {
        return "crate".to_string();
    }
    format!("crate::{}", parts.join("::"))
}

// =============================================================================
// Additional language module naming helpers (V2 canonical)
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

fn normalize_rel_str(path: &Path) -> String {
    let mut rel = path.to_string_lossy().replace('\\', "/");
    if let Some(stripped) = rel.strip_prefix("./") {
        rel = stripped.to_string();
    }
    rel.trim_start_matches('/').to_string()
}

fn strip_known_prefixes<'a>(path: &'a str, prefixes: &[&str]) -> &'a str {
    let mut best_end: Option<usize> = None;
    let mut best_prefix_len: usize = 0;
    for prefix in prefixes {
        if let Some(pos) = path.find(prefix) {
            // Only match at start of path or after a '/' boundary
            if (pos == 0 || path.as_bytes()[pos - 1] == b'/')
                && prefix.len() > best_prefix_len {
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

fn dot_module_from_path(path: &Path, prefixes: &[&str], extensions: &[&str]) -> String {
    let rel = normalize_rel_str(path);
    let rel = strip_known_prefixes(&rel, prefixes);
    let rel = strip_extension_any(rel, extensions);
    if rel.is_empty() {
        return String::new();
    }
    rel.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn separator_module_from_path(
    path: &Path,
    prefixes: &[&str],
    extensions: &[&str],
    separator: char,
) -> String {
    let rel = normalize_rel_str(path);
    let rel = strip_known_prefixes(&rel, prefixes);
    let rel = strip_extension_any(rel, extensions);
    if rel.is_empty() {
        return String::new();
    }
    if separator == '/' {
        rel.to_string()
    } else {
        rel.replace('/', &separator.to_string())
    }
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

fn path_to_module_java(path: &Path) -> String {
    dot_module_from_path(path, &JAVA_PREFIXES, &[".java"])
}

fn path_to_module_kotlin(path: &Path) -> String {
    dot_module_from_path(path, &KOTLIN_PREFIXES, &[".kt", ".kts"])
}

fn path_to_module_scala(path: &Path) -> String {
    dot_module_from_path(path, &SCALA_PREFIXES, &[".scala"])
}

fn path_to_module_csharp(path: &Path) -> String {
    dot_module_from_path(path, &CSHARP_PREFIXES, &[".cs"])
}

fn path_to_module_php(path: &Path) -> String {
    separator_module_from_path(path, &PHP_PREFIXES, &[".php"], '\\')
}

fn path_to_module_ruby(path: &Path) -> String {
    separator_module_from_path(path, &RUBY_PREFIXES, &[".rb"], '/')
}

fn path_to_module_lua(path: &Path) -> String {
    dot_module_from_path(path, &LUA_PREFIXES, &[".lua", ".luau"])
}

fn path_to_module_elixir(path: &Path) -> String {
    let rel = normalize_rel_str(path);
    let mut module_parts: Vec<String> = Vec::new();

    if let Some(rest) = rel.strip_prefix("apps/") {
        let mut parts = rest.splitn(2, '/');
        if let Some(app) = parts.next() {
            if let Some(after_app) = parts.next() {
                if let Some(after_lib) = after_app.strip_prefix("lib/") {
                    module_parts.push(snake_to_camel(app));
                    let stripped = strip_extension_any(after_lib, &[".ex", ".exs"]);
                    for seg in stripped.split('/') {
                        if seg.is_empty() {
                            continue;
                        }
                        module_parts.push(snake_to_camel(seg));
                    }
                    return module_parts.join(".");
                }
            }
        }
    }

    let rel = if let Some(after_lib) = rel.strip_prefix("lib/") {
        after_lib
    } else {
        rel.as_str()
    };
    let rel = strip_extension_any(rel, &[".ex", ".exs"]);
    for seg in rel.split('/') {
        if seg.is_empty() {
            continue;
        }
        module_parts.push(snake_to_camel(seg));
    }
    module_parts.join(".")
}

fn path_to_module_swift(path: &Path) -> String {
    let rel = normalize_rel_str(path);
    if let Some(rest) = rel.strip_prefix("Sources/") {
        return swift_module_from_sources(rest);
    }
    if let Some(rest) = rel.strip_prefix("Tests/") {
        return swift_module_from_sources(rest);
    }
    dot_module_from_path(path, &SWIFT_PREFIXES, &[".swift"])
}

fn path_to_module_c(path: &Path) -> String {
    normalize_rel_str(path)
}

fn path_to_module_cpp(path: &Path) -> String {
    normalize_rel_str(path)
}

fn path_to_module_ocaml(path: &Path) -> String {
    dot_module_from_path(path, &OCAML_PREFIXES, &[".ml", ".mli"])
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    
    
    use super::*;
    
    use std::path::Path;

    // NOTE: Tests moved from builder_v2.rs (Phase 4 modularization).

    // =========================================================================
    // Tests for language registry wiring in extract_definitions
    // =========================================================================

    /// Test: extract_definitions produces imports for TypeScript source.
    /// Before the registry wiring, this returned empty for all non-Python languages.
    #[test]
    fn test_extract_definitions_typescript_imports() {
        let ts_source = r#"
import { useState } from 'react';
import axios from 'axios';

function App() {
    const [data, setData] = useState(null);
    axios.get('/api').then(res => setData(res.data));
}
"#;
        let result = extract_definitions(ts_source, Path::new("app.tsx"), "typescript");
        assert!(
            !result.imports.is_empty(),
            "TypeScript extract_definitions should produce imports, got empty"
        );
        // Should find 'react' and 'axios' imports
        let import_modules: Vec<&str> = result.imports.iter().map(|i| i.module.as_str()).collect();
        assert!(
            import_modules.iter().any(|m| m.contains("react")),
            "Should find react import, got: {:?}",
            import_modules
        );
    }

    /// Test: extract_definitions produces calls for TypeScript source.
    #[test]
    fn test_extract_definitions_typescript_calls() {
        let ts_source = r#"
import { readFile } from 'fs';

function loadConfig(path: string) {
    readFile(path, (err, data) => {
        console.log(data);
    });
}

function main() {
    loadConfig('./config.json');
}
"#;
        let result = extract_definitions(ts_source, Path::new("main.ts"), "typescript");
        assert!(
            !result.calls.is_empty(),
            "TypeScript extract_definitions should produce calls, got empty"
        );
    }

    /// Test: extract_definitions still works for Python (regression).
    #[test]
    fn test_extract_definitions_python_regression() {
        let py_source = r#"
import os

def greet(name):
    print(f"Hello, {name}")

def main():
    greet("world")
    os.path.exists("/tmp")
"#;
        let result = extract_definitions(py_source, Path::new("main.py"), "python");
        assert!(
            !result.funcs.is_empty(),
            "Python should still find functions"
        );
        assert!(
            !result.imports.is_empty(),
            "Python should still find imports"
        );
        assert!(!result.calls.is_empty(), "Python should still find calls");
    }

    /// Test: extract_definitions returns empty for truly unknown languages.
    #[test]
    fn test_extract_definitions_unknown_language() {
        let source = "some code";
        let result = extract_definitions(source, Path::new("test.bf"), "brainfuck");
        assert!(result.funcs.is_empty());
        assert!(result.classes.is_empty());
        assert!(result.imports.is_empty());
        assert!(result.calls.is_empty());
    }

    /// Test: extract_definitions produces imports for Go source.
    #[test]
    fn test_extract_definitions_go_imports() {
        let go_source = r#"
package main

import (
    "fmt"
    "os"
)

func main() {
    fmt.Println("hello")
    os.Exit(0)
}
"#;
        let result = extract_definitions(go_source, Path::new("main.go"), "go");
        assert!(
            !result.imports.is_empty(),
            "Go extract_definitions should produce imports, got empty"
        );
    }

    /// Test: extract_definitions produces imports for Rust source.
    #[test]
    fn test_extract_definitions_rust_imports() {
        let rust_source = r#"
use std::collections::HashMap;
use std::path::Path;

fn process(map: &HashMap<String, String>) {
    println!("{:?}", map);
}
"#;
        let result = extract_definitions(rust_source, Path::new("lib.rs"), "rust");
        assert!(
            !result.imports.is_empty(),
            "Rust extract_definitions should produce imports, got empty"
        );
    }

    // =========================================================================
    // Tests for path_to_module language-awareness (module name mismatch fix)
    // =========================================================================

    #[test]
    fn test_path_to_module_python_unchanged() {
        // Python: dot-separated, no prefix (current behavior must be preserved)
        assert_eq!(
            path_to_module(Path::new("myapp/utils.py"), "python"),
            "myapp.utils"
        );
        assert_eq!(
            path_to_module(Path::new("pkg/sub/module.py"), "python"),
            "pkg.sub.module"
        );
        assert_eq!(path_to_module(Path::new("module.py"), "python"), "module");
        assert_eq!(
            path_to_module(Path::new("pkg/__init__.py"), "python"),
            "pkg"
        );
    }

    #[test]
    fn test_path_to_module_python_strips_src_prefix() {
        // Python: should strip src/ and lib/ prefixes
        assert_eq!(
            path_to_module(Path::new("src/pkg/module.py"), "python"),
            "pkg.module"
        );
        assert_eq!(
            path_to_module(Path::new("lib/pkg/module.py"), "python"),
            "pkg.module"
        );
    }

    #[test]
    fn test_path_to_module_typescript_uses_dot_slash_prefix() {
        // TypeScript: must use ./ prefix, slash-separated (matching ModuleIndex)
        assert_eq!(
            path_to_module(Path::new("errors.ts"), "typescript"),
            "./errors"
        );
        assert_eq!(
            path_to_module(Path::new("v4/core/errors.ts"), "typescript"),
            "./v4/core/errors"
        );
        assert_eq!(
            path_to_module(Path::new("utils.tsx"), "typescript"),
            "./utils"
        );
        assert_eq!(
            path_to_module(Path::new("helpers.js"), "javascript"),
            "./helpers"
        );
    }

    #[test]
    fn test_path_to_module_typescript_index_files() {
        // TypeScript: index.ts maps to parent directory
        assert_eq!(
            path_to_module(Path::new("utils/index.ts"), "typescript"),
            "./utils"
        );
        assert_eq!(
            path_to_module(Path::new("v4/core/index.tsx"), "typescript"),
            "./v4/core"
        );
    }

    #[test]
    fn test_path_to_module_go_slash_separated() {
        // Go: directory path is the package (slash-separated, no prefix)
        assert_eq!(
            path_to_module(Path::new("pkg/utils/helpers.go"), "go"),
            "pkg/utils"
        );
        assert_eq!(path_to_module(Path::new("cmd/main.go"), "go"), "cmd");
        assert_eq!(path_to_module(Path::new("main.go"), "go"), "");
    }

    #[test]
    fn test_path_to_module_rust_crate_prefix() {
        // Rust: crate:: prefix, :: separated (matching ModuleIndex)
        assert_eq!(path_to_module(Path::new("src/lib.rs"), "rust"), "crate");
        assert_eq!(path_to_module(Path::new("src/main.rs"), "rust"), "crate");
        assert_eq!(
            path_to_module(Path::new("src/utils/mod.rs"), "rust"),
            "crate::utils"
        );
        assert_eq!(
            path_to_module(Path::new("src/utils/helpers.rs"), "rust"),
            "crate::utils::helpers"
        );
    }

    #[test]
    fn test_path_to_module_default_language_uses_python_style() {
        // Unknown languages should default to Python-style (current behavior)
        assert_eq!(
            path_to_module(Path::new("module.rb"), "unknown"),
            "module.rb"
        );
    }

    // =========================================================================
    // Tests for nested multi-module path stripping (Java/Kotlin/C#/Scala)
    // =========================================================================

    #[test]
    fn test_path_to_module_java_flat() {
        assert_eq!(
            path_to_module(Path::new("src/main/java/com/example/Foo.java"), "java"),
            "com.example.Foo"
        );
        assert_eq!(
            path_to_module(Path::new("com/example/Foo.java"), "java"),
            "com.example.Foo"
        );
    }

    #[test]
    fn test_path_to_module_java_nested() {
        assert_eq!(
            path_to_module(Path::new("backend/src/main/java/com/example/service/UserService.java"), "java"),
            "com.example.service.UserService"
        );
        assert_eq!(
            path_to_module(Path::new("modules/core/src/test/java/com/example/FooTest.java"), "java"),
            "com.example.FooTest"
        );
    }

    #[test]
    fn test_path_to_module_kotlin_flat() {
        assert_eq!(
            path_to_module(Path::new("src/main/kotlin/com/example/Bar.kt"), "kotlin"),
            "com.example.Bar"
        );
    }

    #[test]
    fn test_path_to_module_kotlin_nested() {
        assert_eq!(
            path_to_module(Path::new("app/src/main/kotlin/com/example/ui/MainScreen.kt"), "kotlin"),
            "com.example.ui.MainScreen"
        );
        assert_eq!(
            path_to_module(Path::new("feature/auth/src/main/kotlin/com/example/auth/Login.kt"), "kotlin"),
            "com.example.auth.Login"
        );
    }

    #[test]
    fn test_path_to_module_scala_flat() {
        assert_eq!(
            path_to_module(Path::new("src/main/scala/com/example/Baz.scala"), "scala"),
            "com.example.Baz"
        );
    }

    #[test]
    fn test_path_to_module_scala_nested() {
        assert_eq!(
            path_to_module(Path::new("core/src/main/scala/com/example/domain/Model.scala"), "scala"),
            "com.example.domain.Model"
        );
        assert_eq!(
            path_to_module(Path::new("project/sub/src/test/scala/com/example/SpecTest.scala"), "scala"),
            "com.example.SpecTest"
        );
    }

    #[test]
    fn test_path_to_module_csharp_flat() {
        assert_eq!(
            path_to_module(Path::new("src/Models/User.cs"), "csharp"),
            "Models.User"
        );
    }

    #[test]
    fn test_path_to_module_csharp_nested() {
        assert_eq!(
            path_to_module(Path::new("MyProject/src/Models/User.cs"), "csharp"),
            "Models.User"
        );
        assert_eq!(
            path_to_module(Path::new("backend/app/Controllers/HomeController.cs"), "csharp"),
            "Controllers.HomeController"
        );
    }
}
