//! P4 Shared Validators Tests: DRY validation helpers
//!
//! Tests for validation module implemented in Phase 2 (P4).
//!
//! Contracts:
//! - 2.1: validate_file_path
//! - 2.2: detect_or_parse_language
//! - 2.3: Error mapping helpers

use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tldr_core::{detect_or_parse_language, validate_file_path};
use tldr_core::{Language, TldrError};

// =============================================================================
// Contract 2.1: validate_file_path
//
// This module doesn't exist yet. Tests will fail to compile until implemented.
// The expected function signature is:
//
// pub fn validate_file_path(file: &str, project: Option<&Path>) -> TldrResult<PathBuf>
// =============================================================================

#[cfg(test)]
mod validate_file_path_tests {
    use super::*;

    /// Contract 2.1: Relative path with project should resolve correctly
    #[test]
    fn test_validate_file_path_relative_with_project() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        std::fs::create_dir(&src_dir).unwrap();
        let main_rs = src_dir.join("main.rs");
        std::fs::write(&main_rs, "fn main() {}").unwrap();

        let result = validate_file_path("src/main.rs", Some(temp.path()));
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
        assert!(result.unwrap().ends_with("src/main.rs"));
    }

    /// Contract 2.1: Path traversal attack should be blocked
    #[test]
    fn test_validate_file_path_traversal_blocked() {
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("project");
        std::fs::create_dir(&project_dir).unwrap();
        // Create a file outside project dir that could be accessed via traversal
        let escape_file = temp.path().join("escape.rs");
        std::fs::write(&escape_file, "// escaped").unwrap();

        let result = validate_file_path("../escape.rs", Some(&project_dir));
        assert!(
            matches!(result, Err(TldrError::PathTraversal(_))),
            "Expected PathTraversal error, got {:?}",
            result
        );
    }

    /// Contract 2.1: Non-existent file should return PathNotFound
    #[test]
    fn test_validate_file_path_not_found() {
        let result = validate_file_path("/nonexistent/path/file.rs", None);
        assert!(
            matches!(result, Err(TldrError::PathNotFound(_))),
            "Expected PathNotFound error, got {:?}",
            result
        );
    }
}

// =============================================================================
// Contract 2.2: detect_or_parse_language
//
// Expected function signature:
// pub fn detect_or_parse_language(lang: Option<&str>, path: &Path) -> TldrResult<Language>
// =============================================================================

#[cfg(test)]
mod detect_or_parse_language_tests {
    use super::*;

    /// Contract 2.2: Explicit language string parses correctly
    #[test]
    fn test_parse_explicit_python() {
        let result = detect_or_parse_language(Some("python"), Path::new("any.xyz"));
        assert_eq!(result.unwrap(), Language::Python);
    }

    /// Contract 2.2: Auto-detect from .py extension
    #[test]
    fn test_detect_python_extension() {
        let result = detect_or_parse_language(None, Path::new("script.py"));
        assert_eq!(result.unwrap(), Language::Python);
    }

    /// Contract 2.2: Invalid language returns error
    #[test]
    fn test_parse_invalid_language() {
        let result = detect_or_parse_language(Some("invalid_lang"), Path::new("any.xyz"));
        assert!(
            matches!(result, Err(TldrError::UnsupportedLanguage(_))),
            "Expected UnsupportedLanguage error, got {:?}",
            result
        );
    }

    /// Contract 2.2: Unknown extension returns error
    #[test]
    fn test_detect_unknown_extension() {
        let result = detect_or_parse_language(None, Path::new("file.xyz"));
        assert!(
            matches!(result, Err(TldrError::UnsupportedLanguage(_))),
            "Expected UnsupportedLanguage error, got {:?}",
            result
        );
    }

    // Test Language::from_extension which DOES exist
    #[test]
    fn test_language_from_extension_py() {
        let lang = Language::from_extension(".py");
        assert_eq!(lang, Some(Language::Python));
    }

    #[test]
    fn test_language_from_extension_rs() {
        let lang = Language::from_extension(".rs");
        assert_eq!(lang, Some(Language::Rust));
    }

    #[test]
    fn test_language_from_extension_ts() {
        let lang = Language::from_extension(".ts");
        assert_eq!(lang, Some(Language::TypeScript));
    }

    #[test]
    fn test_language_from_extension_go() {
        let lang = Language::from_extension(".go");
        assert_eq!(lang, Some(Language::Go));
    }

    #[test]
    fn test_language_from_extension_unknown() {
        let lang = Language::from_extension(".xyz");
        assert_eq!(lang, None);
    }
}

// =============================================================================
// Contract 2.3: Error Mapping - Exit Codes
//
// These tests verify the exit_code() method on TldrError, which DOES exist.
// =============================================================================

#[cfg(test)]
mod error_exit_codes_tests {
    use super::*;

    /// Contract 2.3: PathNotFound has exit code 2
    #[test]
    fn test_exit_code_path_not_found() {
        let err = TldrError::PathNotFound(PathBuf::from("/nonexistent"));
        assert_eq!(err.exit_code(), 2, "PathNotFound should have exit code 2");
    }

    /// Contract 2.3: PathTraversal has exit code 3
    #[test]
    fn test_exit_code_path_traversal() {
        let err = TldrError::PathTraversal(PathBuf::from("../escape"));
        assert_eq!(err.exit_code(), 3, "PathTraversal should have exit code 3");
    }

    /// Contract 2.3: SymlinkCycle has exit code 4
    #[test]
    fn test_exit_code_symlink_cycle() {
        let err = TldrError::SymlinkCycle(PathBuf::from("/cycle"));
        assert_eq!(err.exit_code(), 4, "SymlinkCycle should have exit code 4");
    }

    /// Contract 2.3: PermissionDenied has exit code 5
    #[test]
    fn test_exit_code_permission_denied() {
        let err = TldrError::PermissionDenied(PathBuf::from("/secret"));
        assert_eq!(
            err.exit_code(),
            5,
            "PermissionDenied should have exit code 5"
        );
    }

    /// Contract 2.3: ParseError has exit code 10
    #[test]
    fn test_exit_code_parse_error() {
        let err = TldrError::parse_error(PathBuf::from("file.py"), Some(10), "syntax error");
        assert_eq!(err.exit_code(), 10, "ParseError should have exit code 10");
    }

    /// Contract 2.3: UnsupportedLanguage has exit code 11
    #[test]
    fn test_exit_code_unsupported_language() {
        let err = TldrError::UnsupportedLanguage("unknown".to_string());
        assert_eq!(
            err.exit_code(),
            11,
            "UnsupportedLanguage should have exit code 11"
        );
    }

    /// Contract 2.3: FunctionNotFound has exit code 20
    #[test]
    fn test_exit_code_function_not_found() {
        let err = TldrError::function_not_found("missing_fn");
        assert_eq!(
            err.exit_code(),
            20,
            "FunctionNotFound should have exit code 20"
        );
    }

    /// Contract 2.3: InvalidDirection has exit code 21
    #[test]
    fn test_exit_code_invalid_direction() {
        let err = TldrError::InvalidDirection("sideways".to_string());
        assert_eq!(
            err.exit_code(),
            21,
            "InvalidDirection should have exit code 21"
        );
    }

    /// Contract 2.3: LineNotInFunction has exit code 22
    #[test]
    fn test_exit_code_line_not_in_function() {
        let err = TldrError::LineNotInFunction(999);
        assert_eq!(
            err.exit_code(),
            22,
            "LineNotInFunction should have exit code 22"
        );
    }

    /// Contract 2.3: DaemonError has exit code 30
    #[test]
    fn test_exit_code_daemon_error() {
        let err = TldrError::DaemonError("connection failed".to_string());
        assert_eq!(err.exit_code(), 30, "DaemonError should have exit code 30");
    }

    /// Contract 2.3: ConnectionFailed has exit code 31
    #[test]
    fn test_exit_code_connection_failed() {
        let err = TldrError::ConnectionFailed("timeout".to_string());
        assert_eq!(
            err.exit_code(),
            31,
            "ConnectionFailed should have exit code 31"
        );
    }

    /// Contract 2.3: Timeout has exit code 32
    #[test]
    fn test_exit_code_timeout() {
        let err = TldrError::Timeout("5s".to_string());
        assert_eq!(err.exit_code(), 32, "Timeout should have exit code 32");
    }

    /// Contract 2.3: McpError has exit code 33
    #[test]
    fn test_exit_code_mcp_error() {
        let err = TldrError::McpError("protocol error".to_string());
        assert_eq!(err.exit_code(), 33, "McpError should have exit code 33");
    }

    /// Contract 2.3: SerializationError has exit code 40
    #[test]
    fn test_exit_code_serialization_error() {
        let err = TldrError::SerializationError("invalid json".to_string());
        assert_eq!(
            err.exit_code(),
            40,
            "SerializationError should have exit code 40"
        );
    }

    /// Contract 2.3: IoError has exit code 1
    #[test]
    fn test_exit_code_io_error() {
        let io_err = std::io::Error::other("io error");
        let err = TldrError::IoError(io_err);
        assert_eq!(err.exit_code(), 1, "IoError should have exit code 1");
    }
}

// =============================================================================
// Contract 2.3: Error Message Format
// =============================================================================

#[cfg(test)]
mod error_message_tests {
    use super::*;

    /// Contract 2.3: PathNotFound message format
    #[test]
    fn test_error_message_path_not_found() {
        let err = TldrError::PathNotFound(PathBuf::from("/missing/file.rs"));
        let msg = err.to_string();
        assert!(
            msg.contains("Path not found") && msg.contains("/missing/file.rs"),
            "Error message should contain 'Path not found: /missing/file.rs', got: {}",
            msg
        );
    }

    /// Contract 2.3: PathTraversal message format
    #[test]
    fn test_error_message_path_traversal() {
        let err = TldrError::PathTraversal(PathBuf::from("../escape.txt"));
        let msg = err.to_string();
        assert!(
            msg.contains("traversal") && msg.contains("../escape.txt"),
            "Error message should mention traversal and path, got: {}",
            msg
        );
    }

    /// Contract 2.3: FunctionNotFound with suggestions
    #[test]
    fn test_error_message_function_not_found_with_suggestions() {
        let err = TldrError::function_not_found_with_suggestions(
            "proces_data",
            Some(PathBuf::from("main.py")),
            vec!["process_data".to_string(), "process".to_string()],
        );
        let msg = err.to_string();

        assert!(
            msg.contains("proces_data"),
            "Should contain function name: {}",
            msg
        );
        assert!(
            msg.contains("Did you mean"),
            "Should contain suggestions: {}",
            msg
        );
        assert!(
            msg.contains("process_data"),
            "Should list suggested name: {}",
            msg
        );
    }

    /// Contract 2.3: ParseError with line number
    #[test]
    fn test_error_message_parse_error_with_line() {
        let err = TldrError::parse_error(PathBuf::from("broken.py"), Some(42), "unexpected token");
        let msg = err.to_string();

        assert!(
            msg.contains("broken.py"),
            "Should contain filename: {}",
            msg
        );
        assert!(msg.contains("42"), "Should contain line number: {}", msg);
        assert!(
            msg.contains("unexpected token"),
            "Should contain message: {}",
            msg
        );
    }

    /// Contract 2.3: UnsupportedLanguage message format
    #[test]
    fn test_error_message_unsupported_language() {
        let err = TldrError::UnsupportedLanguage("brainfuck".to_string());
        let msg = err.to_string();
        assert!(
            msg.contains("Unsupported language") && msg.contains("brainfuck"),
            "Error message should contain 'Unsupported language: brainfuck', got: {}",
            msg
        );
    }
}
