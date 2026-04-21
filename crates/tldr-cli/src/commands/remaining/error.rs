//! Error types for remaining commands
//!
//! This module defines the error types used across all remaining analysis
//! commands (todo, explain, secure, definition, diff, diff_impact, api_check,
//! equivalence, vuln).

use std::path::PathBuf;
use thiserror::Error;

/// Errors for remaining commands.
#[derive(Debug, Error)]
pub enum RemainingError {
    /// File not found.
    #[error("file not found: {}", path.display())]
    FileNotFound { path: PathBuf },

    /// Function/symbol not found.
    #[error("symbol '{}' not found in {}", symbol, file.display())]
    SymbolNotFound { symbol: String, file: PathBuf },

    /// Parse error.
    #[error("parse error in {}: {message}", file.display())]
    ParseError { file: PathBuf, message: String },

    /// Invalid arguments.
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },

    /// File too large.
    #[error("file too large: {} ({bytes} bytes)", path.display())]
    FileTooLarge { path: PathBuf, bytes: u64 },

    /// Path traversal blocked.
    #[error("path traversal blocked: {}", path.display())]
    PathTraversal { path: PathBuf },

    /// Unsupported language.
    #[error("unsupported language: {language}")]
    UnsupportedLanguage { language: String },

    /// Analysis error.
    #[error("analysis error: {message}")]
    AnalysisError { message: String },

    /// Findings detected (for vuln/api-check - special exit code).
    #[error("{count} findings detected")]
    FindingsDetected { count: u32 },

    /// Timeout.
    #[error("analysis timed out after {seconds}s")]
    Timeout { seconds: u64 },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl RemainingError {
    /// Create a FileNotFound error
    pub fn file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::FileNotFound { path: path.into() }
    }

    /// Create a SymbolNotFound error
    pub fn symbol_not_found(symbol: impl Into<String>, file: impl Into<PathBuf>) -> Self {
        Self::SymbolNotFound {
            symbol: symbol.into(),
            file: file.into(),
        }
    }

    /// Create a ParseError
    pub fn parse_error(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::ParseError {
            file: file.into(),
            message: message.into(),
        }
    }

    /// Create an InvalidArgument error
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            message: message.into(),
        }
    }

    /// Create a FileTooLarge error
    pub fn file_too_large(path: impl Into<PathBuf>, bytes: u64) -> Self {
        Self::FileTooLarge {
            path: path.into(),
            bytes,
        }
    }

    /// Create a PathTraversal error
    pub fn path_traversal(path: impl Into<PathBuf>) -> Self {
        Self::PathTraversal { path: path.into() }
    }

    /// Create an UnsupportedLanguage error
    pub fn unsupported_language(language: impl Into<String>) -> Self {
        Self::UnsupportedLanguage {
            language: language.into(),
        }
    }

    /// Create an AnalysisError
    pub fn analysis_error(message: impl Into<String>) -> Self {
        Self::AnalysisError {
            message: message.into(),
        }
    }

    /// Create a FindingsDetected error
    pub fn findings_detected(count: u32) -> Self {
        Self::FindingsDetected { count }
    }

    /// Create a Timeout error
    pub fn timeout(seconds: u64) -> Self {
        Self::Timeout { seconds }
    }

    /// Get the appropriate exit code for this error
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::FindingsDetected { .. } => 2, // Special exit code for findings
            _ => 1,                             // General error
        }
    }
}

/// Result type alias for remaining commands
pub type RemainingResult<T> = Result<T, RemainingError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_not_found_error() {
        let err = RemainingError::file_not_found("/path/to/file.py");
        assert!(err.to_string().contains("file not found"));
        assert!(err.to_string().contains("file.py"));
    }

    #[test]
    fn test_symbol_not_found_error() {
        let err = RemainingError::symbol_not_found("my_function", "/path/to/file.py");
        assert!(err.to_string().contains("my_function"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_exit_codes() {
        assert_eq!(RemainingError::file_not_found("/foo").exit_code(), 1);
        assert_eq!(RemainingError::findings_detected(5).exit_code(), 2);
    }
}
