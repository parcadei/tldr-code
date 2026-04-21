//! File discovery and filtering for clone detection.

use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::is_generated_file;

/// Check if a file appears to be a test file.
///
/// Matches common test file naming conventions across languages:
/// - Python: test_*.py, *_test.py
/// - Go: *_test.go
/// - Rust: *_test.rs (integration tests; unit tests in same file are not filtered)
/// - Ruby: *_spec.rb
/// - JavaScript/TypeScript: *.test.ts, *.test.js, *.spec.ts, *.spec.js
/// - Java: *Test.java
/// - C#: *Tests.cs
/// - Directories: tests/, test/, __tests__/, spec/, testing/
pub fn is_test_file(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy())
        .unwrap_or_default();

    // Check test directory patterns (with leading slash)
    if path_str.contains("/tests/")
        || path_str.contains("/test/")
        || path_str.contains("/__tests__/")
        || path_str.contains("/spec/")
        || path_str.contains("/testing/")
    {
        return true;
    }

    // Also handle paths that START with test directory names
    // (relative paths without leading slash)
    if path_str.starts_with("tests/")
        || path_str.starts_with("test/")
        || path_str.starts_with("__tests__/")
        || path_str.starts_with("spec/")
        || path_str.starts_with("testing/")
    {
        return true;
    }

    // Check test file name patterns
    let name = file_name.as_ref();
    name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_test.go")
        || name.ends_with("_test.rs")
        || name.ends_with("_spec.rb")
        || name.ends_with(".test.ts")
        || name.ends_with(".test.js")
        || name.ends_with(".spec.ts")
        || name.ends_with(".spec.js")
        || name.ends_with("Test.java")
        || name.ends_with("Tests.cs")
}

/// Discover source files for clone detection.
/// Wraps walkdir with extension filter, max_files cap, test/generated exclusion.
pub fn discover_source_files(
    path: &Path,
    language: Option<&str>,
    max_files: usize,
    exclude_generated: bool,
    exclude_tests: bool,
) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for entry in WalkDir::new(path).into_iter() {
        match entry {
            Ok(e) => {
                if !e.file_type().is_file() {
                    continue;
                }
                let file_path = e.path();

                // Skip generated files if requested
                if exclude_generated && is_generated_file(file_path) {
                    continue;
                }

                // Skip test files if requested
                if exclude_tests && is_test_file(file_path) {
                    continue;
                }

                if is_source_file_for_clones(file_path, language) {
                    files.push(file_path.to_path_buf());
                    if files.len() >= max_files {
                        break;
                    }
                }
            }
            Err(_) => {
                // Skip entries we can't read
            }
        }
    }

    files
}

/// Check if a file is a source file for clone detection
fn is_source_file_for_clones(path: &Path, language: Option<&str>) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());

    match (ext, language) {
        // If language specified, only match that language's extension
        (Some("py"), Some("python")) => true,
        (Some("ts" | "tsx"), Some("typescript")) => true,
        (Some("js" | "jsx"), Some("javascript")) => true,
        (Some("go"), Some("go")) => true,
        (Some("rs"), Some("rust")) => true,
        (Some("java"), Some("java")) => true,
        (Some("c" | "h"), Some("c")) => true,
        (Some("cs"), Some("csharp")) => true,
        (Some("ex" | "exs"), Some("elixir")) => true,
        (Some("lua"), Some("lua")) => true,
        (Some("ml" | "mli"), Some("ocaml")) => true,
        (Some("php"), Some("php")) => true,
        (Some("rb"), Some("ruby")) => true,
        (Some("scala"), Some("scala")) => true,
        (Some("swift"), Some("swift")) => true,
        (Some("kt" | "kts"), Some("kotlin")) => true,
        (Some("cpp" | "cc" | "cxx" | "hpp"), Some("cpp")) => true,
        (Some("luau"), Some("luau")) => true,

        // If no language specified, accept common source files
        (
            Some(
                "py" | "ts" | "tsx" | "js" | "jsx" | "go" | "rs" | "java" | "c" | "h" | "cs" | "ex"
                | "exs" | "lua" | "ml" | "mli" | "php" | "rb" | "scala" | "swift" | "kt" | "kts"
                | "cpp" | "cc" | "cxx" | "hpp" | "luau",
            ),
            None,
        ) => true,

        _ => false,
    }
}

/// Get language name from file extension
pub fn get_language_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    match ext {
        "py" => Some("python"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "go" => Some("go"),
        "rs" => Some("rust"),
        "java" => Some("java"),
        "c" | "h" => Some("c"),
        "cpp" | "cc" | "cxx" | "hpp" => Some("cpp"),
        "cs" => Some("csharp"),
        "ex" | "exs" => Some("elixir"),
        "lua" => Some("lua"),
        "luau" => Some("luau"),
        "ml" | "mli" => Some("ocaml"),
        "php" => Some("php"),
        "rb" => Some("ruby"),
        "scala" => Some("scala"),
        "swift" => Some("swift"),
        "kt" | "kts" => Some("kotlin"),
        _ => None,
    }
}
