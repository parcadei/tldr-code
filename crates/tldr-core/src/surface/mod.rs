//! API surface extraction for libraries and packages.
//!
//! This module extracts machine-readable API surfaces from installed packages,
//! producing structured data about every public function, method, class, constant,
//! and type alias. The output includes:
//!
//! - Qualified names and module paths
//! - Typed signatures with parameter defaults
//! - Docstrings (first paragraph, truncated)
//! - Example usage strings (templated from types)
//! - Trigger keywords for intent-based retrieval
//!
//! # Relationship to behavioral contracts
//!
//! The CLI `contracts` command (in `tldr-cli/src/commands/contracts/`) extracts
//! *behavioral* contracts (pre/postconditions from guard clauses and assertions).
//! This module (`surface`) extracts *structural* API shapes from a library.
//! They are complementary, not overlapping.
//!
//! # Supported languages
//!
//! - **Python** (Phase 1): Full support via tree-sitter + C extension fallback
//! - **Rust** (Phase 3): Full support via tree-sitter + Cargo.toml resolution
//! - **TypeScript** (Phase 4): Full support via tree-sitter + .d.ts parsing
//! - **Go** (Phase 5): Full support via tree-sitter + exported-name filtering
//! - **JavaScript** (Phase 6): Full support via tree-sitter + JSDoc parsing

pub mod examples;
pub mod c_lang;
pub mod elixir;
pub mod go;
pub mod java;
pub mod javascript;
pub mod kotlin;
pub mod lua;
pub mod php;
pub mod python;
pub mod resolve;
pub mod cpp;
pub mod scala;
pub mod swift;
pub mod csharp;
pub mod ruby;
pub mod rust_lang;
pub mod triggers;
pub mod typescript;
pub mod language_profile;
pub mod types;

#[cfg(test)]
mod hardening;

// Re-export core types for public API
pub use types::{ApiEntry, ApiKind, ApiSurface, Location, Param, Signature};

use crate::TldrResult;
use language_profile::static_preference_score;

/// Extract the complete API surface for a package.
///
/// This is the main entry point. It resolves the package path, determines the
/// language, and dispatches to the language-specific extractor.
///
/// # Arguments
/// * `target` - Package name (e.g., "flask") or directory path
/// * `lang` - Optional language hint (auto-detected if not specified)
/// * `include_private` - Whether to include private APIs
/// * `limit` - Optional maximum number of APIs
/// * `lookup` - Optional: return only the API matching this qualified name
///
/// # Returns
/// * `Ok(ApiSurface)` - The extracted API surface
/// * `Err(TldrError)` - If resolution or extraction fails
pub fn extract_api_surface(
    target: &str,
    lang: Option<&str>,
    include_private: bool,
    limit: Option<usize>,
    lookup: Option<&str>,
) -> TldrResult<ApiSurface> {
    // Resolve the target to a package directory
    let resolved = resolve::resolve_target(target, lang)?;

    // Determine language and dispatch
    let detected = lang.or_else(|| detect_lang_from_path(target));
    let effective_lang = detected.unwrap_or("python");

    let mut surface = match effective_lang {
        "c" => c_lang::extract_c_api_surface(&resolved, include_private, limit)?,
        "cpp" => cpp::extract_cpp_api_surface(&resolved, include_private, limit)?,
        "elixir" => elixir::extract_elixir_api_surface(&resolved, include_private, limit)?,
        "csharp" => csharp::extract_csharp_api_surface(&resolved, include_private, limit)?,
        "python" => python::extract_python_api_surface(&resolved, include_private, limit)?,
        "rust" => rust_lang::extract_rust_api_surface(&resolved, include_private, limit)?,
        "typescript" => {
            typescript::extract_typescript_api_surface(&resolved, include_private, limit)?
        }
        "go" => go::extract_go_api_surface(&resolved, include_private, limit)?,
        "java" => java::extract_java_api_surface(&resolved, include_private, limit)?,
        "javascript" | "js" => {
            javascript::extract_javascript_api_surface(&resolved, include_private, limit)?
        }
        "kotlin" => kotlin::extract_kotlin_api_surface(&resolved, include_private, limit)?,
        "lua" => lua::extract_lua_api_surface(&resolved, include_private, limit)?,
        "php" => php::extract_php_api_surface(&resolved, include_private, limit)?,
        "scala" => scala::extract_scala_api_surface(&resolved, include_private, limit)?,
        "swift" => swift::extract_swift_api_surface(&resolved, include_private, limit)?,
        "ruby" => ruby::extract_ruby_api_surface(&resolved, include_private, limit)?,
        other => {
            return Err(crate::error::TldrError::UnsupportedLanguage(format!(
                "API surface extraction not yet supported for language: {}",
                other
            )))
        }
    };

    // Handle --lookup: filter to a single API entry
    if let Some(lookup_name) = lookup {
        surface.apis.retain(|api| {
            api.qualified_name == lookup_name
                || api.qualified_name.ends_with(&format!(".{}", lookup_name))
        });
        surface.total = surface.apis.len();
    }

    Ok(surface)
}

pub(crate) fn sort_apis_by_static_preference(apis: &mut [ApiEntry], language: &str) {
    apis.sort_by(|left, right| {
        let left_score = api_static_preference_score(left, language);
        let right_score = api_static_preference_score(right, language);

        right_score
            .cmp(&left_score)
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            .then_with(|| left.module.cmp(&right.module))
            .then_with(|| api_location_path(left).cmp(&api_location_path(right)))
            .then_with(|| api_location_line(left).cmp(&api_location_line(right)))
            .then_with(|| left.kind.to_string().cmp(&right.kind.to_string()))
    });
}

fn api_static_preference_score(api: &ApiEntry, language: &str) -> i32 {
    api.location
        .as_ref()
        .map(|location| static_preference_score(language, &location.file))
        .unwrap_or_default()
}

fn api_location_path(api: &ApiEntry) -> String {
    api.location
        .as_ref()
        .map(|location| location.file.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn api_location_line(api: &ApiEntry) -> usize {
    api.location
        .as_ref()
        .map(|location| location.line)
        .unwrap_or_default()
}

/// Detect language from a file path or directory.
///
/// For file paths, returns `Some(lang)` when the target has a recognisable
/// extension. For directories, scans the immediate contents and returns the
/// language of the most common recognised extension.
///
/// Returns `None` for bare package names (no `/` or `.`) so callers can fall
/// back to their own default. Also returns `None` for empty directories or
/// directories with no recognised source files.
fn detect_lang_from_path(target: &str) -> Option<&'static str> {
    let path = std::path::Path::new(target);

    // If the target is a directory on disk, scan its contents.
    if path.is_dir() {
        return detect_lang_from_directory(path);
    }

    // Only attempt extension-based detection when the target looks like a file
    // path: it must contain a '/' or a '.' (i.e., have an extension).
    if !target.contains('/') && !target.contains('.') {
        return None;
    }

    detect_lang_from_filename(target)
}

/// Map a single file name/path to a language based on its extension.
fn detect_lang_from_filename(target: &str) -> Option<&'static str> {
    // Strip a leading directory component so we can match the file name.
    let file_name = target.rsplit('/').next().unwrap_or(target);

    // Handle compound extensions first (e.g., ".d.ts").
    if file_name.ends_with(".d.ts") {
        return Some("typescript");
    }

    // Match on the final extension.
    let ext = file_name.rsplit('.').next().unwrap_or("");
    match ext {
        "py" => Some("python"),
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some("cpp"),
        "js" | "mjs" => Some("javascript"),
        "cs" => Some("csharp"),
        "kt" | "kts" => Some("kotlin"),
        "lua" => Some("lua"),
        "php" => Some("php"),
        "scala" => Some("scala"),
        "swift" => Some("swift"),
        "rb" => Some("ruby"),
        "ex" | "exs" => Some("elixir"),
        _ => None,
    }
}

/// Scan a directory's immediate children and detect the dominant language.
///
/// Counts files by recognised language and returns the one with the most files.
/// Handles compound extensions like `.d.ts` by checking file names before
/// falling back to `std::path::Path::extension()`.
fn detect_lang_from_directory(dir: &std::path::Path) -> Option<&'static str> {
    let mut counts: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();

    detect_lang_from_directory_recursive(dir, &mut counts, 0);

    counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(lang, _)| lang)
}

/// Recursively scan up to 3 levels deep to find source files for language detection.
fn detect_lang_from_directory_recursive(
    dir: &std::path::Path,
    counts: &mut std::collections::HashMap<&'static str, usize>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(lang) = detect_lang_from_filename(name) {
                    *counts.entry(lang).or_insert(0) += 1;
                }
            }
        } else if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with('.') && !matches!(name, "node_modules" | "target" | "vendor" | "__pycache__" | ".git") {
                    detect_lang_from_directory_recursive(&path, counts, depth + 1);
                }
            }
        }
    }
}

/// Format an API surface as human-readable text.
pub fn format_api_surface_text(surface: &ApiSurface) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "API Surface: {} ({}) - {} APIs\n",
        surface.package, surface.language, surface.total
    ));
    output.push_str(&"=".repeat(60));
    output.push('\n');

    for api in &surface.apis {
        output.push('\n');

        // Kind and name
        output.push_str(&format!("[{}] {}\n", api.kind, api.qualified_name));

        // Signature
        if let Some(sig) = &api.signature {
            let params_str: Vec<String> = sig
                .params
                .iter()
                .map(|p| {
                    let mut s = p.name.clone();
                    if p.is_variadic {
                        s = format!("*{}", s);
                    }
                    if p.is_keyword {
                        s = format!("**{}", s);
                    }
                    if let Some(t) = &p.type_annotation {
                        s = format!("{}: {}", s, t);
                    }
                    if let Some(d) = &p.default {
                        s = format!("{} = {}", s, d);
                    }
                    s
                })
                .collect();

            let ret = sig
                .return_type
                .as_ref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default();

            let async_prefix = if sig.is_async { "async " } else { "" };

            output.push_str(&format!(
                "  {}({}){}\n",
                async_prefix,
                params_str.join(", "),
                ret
            ));
        }

        // Docstring
        if let Some(doc) = &api.docstring {
            output.push_str(&format!("  {}\n", doc));
        }

        // Example
        if let Some(ex) = &api.example {
            output.push_str(&format!("  Example: {}\n", ex));
        }

        // Triggers
        if !api.triggers.is_empty() {
            output.push_str(&format!("  Triggers: {}\n", api.triggers.join(", ")));
        }

        // Location
        if let Some(loc) = &api.location {
            output.push_str(&format!("  Location: {}:{}\n", loc.file.display(), loc.line));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::detect_lang_from_path;

    #[test]
    fn test_detect_lang_python() {
        assert_eq!(detect_lang_from_path("/tmp/test_api.py"), Some("python"));
        assert_eq!(detect_lang_from_path("script.py"), Some("python"));
    }

    #[test]
    fn test_detect_lang_rust() {
        assert_eq!(detect_lang_from_path("/src/lib.rs"), Some("rust"));
        assert_eq!(detect_lang_from_path("main.rs"), Some("rust"));
    }

    #[test]
    fn test_detect_lang_typescript() {
        assert_eq!(detect_lang_from_path("/tmp/test_api.ts"), Some("typescript"));
        assert_eq!(detect_lang_from_path("app.tsx"), Some("typescript"));
        assert_eq!(detect_lang_from_path("index.d.ts"), Some("typescript"));
        assert_eq!(detect_lang_from_path("/types/foo.d.ts"), Some("typescript"));
    }

    #[test]
    fn test_detect_lang_go() {
        assert_eq!(detect_lang_from_path("main.go"), Some("go"));
        assert_eq!(detect_lang_from_path("/project/server.go"), Some("go"));
    }

    #[test]
    fn test_detect_lang_javascript() {
        assert_eq!(detect_lang_from_path("index.js"), Some("javascript"));
        assert_eq!(detect_lang_from_path("module.mjs"), Some("javascript"));
        assert_eq!(detect_lang_from_path("/src/app.js"), Some("javascript"));
    }

    #[test]
    fn test_detect_lang_java() {
        assert_eq!(detect_lang_from_path("Main.java"), Some("java"));
        assert_eq!(detect_lang_from_path("/srv/src/App.java"), Some("java"));
    }

    #[test]
    fn test_detect_lang_kotlin() {
        assert_eq!(detect_lang_from_path("Main.kt"), Some("kotlin"));
        assert_eq!(detect_lang_from_path("build.gradle.kts"), Some("kotlin"));
    }

    #[test]
    fn test_detect_lang_csharp() {
        assert_eq!(detect_lang_from_path("Program.cs"), Some("csharp"));
        assert_eq!(detect_lang_from_path("/srv/src/App.cs"), Some("csharp"));
    }

    #[test]
    fn test_detect_lang_scala() {
        assert_eq!(detect_lang_from_path("Main.scala"), Some("scala"));
        assert_eq!(detect_lang_from_path("/srv/src/App.scala"), Some("scala"));
    }

    #[test]
    fn test_detect_lang_php() {
        assert_eq!(detect_lang_from_path("index.php"), Some("php"));
        assert_eq!(detect_lang_from_path("/srv/public/app.php"), Some("php"));
    }

    #[test]
    fn test_detect_lang_swift() {
        assert_eq!(detect_lang_from_path("App.swift"), Some("swift"));
        assert_eq!(detect_lang_from_path("/srv/Sources/App.swift"), Some("swift"));
    }

    #[test]
    fn test_detect_lang_c() {
        assert_eq!(detect_lang_from_path("api.h"), Some("c"));
        assert_eq!(detect_lang_from_path("main.c"), Some("c"));
    }

    #[test]
    fn test_detect_lang_cpp() {
        assert_eq!(detect_lang_from_path("api.hpp"), Some("cpp"));
        assert_eq!(detect_lang_from_path("main.cpp"), Some("cpp"));
    }

    #[test]
    fn test_detect_lang_lua() {
        assert_eq!(detect_lang_from_path("init.lua"), Some("lua"));
        assert_eq!(detect_lang_from_path("/srv/lua/app.lua"), Some("lua"));
    }

    #[test]
    fn test_detect_lang_ruby() {
        assert_eq!(detect_lang_from_path("app.rb"), Some("ruby"));
        assert_eq!(detect_lang_from_path("/srv/lib/service.rb"), Some("ruby"));
    }

    #[test]
    fn test_detect_lang_elixir() {
        assert_eq!(detect_lang_from_path("app.ex"), Some("elixir"));
        assert_eq!(detect_lang_from_path("config.exs"), Some("elixir"));
    }

    #[test]
    fn test_bare_package_name_returns_none() {
        // No extension, no slash → bare package name → no detection
        assert_eq!(detect_lang_from_path("flask"), None);
        assert_eq!(detect_lang_from_path("requests"), None);
        assert_eq!(detect_lang_from_path("serde"), None);
    }

    #[test]
    fn test_path_with_slash_unknown_ext_returns_none() {
        // Path-like but unrecognised extension → None
        assert_eq!(detect_lang_from_path("/tmp/file.xyz"), None);
    }

    #[test]
    fn test_unknown_extension_returns_none() {
        // Has a dot but unrecognised extension → None
        assert_eq!(detect_lang_from_path("file.toml"), None);
    }

    #[test]
    fn test_detect_lang_directory_with_dts_files() {
        // A directory containing .d.ts files should auto-detect as TypeScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.d.ts"), "export declare const x: number;").unwrap();
        std::fs::write(dir.path().join("types.d.ts"), "export interface Foo {}").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("typescript"),
            "directory containing .d.ts files should detect as typescript"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_ts_files() {
        // A directory containing .ts files should auto-detect as TypeScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.ts"), "const x: number = 1;").unwrap();
        std::fs::write(dir.path().join("utils.ts"), "export function foo() {}").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("typescript"),
            "directory containing .ts files should detect as typescript"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_py_files() {
        // A directory containing .py files should auto-detect as Python.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("__init__.py"), "").unwrap();
        std::fs::write(dir.path().join("module.py"), "def foo(): pass").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("python"),
            "directory containing .py files should detect as python"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_rs_files() {
        // A directory containing .rs files should auto-detect as Rust.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn foo() {}").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("rust"),
            "directory containing .rs files should detect as rust"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_go_files() {
        // A directory containing .go files should auto-detect as Go.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.go"), "package main").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("go"),
            "directory containing .go files should detect as go"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_js_files() {
        // A directory containing .js files should auto-detect as JavaScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.js"), "module.exports = {}").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("javascript"),
            "directory containing .js files should detect as javascript"
        );
    }

    #[test]
    fn test_detect_lang_empty_directory_returns_none() {
        // An empty directory should return None.
        let dir = tempfile::tempdir().unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            None,
            "empty directory should return None"
        );
    }

    #[test]
    fn test_detect_lang_directory_with_mixed_dts_and_ts() {
        // A directory with both .d.ts and .ts should detect as TypeScript.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("types.d.ts"), "export type X = string;").unwrap();
        std::fs::write(dir.path().join("app.ts"), "const x = 1;").unwrap();

        let path_str = dir.path().to_str().unwrap();
        assert_eq!(
            detect_lang_from_path(path_str),
            Some("typescript"),
            "directory with .d.ts and .ts should detect as typescript"
        );
    }
}
