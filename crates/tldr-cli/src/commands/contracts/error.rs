//! Error types for Contracts & Flow commands
//!
//! Provides specific error types for all failure modes in the contracts
//! and flow analysis commands. Errors include actionable information like
//! file paths and line numbers.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors specific to contracts and flow analysis commands.
///
/// Each variant includes contextual information to help users understand
/// and fix the issue.
#[derive(Debug, Error)]
pub enum ContractsError {
    /// Source file not found.
    #[error("file not found: {}", path.display())]
    FileNotFound {
        /// Path that was not found
        path: PathBuf,
    },

    /// Function not found in source file.
    #[error("function '{function}' not found in {}", file.display())]
    FunctionNotFound {
        /// Function name that was searched for
        function: String,
        /// File that was searched
        file: PathBuf,
    },

    /// Test path not found.
    #[error("test path not found: {}", path.display())]
    TestPathNotFound {
        /// Path that was not found
        path: PathBuf,
    },

    /// Line number is outside function range.
    #[error("line {line} is outside function '{function}' (lines {start}-{end})")]
    LineOutsideFunction {
        /// Line number that was requested
        line: u32,
        /// Function name
        function: String,
        /// Start line of function
        start: u32,
        /// End line of function
        end: u32,
    },

    /// Parse error in source file.
    #[error("parse error in {}: {message}", file.display())]
    ParseError {
        /// File that failed to parse
        file: PathBuf,
        /// Parser error message
        message: String,
    },

    /// SSA construction failed.
    #[error("SSA construction failed: {0}")]
    SsaError(String),

    /// Analysis did not converge within iteration limit.
    #[error("analysis did not converge after {iterations} iterations")]
    DidNotConverge {
        /// Number of iterations attempted
        iterations: u32,
    },

    /// Sub-analysis failed in verify command.
    #[error("sub-analysis '{name}' failed: {message}")]
    SubAnalysisFailed {
        /// Name of the sub-analysis that failed
        name: String,
        /// Error message from the sub-analysis
        message: String,
    },

    /// No test directory found in project.
    #[error("no test directory found in {}", project.display())]
    NoTestDirectory {
        /// Project directory that was searched
        project: PathBuf,
    },

    /// Operation timed out.
    #[error("operation timed out after {timeout_secs}s")]
    Timeout {
        /// Timeout duration in seconds
        timeout_secs: u64,
    },

    /// File too large to analyze.
    #[error("file too large: {} ({bytes} bytes, max {max_bytes} bytes)", path.display())]
    FileTooLarge {
        /// Path to the file
        path: PathBuf,
        /// Actual file size
        bytes: u64,
        /// Maximum allowed size
        max_bytes: u64,
    },

    /// AST too deeply nested.
    #[error("AST too deeply nested in {}: depth {depth} exceeds limit {max_depth}", file.display())]
    AstTooDeep {
        /// File with deeply nested AST
        file: PathBuf,
        /// Actual depth
        depth: u32,
        /// Maximum allowed depth
        max_depth: u32,
    },

    /// SSA graph has too many nodes.
    #[error("SSA graph too large: {nodes} nodes exceeds limit {max_nodes}")]
    SsaTooLarge {
        /// Actual number of nodes
        nodes: u32,
        /// Maximum allowed nodes
        max_nodes: u32,
    },

    /// Slice computation exceeded depth limit.
    #[error("slice computation exceeded depth limit of {max_depth}")]
    SliceDepthExceeded {
        /// Maximum allowed depth
        max_depth: u32,
    },

    /// Invalid function name.
    #[error("invalid function name: {reason}")]
    InvalidFunctionName {
        /// Why the name is invalid
        reason: String,
    },

    /// Path traversal attempt detected.
    #[error("path traversal blocked: {} attempts to escape project root", path.display())]
    PathTraversal {
        /// Suspicious path
        path: PathBuf,
    },

    /// Generic IO error.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// JSON serialization/deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for contracts commands.
pub type ContractsResult<T> = Result<T, ContractsError>;

// =============================================================================
// Error Construction Helpers
// =============================================================================

impl ContractsError {
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

    /// Create a ParseError.
    pub fn parse_error(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::ParseError {
            file: file.into(),
            message: message.into(),
        }
    }

    /// Create an SsaError.
    pub fn ssa_error(message: impl Into<String>) -> Self {
        Self::SsaError(message.into())
    }

    /// Create a LineOutsideFunction error.
    pub fn line_outside_function(
        line: u32,
        function: impl Into<String>,
        start: u32,
        end: u32,
    ) -> Self {
        Self::LineOutsideFunction {
            line,
            function: function.into(),
            start,
            end,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_file_not_found() {
        let err = ContractsError::file_not_found("/path/to/file.py");
        let msg = err.to_string();
        assert!(msg.contains("file not found"));
        assert!(msg.contains("file.py"));
    }

    #[test]
    fn test_error_function_not_found() {
        let err = ContractsError::function_not_found("my_func", "/path/to/file.py");
        let msg = err.to_string();
        assert!(msg.contains("my_func"));
        assert!(msg.contains("not found"));
        assert!(msg.contains("file.py"));
    }

    #[test]
    fn test_error_parse_error() {
        let err = ContractsError::parse_error("/path/to/file.py", "unexpected token");
        let msg = err.to_string();
        assert!(msg.contains("parse error"));
        assert!(msg.contains("unexpected token"));
    }

    #[test]
    fn test_error_ssa_error() {
        let err = ContractsError::ssa_error("failed to compute dominators");
        let msg = err.to_string();
        assert!(msg.contains("SSA construction failed"));
        assert!(msg.contains("dominators"));
    }

    #[test]
    fn test_error_line_outside_function() {
        let err = ContractsError::line_outside_function(100, "my_func", 10, 50);
        let msg = err.to_string();
        assert!(msg.contains("line 100"));
        assert!(msg.contains("my_func"));
        assert!(msg.contains("10-50"));
    }

    #[test]
    fn test_error_test_path_not_found() {
        let err = ContractsError::TestPathNotFound {
            path: PathBuf::from("/path/to/tests"),
        };
        let msg = err.to_string();
        assert!(msg.contains("test path not found"));
    }

    #[test]
    fn test_error_did_not_converge() {
        let err = ContractsError::DidNotConverge { iterations: 50 };
        let msg = err.to_string();
        assert!(msg.contains("did not converge"));
        assert!(msg.contains("50"));
    }

    #[test]
    fn test_error_timeout() {
        let err = ContractsError::Timeout { timeout_secs: 60 };
        let msg = err.to_string();
        assert!(msg.contains("timed out"));
        assert!(msg.contains("60s"));
    }

    #[test]
    fn test_error_file_too_large() {
        let err = ContractsError::FileTooLarge {
            path: PathBuf::from("/path/to/large.py"),
            bytes: 15_000_000,
            max_bytes: 10_000_000,
        };
        let msg = err.to_string();
        assert!(msg.contains("file too large"));
        assert!(msg.contains("large.py"));
    }

    #[test]
    fn test_error_path_traversal() {
        let err = ContractsError::PathTraversal {
            path: PathBuf::from("../../etc/passwd"),
        };
        let msg = err.to_string();
        assert!(msg.contains("path traversal blocked"));
    }

    #[test]
    fn test_error_sub_analysis_failed() {
        let err = ContractsError::SubAnalysisFailed {
            name: "contracts".to_string(),
            message: "parse error".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("sub-analysis"));
        assert!(msg.contains("contracts"));
    }

    #[test]
    fn test_error_no_test_directory() {
        let err = ContractsError::NoTestDirectory {
            project: PathBuf::from("/path/to/project"),
        };
        let msg = err.to_string();
        assert!(msg.contains("no test directory"));
    }

    #[test]
    fn test_error_io_from() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let contracts_err: ContractsError = io_err.into();
        assert!(matches!(contracts_err, ContractsError::Io(_)));
    }

    #[test]
    fn test_error_json_from() {
        let json_str = "{ invalid json }";
        let json_result: Result<serde_json::Value, _> = serde_json::from_str(json_str);
        let json_err = json_result.unwrap_err();
        let contracts_err: ContractsError = json_err.into();
        assert!(matches!(contracts_err, ContractsError::Json(_)));
    }

    #[test]
    fn test_result_type_alias() {
        fn example_fn() -> ContractsResult<i32> {
            Ok(42)
        }

        fn example_err() -> ContractsResult<i32> {
            Err(ContractsError::file_not_found("/test.py"))
        }

        assert_eq!(example_fn().unwrap(), 42);
        assert!(example_err().is_err());
    }
}
