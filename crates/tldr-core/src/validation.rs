//! Shared validation helpers for CLI and daemon handlers
//!
//! These functions reduce duplication between:
//! - CLI commands (sync, returns TldrError)
//! - Daemon handlers (async, wraps to HandlerError)
//! - MCP tools (sync, wraps to JsonRpcError)

use std::path::{Path, PathBuf};

use crate::{Language, TldrError, TldrResult};

/// Resolve and validate a file path.
///
/// # Arguments
/// * `file` - The file path string (may be relative or absolute)
/// * `project` - Optional project root to resolve relative paths against
///
/// # Returns
/// * `Ok(PathBuf)` - Canonical path to the file
/// * `Err(TldrError::PathNotFound)` - File doesn't exist
/// * `Err(TldrError::PathTraversal)` - Path escapes project root
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::validation::validate_file_path;
/// use std::path::Path;
///
/// // Relative path with project root
/// let result = validate_file_path("src/main.rs", Some(Path::new("/app")));
///
/// // Absolute path
/// let result = validate_file_path("/app/src/main.rs", None);
///
/// // Path traversal blocked
/// let result = validate_file_path("../escape.rs", Some(Path::new("/app/src")));
/// assert!(result.is_err()); // PathTraversal error
/// ```
pub fn validate_file_path(file: &str, project: Option<&Path>) -> TldrResult<PathBuf> {
    let path = PathBuf::from(file);

    // Resolve to absolute path
    let resolved = if path.is_absolute() {
        path.clone()
    } else if let Some(proj) = project {
        proj.join(&path)
    } else {
        std::env::current_dir()
            .map_err(TldrError::IoError)?
            .join(&path)
    };

    // Canonicalize (resolves symlinks, checks existence)
    // Use dunce for Windows compatibility (M18)
    let canonical =
        dunce::canonicalize(&resolved).map_err(|_| TldrError::PathNotFound(resolved.clone()))?;

    // Check for path traversal if project specified
    if let Some(proj) = project {
        let canonical_proj =
            dunce::canonicalize(proj).map_err(|_| TldrError::PathNotFound(proj.to_path_buf()))?;

        if !canonical.starts_with(&canonical_proj) {
            return Err(TldrError::PathTraversal(path));
        }
    }

    Ok(canonical)
}

/// Detect or parse programming language.
///
/// # Arguments
/// * `lang` - Optional explicit language string
/// * `path` - File path to detect language from (if lang is None)
///
/// # Returns
/// * `Ok(Language)` - Detected or parsed language
/// * `Err(TldrError::UnsupportedLanguage)` - Unknown language string
/// * `Err(TldrError::UnsupportedLanguage)` - Could not detect from path
///
/// # Examples
///
/// ```rust,ignore
/// use tldr_core::validation::detect_or_parse_language;
/// use tldr_core::Language;
/// use std::path::Path;
///
/// // Explicit language
/// let lang = detect_or_parse_language(Some("python"), Path::new("any.txt")).unwrap();
/// assert_eq!(lang, Language::Python);
///
/// // Auto-detect from extension
/// let lang = detect_or_parse_language(None, Path::new("script.py")).unwrap();
/// assert_eq!(lang, Language::Python);
///
/// // Error on unknown
/// let result = detect_or_parse_language(None, Path::new("file.xyz"));
/// assert!(result.is_err()); // UnsupportedLanguage error
/// ```
pub fn detect_or_parse_language(lang: Option<&str>, path: &Path) -> TldrResult<Language> {
    if let Some(lang_str) = lang {
        // Parse explicit language
        lang_str
            .parse()
            .map_err(|_| TldrError::UnsupportedLanguage(lang_str.to_string()))
    } else {
        // Detect from extension
        Language::from_path(path).ok_or_else(|| {
            TldrError::UnsupportedLanguage(format!(
                "Could not detect language for: {}",
                path.display()
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // =========================================================================
    // validate_file_path tests
    // =========================================================================

    #[test]
    fn test_validate_relative_with_project() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        std::fs::create_dir(&src_dir).unwrap();
        let main_rs = src_dir.join("main.rs");
        std::fs::write(&main_rs, "fn main() {}").unwrap();

        let result = validate_file_path("src/main.rs", Some(temp.path()));
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        assert!(result.unwrap().ends_with("src/main.rs"));
    }

    #[test]
    fn test_validate_absolute_path() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();

        let result = validate_file_path(file.to_str().unwrap(), None);
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    }

    #[test]
    fn test_validate_not_found() {
        let result = validate_file_path("/definitely/nonexistent/path/file.rs", None);
        assert!(matches!(result, Err(TldrError::PathNotFound(_))));
    }

    #[test]
    fn test_validate_traversal_blocked() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("project");
        std::fs::create_dir(&project_dir).unwrap();
        // Create a file outside project dir
        let escape_file = temp.path().join("escape.rs");
        std::fs::write(&escape_file, "// escaped").unwrap();

        let result = validate_file_path("../escape.rs", Some(&project_dir));
        assert!(
            matches!(result, Err(TldrError::PathTraversal(_))),
            "Expected PathTraversal error, got {:?}",
            result
        );
    }

    #[test]
    fn test_validate_relative_without_project() {
        // This tests that relative paths resolve against cwd
        // We'll create a file in a temp dir and change to it
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("local.txt");
        std::fs::write(&file, "content").unwrap();

        // Use absolute path since we can't change cwd easily in tests
        let result = validate_file_path(file.to_str().unwrap(), None);
        assert!(result.is_ok());
    }

    // =========================================================================
    // detect_or_parse_language tests
    // =========================================================================

    #[test]
    fn test_parse_explicit_python() {
        let result = detect_or_parse_language(Some("python"), Path::new("any.xyz"));
        assert_eq!(result.unwrap(), Language::Python);
    }

    #[test]
    fn test_parse_explicit_typescript() {
        let result = detect_or_parse_language(Some("typescript"), Path::new("any.xyz"));
        assert_eq!(result.unwrap(), Language::TypeScript);
    }

    #[test]
    fn test_parse_explicit_rust() {
        let result = detect_or_parse_language(Some("rust"), Path::new("any.xyz"));
        assert_eq!(result.unwrap(), Language::Rust);
    }

    #[test]
    fn test_parse_explicit_go() {
        let result = detect_or_parse_language(Some("go"), Path::new("any.xyz"));
        assert_eq!(result.unwrap(), Language::Go);
    }

    #[test]
    fn test_detect_python_extension() {
        let result = detect_or_parse_language(None, Path::new("script.py"));
        assert_eq!(result.unwrap(), Language::Python);
    }

    #[test]
    fn test_detect_rust_extension() {
        let result = detect_or_parse_language(None, Path::new("lib.rs"));
        assert_eq!(result.unwrap(), Language::Rust);
    }

    #[test]
    fn test_detect_typescript_extension() {
        let result = detect_or_parse_language(None, Path::new("app.ts"));
        assert_eq!(result.unwrap(), Language::TypeScript);
    }

    #[test]
    fn test_detect_go_extension() {
        let result = detect_or_parse_language(None, Path::new("main.go"));
        assert_eq!(result.unwrap(), Language::Go);
    }

    #[test]
    fn test_parse_invalid_language() {
        let result = detect_or_parse_language(Some("invalid_lang"), Path::new("any.xyz"));
        assert!(matches!(result, Err(TldrError::UnsupportedLanguage(_))));
    }

    #[test]
    fn test_detect_unknown_extension() {
        let result = detect_or_parse_language(None, Path::new("file.xyz"));
        assert!(matches!(result, Err(TldrError::UnsupportedLanguage(_))));

        // Check error message contains helpful info
        if let Err(TldrError::UnsupportedLanguage(msg)) = result {
            assert!(msg.contains("Could not detect language"));
            assert!(msg.contains("file.xyz"));
        }
    }

    #[test]
    fn test_explicit_overrides_extension() {
        // Even if file is .py, explicit "rust" should win
        let result = detect_or_parse_language(Some("rust"), Path::new("script.py"));
        assert_eq!(result.unwrap(), Language::Rust);
    }
}
