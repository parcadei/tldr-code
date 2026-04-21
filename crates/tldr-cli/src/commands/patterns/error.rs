//! Error types for Pattern Analysis commands.
//!
//! This module provides the `PatternsError` enum and `PatternsResult<T>` type alias
//! for all pattern analysis operations.

use std::path::PathBuf;
use thiserror::Error;

/// Errors specific to pattern analysis commands.
#[derive(Debug, Error)]
pub enum PatternsError {
    /// Source file not found.
    #[error("file not found: {}", path.display())]
    FileNotFound { path: PathBuf },

    /// Function not found in source file.
    #[error("function '{function}' not found in {}", file.display())]
    FunctionNotFound { function: String, file: PathBuf },

    /// Class not found in source file.
    #[error("class '{class_name}' not found in {}", file.display())]
    ClassNotFound { class_name: String, file: PathBuf },

    /// Parse error in source file.
    #[error("parse error in {}: {message}", file.display())]
    ParseError { file: PathBuf, message: String },

    /// File too large to analyze.
    #[error("file too large: {} ({bytes} bytes, max {max_bytes} bytes)", path.display())]
    FileTooLarge {
        path: PathBuf,
        bytes: u64,
        max_bytes: u64,
    },

    /// Directory scan limit exceeded.
    #[error("directory scan limit exceeded: {count} files found, max {max_files}")]
    TooManyFiles { count: u32, max_files: u32 },

    /// Analysis depth limit exceeded.
    #[error("analysis depth limit exceeded: depth {depth}, max {max_depth}")]
    DepthLimitExceeded { depth: u32, max_depth: u32 },

    /// Analysis timed out.
    #[error("analysis timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },

    /// Invalid parameter value.
    #[error("invalid parameter: {message}")]
    InvalidParameter { message: String },

    /// Path traversal attempt detected.
    #[error("path traversal blocked: {} attempts to escape project root", path.display())]
    PathTraversal { path: PathBuf },

    /// Path is not a directory.
    #[error("path is not a directory: {}", path.display())]
    NotADirectory { path: PathBuf },

    /// Unsupported language.
    #[error("unsupported language: {language}")]
    UnsupportedLanguage { language: String },

    /// No constraints found (not an error, but special exit code).
    #[error("no constraints found matching criteria")]
    NoConstraintsFound,

    /// Issues found (for resources command).
    #[error("resource issues found: {leaks} leaks, {double_closes} double-closes, {use_after_closes} use-after-close")]
    IssuesFound {
        leaks: u32,
        double_closes: u32,
        use_after_closes: u32,
    },

    /// Generic IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for pattern analysis commands.
pub type PatternsResult<T> = Result<T, PatternsError>;

impl PatternsError {
    /// Create a FileNotFound error.
    pub fn file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Create a FunctionNotFound error.
    pub fn function_not_found(function: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Self::FunctionNotFound {
            function: function.into(),
            file: file.into(),
        }
    }

    /// Create a ClassNotFound error.
    pub fn class_not_found(class_name: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Self::ClassNotFound {
            class_name: class_name.into(),
            file: file.into(),
        }
    }

    /// Create a ParseError.
    pub fn parse_error(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::ParseError {
            file: file.into(),
            message: message.into(),
        }
    }

    /// Create an InvalidParameter error.
    pub fn invalid_parameter(message: impl Into<String>) -> Self {
        Self::InvalidParameter {
            message: message.into(),
        }
    }

    /// Create a PathTraversal error.
    pub fn path_traversal(path: impl Into<PathBuf>) -> Self {
        Self::PathTraversal { path: path.into() }
    }

    /// Create a FileTooLarge error.
    pub fn file_too_large(path: impl Into<PathBuf>, bytes: u64, max_bytes: u64) -> Self {
        Self::FileTooLarge {
            path: path.into(),
            bytes,
            max_bytes,
        }
    }

    /// Create a DepthLimitExceeded error.
    pub fn depth_exceeded(depth: u32, max_depth: u32) -> Self {
        Self::DepthLimitExceeded { depth, max_depth }
    }

    /// Create a TooManyFiles error.
    pub fn too_many_files(count: u32, max_files: u32) -> Self {
        Self::TooManyFiles { count, max_files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_file_not_found_message() {
        let err = PatternsError::file_not_found("/path/to/file.py");
        let msg = err.to_string();
        assert!(msg.contains("file not found"));
        assert!(msg.contains("file.py"));
    }

    #[test]
    fn test_error_function_not_found_message() {
        let err = PatternsError::function_not_found("my_func", "/path/to/file.py");
        let msg = err.to_string();
        assert!(msg.contains("my_func"));
        assert!(msg.contains("not found"));
    }

    #[test]
    fn test_error_path_traversal_message() {
        let err = PatternsError::path_traversal("../etc/passwd");
        let msg = err.to_string();
        assert!(msg.contains("path traversal"));
        assert!(msg.contains("etc/passwd"));
    }

    #[test]
    fn test_error_file_too_large_message() {
        let err = PatternsError::file_too_large("/big/file.py", 20_000_000, 10_000_000);
        let msg = err.to_string();
        assert!(msg.contains("too large"));
        assert!(msg.contains("20000000"));
    }

    #[test]
    fn test_error_depth_exceeded_message() {
        let err = PatternsError::depth_exceeded(150, 100);
        let msg = err.to_string();
        assert!(msg.contains("depth"));
        assert!(msg.contains("150"));
        assert!(msg.contains("100"));
    }

    #[test]
    fn test_error_too_many_files_message() {
        let err = PatternsError::too_many_files(1500, 1000);
        let msg = err.to_string();
        assert!(msg.contains("1500"));
        assert!(msg.contains("1000"));
    }

    #[test]
    fn test_error_parse_error_message() {
        let err = PatternsError::parse_error("/file.py", "unexpected token");
        let msg = err.to_string();
        assert!(msg.contains("parse error"));
        assert!(msg.contains("unexpected token"));
    }

    #[test]
    fn test_error_invalid_parameter_message() {
        let err = PatternsError::invalid_parameter("value must be positive");
        let msg = err.to_string();
        assert!(msg.contains("invalid parameter"));
        assert!(msg.contains("positive"));
    }
}
