//! Test coverage for tldr-core error module
//!
//! Tests all public error types and methods from:
//! - crates/tldr-core/src/error.rs

use std::path::PathBuf;

use tldr_core::TldrError;

// =============================================================================
// Error Display Tests
// =============================================================================

#[test]
fn test_error_display_path_not_found() {
    let err = TldrError::PathNotFound(PathBuf::from("/some/path"));
    assert_eq!(err.to_string(), "Path not found: /some/path");
}

#[test]
fn test_error_display_path_traversal() {
    let err = TldrError::PathTraversal(PathBuf::from("../escape"));
    assert_eq!(err.to_string(), "Path traversal detected: ../escape");
}

#[test]
fn test_error_display_symlink_cycle() {
    let err = TldrError::SymlinkCycle(PathBuf::from("/path/to/link"));
    assert_eq!(err.to_string(), "Symlink cycle detected: /path/to/link");
}

#[test]
fn test_error_display_permission_denied() {
    let err = TldrError::PermissionDenied(PathBuf::from("/root/secret"));
    assert_eq!(err.to_string(), "Permission denied: /root/secret");
}

#[test]
fn test_error_display_file_too_large() {
    let err = TldrError::FileTooLarge {
        path: PathBuf::from("/huge/file.bin"),
        size_mb: 100,
        max_mb: 50,
    };
    let msg = err.to_string();
    assert!(msg.contains("File too large"));
    assert!(msg.contains("100MB"));
    assert!(msg.contains("50MB"));
}

#[test]
fn test_error_display_encoding_error() {
    let err = TldrError::EncodingError {
        path: PathBuf::from("/file.txt"),
        detail: "Invalid UTF-8 sequence".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Encoding error"));
    assert!(msg.contains("Invalid UTF-8 sequence"));
}

#[test]
fn test_error_display_coverage_parse_error() {
    let err = TldrError::CoverageParseError {
        format: "lcov".to_string(),
        detail: "Missing SF line".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Coverage parse error"));
    assert!(msg.contains("lcov"));
    assert!(msg.contains("Missing SF line"));
}

#[test]
fn test_error_display_not_git_repository() {
    let err = TldrError::NotGitRepository(PathBuf::from("/not/a/repo"));
    assert_eq!(err.to_string(), "Not a git repository: /not/a/repo");
}

#[test]
fn test_error_display_git_operation_in_progress() {
    let err = TldrError::GitOperationInProgress("rebase".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Git operation in progress"));
    assert!(msg.contains("rebase"));
}

#[test]
fn test_error_display_parse_error_with_line() {
    let err = TldrError::ParseError {
        file: PathBuf::from("test.py"),
        line: Some(42),
        message: "unexpected token".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Parse error in test.py at line 42"));
    assert!(msg.contains("unexpected token"));
}

#[test]
fn test_error_display_parse_error_without_line() {
    let err = TldrError::ParseError {
        file: PathBuf::from("test.py"),
        line: None,
        message: "file is binary".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Parse error in test.py:"));
    assert!(!msg.contains("at line"));
}

#[test]
fn test_error_display_unsupported_language() {
    let err = TldrError::UnsupportedLanguage("xyz".to_string());
    assert_eq!(err.to_string(), "Unsupported language: xyz");
}

#[test]
fn test_error_display_function_not_found_simple() {
    let err = TldrError::FunctionNotFound {
        name: "process_data".to_string(),
        file: None,
        suggestions: vec![],
    };
    assert_eq!(err.to_string(), "Function not found: process_data");
}

#[test]
fn test_error_display_function_not_found_with_file() {
    let err = TldrError::FunctionNotFound {
        name: "process_data".to_string(),
        file: Some(PathBuf::from("src/main.py")),
        suggestions: vec![],
    };
    let msg = err.to_string();
    assert!(msg.contains("process_data"));
    assert!(msg.contains("src/main.py"));
}

#[test]
fn test_error_display_function_not_found_with_suggestions() {
    let err = TldrError::FunctionNotFound {
        name: "proces_data".to_string(),
        file: None,
        suggestions: vec!["process_data".to_string(), "process_datum".to_string()],
    };
    let msg = err.to_string();
    assert!(msg.contains("proces_data"));
    assert!(msg.contains("Did you mean:"));
    assert!(msg.contains("process_data"));
    assert!(msg.contains("process_datum"));
}

#[test]
fn test_error_display_function_not_found_with_file_and_suggestions() {
    let err = TldrError::FunctionNotFound {
        name: "proces_data".to_string(),
        file: Some(PathBuf::from("src/main.py")),
        suggestions: vec!["process_data".to_string()],
    };
    let msg = err.to_string();
    assert!(msg.contains("proces_data"));
    assert!(msg.contains("src/main.py"));
    assert!(msg.contains("Did you mean:"));
}

#[test]
fn test_error_display_invalid_direction() {
    let err = TldrError::InvalidDirection("up".to_string());
    let msg = err.to_string();
    assert!(msg.contains("Invalid direction"));
    assert!(msg.contains("up"));
    assert!(msg.contains("backward"));
    assert!(msg.contains("forward"));
}

#[test]
fn test_error_display_line_not_in_function() {
    let err = TldrError::LineNotInFunction(100);
    assert_eq!(
        err.to_string(),
        "Line 100 is not within the specified function"
    );
}

#[test]
fn test_error_display_no_supported_files() {
    let err = TldrError::NoSupportedFiles(PathBuf::from("/empty/dir"));
    assert_eq!(err.to_string(), "No supported files found in /empty/dir");
}

#[test]
fn test_error_display_not_found_without_suggestion() {
    let err = TldrError::NotFound {
        entity: "Class".to_string(),
        name: "MyClass".to_string(),
        suggestion: None,
    };
    let msg = err.to_string();
    assert!(msg.contains("Class not found: MyClass"));
}

#[test]
fn test_error_display_not_found_with_suggestion() {
    let err = TldrError::NotFound {
        entity: "Function".to_string(),
        name: "proces".to_string(),
        suggestion: Some("Did you mean 'process'?".to_string()),
    };
    let msg = err.to_string();
    assert!(msg.contains("Function not found: proces"));
    assert!(msg.contains("Did you mean 'process'?"));
}

#[test]
fn test_error_display_invalid_args_without_suggestion() {
    let err = TldrError::InvalidArgs {
        arg: "depth".to_string(),
        message: "must be positive".to_string(),
        suggestion: None,
    };
    let msg = err.to_string();
    assert!(msg.contains("Invalid argument depth"));
    assert!(msg.contains("must be positive"));
}

#[test]
fn test_error_display_invalid_args_with_suggestion() {
    let err = TldrError::InvalidArgs {
        arg: "format".to_string(),
        message: "unknown format".to_string(),
        suggestion: Some("Try 'json' or 'text'".to_string()),
    };
    let msg = err.to_string();
    assert!(msg.contains("Invalid argument format"));
    assert!(msg.contains("unknown format"));
    assert!(msg.contains("Hint: Try 'json' or 'text'"));
}

#[test]
fn test_error_display_daemon_error() {
    let err = TldrError::DaemonError("Connection refused".to_string());
    assert_eq!(err.to_string(), "Daemon error: Connection refused");
}

#[test]
fn test_error_display_connection_failed() {
    let err = TldrError::ConnectionFailed("timeout".to_string());
    assert_eq!(err.to_string(), "Connection failed: timeout");
}

#[test]
fn test_error_display_timeout() {
    let err = TldrError::Timeout("Analysis took too long".to_string());
    assert_eq!(err.to_string(), "Timeout: Analysis took too long");
}

#[test]
fn test_error_display_mcp_error() {
    let err = TldrError::McpError("Invalid request".to_string());
    assert_eq!(err.to_string(), "MCP error: Invalid request");
}

#[test]
fn test_error_display_serialization_error() {
    let err = TldrError::SerializationError("Invalid JSON".to_string());
    assert_eq!(err.to_string(), "Serialization error: Invalid JSON");
}

#[test]
fn test_error_display_embedding_error() {
    let err = TldrError::Embedding("Model not loaded".to_string());
    assert_eq!(err.to_string(), "Embedding error: Model not loaded");
}

#[test]
fn test_error_display_model_load_error() {
    let err = TldrError::ModelLoadError {
        model: "snowflake-arctic".to_string(),
        detail: "File not found".to_string(),
    };
    let msg = err.to_string();
    assert!(msg.contains("Failed to load embedding model 'snowflake-arctic'"));
    assert!(msg.contains("File not found"));
}

#[test]
fn test_error_display_index_too_large() {
    let err = TldrError::IndexTooLarge {
        count: 100000,
        max: 50000,
    };
    let msg = err.to_string();
    assert!(msg.contains("Index too large"));
    assert!(msg.contains("100000"));
    assert!(msg.contains("50000"));
}

#[test]
fn test_error_display_memory_limit_exceeded() {
    let err = TldrError::MemoryLimitExceeded {
        estimated_mb: 2048,
        max_mb: 1024,
    };
    let msg = err.to_string();
    assert!(msg.contains("Memory limit exceeded"));
    assert!(msg.contains("2048MB"));
    assert!(msg.contains("1024MB"));
}

#[test]
fn test_error_display_chunk_not_found_without_function() {
    let err = TldrError::ChunkNotFound {
        file: "main.py".to_string(),
        function: None,
    };
    assert_eq!(err.to_string(), "Chunk not found: main.py");
}

#[test]
fn test_error_display_chunk_not_found_with_function() {
    let err = TldrError::ChunkNotFound {
        file: "main.py".to_string(),
        function: Some("process_data".to_string()),
    };
    assert_eq!(err.to_string(), "Chunk not found: main.py::process_data");
}

#[test]
fn test_error_display_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err = TldrError::IoError(io_err);
    assert!(err.to_string().contains("file not found"));
}

// =============================================================================
// Helper Method Tests
// =============================================================================

#[test]
fn test_function_not_found_helper() {
    let err = TldrError::function_not_found("my_function");

    match err {
        TldrError::FunctionNotFound {
            name,
            file,
            suggestions,
        } => {
            assert_eq!(name, "my_function");
            assert!(file.is_none());
            assert!(suggestions.is_empty());
        }
        _ => panic!("Expected FunctionNotFound error"),
    }
}

#[test]
fn test_function_not_found_in_file_helper() {
    let err = TldrError::function_not_found_in_file("my_function", PathBuf::from("/test.py"));

    match err {
        TldrError::FunctionNotFound {
            name,
            file,
            suggestions,
        } => {
            assert_eq!(name, "my_function");
            assert_eq!(file, Some(PathBuf::from("/test.py")));
            assert!(suggestions.is_empty());
        }
        _ => panic!("Expected FunctionNotFound error"),
    }
}

#[test]
fn test_function_not_found_with_suggestions_helper() {
    let err = TldrError::function_not_found_with_suggestions(
        "my_func",
        Some(PathBuf::from("/test.py")),
        vec!["my_function".to_string(), "my_funcs".to_string()],
    );

    match err {
        TldrError::FunctionNotFound {
            name,
            file,
            suggestions,
        } => {
            assert_eq!(name, "my_func");
            assert_eq!(file, Some(PathBuf::from("/test.py")));
            assert_eq!(suggestions.len(), 2);
            assert!(suggestions.contains(&"my_function".to_string()));
        }
        _ => panic!("Expected FunctionNotFound error"),
    }
}

#[test]
fn test_parse_error_helper() {
    let err = TldrError::parse_error(PathBuf::from("/test.py"), Some(42), "unexpected indent");

    match err {
        TldrError::ParseError {
            file,
            line,
            message,
        } => {
            assert_eq!(file, PathBuf::from("/test.py"));
            assert_eq!(line, Some(42));
            assert_eq!(message, "unexpected indent");
        }
        _ => panic!("Expected ParseError"),
    }
}

#[test]
fn test_parse_error_helper_no_line() {
    let err = TldrError::parse_error(PathBuf::from("/test.py"), None, "file is empty");

    match err {
        TldrError::ParseError {
            file,
            line,
            message,
        } => {
            assert_eq!(file, PathBuf::from("/test.py"));
            assert_eq!(line, None);
            assert_eq!(message, "file is empty");
        }
        _ => panic!("Expected ParseError"),
    }
}

// =============================================================================
// is_recoverable Tests
// =============================================================================

#[test]
fn test_is_recoverable_true_cases() {
    // ParseError is recoverable
    assert!(TldrError::parse_error(PathBuf::from("x"), None, "e").is_recoverable());

    // PermissionDenied is recoverable
    assert!(TldrError::PermissionDenied(PathBuf::from("/")).is_recoverable());

    // FunctionNotFound is recoverable
    assert!(TldrError::function_not_found("foo").is_recoverable());
}

#[test]
fn test_is_recoverable_false_cases() {
    // PathNotFound is not recoverable
    assert!(!TldrError::PathNotFound(PathBuf::from("/")).is_recoverable());

    // PathTraversal is not recoverable
    assert!(!TldrError::PathTraversal(PathBuf::from("../")).is_recoverable());

    // SymlinkCycle is not recoverable
    assert!(!TldrError::SymlinkCycle(PathBuf::from("/link")).is_recoverable());

    // DaemonError is not recoverable
    assert!(!TldrError::DaemonError("test".to_string()).is_recoverable());

    // Timeout is not recoverable
    assert!(!TldrError::Timeout("test".to_string()).is_recoverable());

    // InvalidDirection is not recoverable
    assert!(!TldrError::InvalidDirection("up".to_string()).is_recoverable());
}

// =============================================================================
// exit_code Tests
// =============================================================================

#[test]
fn test_exit_code_file_system_errors() {
    assert_eq!(TldrError::PathNotFound(PathBuf::from("/")).exit_code(), 2);
    assert_eq!(TldrError::PathTraversal(PathBuf::from("/")).exit_code(), 3);
    assert_eq!(TldrError::SymlinkCycle(PathBuf::from("/")).exit_code(), 4);
    assert_eq!(
        TldrError::PermissionDenied(PathBuf::from("/")).exit_code(),
        5
    );
    assert_eq!(
        TldrError::FileTooLarge {
            path: PathBuf::from("/"),
            size_mb: 100,
            max_mb: 50
        }
        .exit_code(),
        6
    );
    assert_eq!(
        TldrError::EncodingError {
            path: PathBuf::from("/"),
            detail: "err".to_string()
        }
        .exit_code(),
        7
    );
    assert_eq!(
        TldrError::NotGitRepository(PathBuf::from("/")).exit_code(),
        8
    );
    assert_eq!(
        TldrError::GitOperationInProgress("rebase".to_string()).exit_code(),
        9
    );
}

#[test]
fn test_exit_code_parse_errors() {
    assert_eq!(
        TldrError::parse_error(PathBuf::from("/"), None, "err").exit_code(),
        10
    );
    assert_eq!(
        TldrError::UnsupportedLanguage("xyz".to_string()).exit_code(),
        11
    );
    assert_eq!(
        TldrError::CoverageParseError {
            format: "lcov".to_string(),
            detail: "err".to_string()
        }
        .exit_code(),
        12
    );
}

#[test]
fn test_exit_code_analysis_errors() {
    assert_eq!(TldrError::function_not_found("foo").exit_code(), 20);
    assert_eq!(
        TldrError::InvalidDirection("up".to_string()).exit_code(),
        21
    );
    assert_eq!(TldrError::LineNotInFunction(42).exit_code(), 22);
    assert_eq!(
        TldrError::NoSupportedFiles(PathBuf::from("/")).exit_code(),
        23
    );
    assert_eq!(
        TldrError::NotFound {
            entity: "Class".to_string(),
            name: "X".to_string(),
            suggestion: None
        }
        .exit_code(),
        24
    );
    assert_eq!(
        TldrError::InvalidArgs {
            arg: "x".to_string(),
            message: "err".to_string(),
            suggestion: None
        }
        .exit_code(),
        25
    );
}

#[test]
fn test_exit_code_daemon_errors() {
    assert_eq!(TldrError::DaemonError("err".to_string()).exit_code(), 30);
    assert_eq!(
        TldrError::ConnectionFailed("err".to_string()).exit_code(),
        31
    );
    assert_eq!(TldrError::Timeout("err".to_string()).exit_code(), 32);
    assert_eq!(TldrError::McpError("err".to_string()).exit_code(), 33);
}

#[test]
fn test_exit_code_serialization_errors() {
    assert_eq!(
        TldrError::SerializationError("err".to_string()).exit_code(),
        40
    );
}

#[test]
fn test_exit_code_semantic_search_errors() {
    assert_eq!(TldrError::Embedding("err".to_string()).exit_code(), 50);
    assert_eq!(
        TldrError::ModelLoadError {
            model: "x".to_string(),
            detail: "err".to_string()
        }
        .exit_code(),
        51
    );
    assert_eq!(
        TldrError::IndexTooLarge {
            count: 100,
            max: 50
        }
        .exit_code(),
        52
    );
    assert_eq!(
        TldrError::MemoryLimitExceeded {
            estimated_mb: 100,
            max_mb: 50
        }
        .exit_code(),
        53
    );
    assert_eq!(
        TldrError::ChunkNotFound {
            file: "x".to_string(),
            function: None
        }
        .exit_code(),
        54
    );
}

#[test]
fn test_exit_code_io_error() {
    let io_err = std::io::Error::other("test");
    assert_eq!(TldrError::IoError(io_err).exit_code(), 1);
}

// =============================================================================
// category Tests
// =============================================================================

#[test]
fn test_category_filesystem() {
    assert_eq!(
        TldrError::PathNotFound(PathBuf::from("/")).category(),
        "filesystem"
    );
    assert_eq!(
        TldrError::PathTraversal(PathBuf::from("/")).category(),
        "filesystem"
    );
    assert_eq!(
        TldrError::SymlinkCycle(PathBuf::from("/")).category(),
        "filesystem"
    );
    assert_eq!(
        TldrError::PermissionDenied(PathBuf::from("/")).category(),
        "filesystem"
    );
    assert_eq!(
        TldrError::FileTooLarge {
            path: PathBuf::from("/"),
            size_mb: 100,
            max_mb: 50
        }
        .category(),
        "filesystem"
    );
    assert_eq!(
        TldrError::EncodingError {
            path: PathBuf::from("/"),
            detail: "err".to_string()
        }
        .category(),
        "filesystem"
    );
}

#[test]
fn test_category_git() {
    assert_eq!(
        TldrError::NotGitRepository(PathBuf::from("/")).category(),
        "git"
    );
    assert_eq!(
        TldrError::GitOperationInProgress("rebase".to_string()).category(),
        "git"
    );
}

#[test]
fn test_category_parse() {
    assert_eq!(
        TldrError::parse_error(PathBuf::from("/"), None, "err").category(),
        "parse"
    );
    assert_eq!(
        TldrError::UnsupportedLanguage("xyz".to_string()).category(),
        "parse"
    );
    assert_eq!(
        TldrError::CoverageParseError {
            format: "lcov".to_string(),
            detail: "err".to_string()
        }
        .category(),
        "parse"
    );
}

#[test]
fn test_category_analysis() {
    assert_eq!(TldrError::function_not_found("foo").category(), "analysis");
    assert_eq!(
        TldrError::InvalidDirection("up".to_string()).category(),
        "analysis"
    );
    assert_eq!(TldrError::LineNotInFunction(42).category(), "analysis");
    assert_eq!(
        TldrError::NoSupportedFiles(PathBuf::from("/")).category(),
        "analysis"
    );
    assert_eq!(
        TldrError::NotFound {
            entity: "X".to_string(),
            name: "Y".to_string(),
            suggestion: None
        }
        .category(),
        "analysis"
    );
    assert_eq!(
        TldrError::InvalidArgs {
            arg: "x".to_string(),
            message: "err".to_string(),
            suggestion: None
        }
        .category(),
        "analysis"
    );
}

#[test]
fn test_category_daemon() {
    assert_eq!(
        TldrError::DaemonError("err".to_string()).category(),
        "daemon"
    );
    assert_eq!(
        TldrError::ConnectionFailed("err".to_string()).category(),
        "daemon"
    );
    assert_eq!(TldrError::Timeout("err".to_string()).category(), "daemon");
}

#[test]
fn test_category_mcp() {
    assert_eq!(TldrError::McpError("err".to_string()).category(), "mcp");
}

#[test]
fn test_category_serialization() {
    assert_eq!(
        TldrError::SerializationError("err".to_string()).category(),
        "serialization"
    );
}

#[test]
fn test_category_semantic() {
    assert_eq!(
        TldrError::Embedding("err".to_string()).category(),
        "semantic"
    );
    assert_eq!(
        TldrError::ModelLoadError {
            model: "x".to_string(),
            detail: "err".to_string()
        }
        .category(),
        "semantic"
    );
    assert_eq!(
        TldrError::IndexTooLarge {
            count: 100,
            max: 50
        }
        .category(),
        "semantic"
    );
    assert_eq!(
        TldrError::MemoryLimitExceeded {
            estimated_mb: 100,
            max_mb: 50
        }
        .category(),
        "semantic"
    );
    assert_eq!(
        TldrError::ChunkNotFound {
            file: "x".to_string(),
            function: None
        }
        .category(),
        "semantic"
    );
}

#[test]
fn test_category_io() {
    let io_err = std::io::Error::other("test");
    assert_eq!(TldrError::IoError(io_err).category(), "io");
}

// =============================================================================
// From Trait Tests
// =============================================================================

#[test]
fn test_from_serde_json_error() {
    let json_err = serde_json::from_str::<serde_json::Value>("invalid json").unwrap_err();
    let tldr_err: TldrError = json_err.into();

    match tldr_err {
        TldrError::SerializationError(msg) => {
            assert!(msg.contains("expected"));
        }
        _ => panic!("Expected SerializationError"),
    }
}

#[test]
fn test_from_regex_error() {
    let pattern = String::from("[invalid");
    let regex_err = regex::Regex::new(&pattern).unwrap_err();
    let tldr_err: TldrError = regex_err.into();

    match tldr_err {
        TldrError::ParseError {
            file,
            line,
            message,
        } => {
            assert_eq!(file, PathBuf::new());
            assert_eq!(line, None);
            assert!(message.contains("Invalid regex pattern"));
        }
        _ => panic!("Expected ParseError, got {:?}", tldr_err),
    }
}

#[test]
fn test_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let tldr_err: TldrError = io_err.into();

    match tldr_err {
        TldrError::IoError(e) => {
            assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
        }
        _ => panic!("Expected IoError"),
    }
}

// =============================================================================
// Error Trait Tests
// =============================================================================

#[test]
fn test_error_trait_source() {
    // Test that we can use the Error trait
    let err = TldrError::PathNotFound(PathBuf::from("/"));
    let _: &dyn std::error::Error = &err;
}

#[test]
fn test_send_sync_bounds() {
    // Verify TldrError is Send + Sync
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TldrError>();
}

// =============================================================================
// Edge Cases and Additional Tests
// =============================================================================

#[test]
fn test_error_with_special_characters() {
    let err = TldrError::UnsupportedLanguage("<script>alert('xss')</script>".to_string());
    let msg = err.to_string();
    // Should contain the special characters as-is
    assert!(msg.contains("<script>"));
}

#[test]
fn test_error_with_unicode() {
    let err = TldrError::PathNotFound(PathBuf::from("/世界/🌍/ñáéíóú"));
    let msg = err.to_string();
    assert!(msg.contains("/世界/🌍/ñáéíóú"));
}

#[test]
fn test_error_with_empty_strings() {
    let err = TldrError::DaemonError("".to_string());
    assert_eq!(err.to_string(), "Daemon error: ");
}

#[test]
fn test_suggestions_empty_vec() {
    let err = TldrError::FunctionNotFound {
        name: "func".to_string(),
        file: None,
        suggestions: vec![],
    };
    let msg = err.to_string();
    assert!(!msg.contains("Did you mean"));
}

#[test]
fn test_suggestions_single_item() {
    let err = TldrError::FunctionNotFound {
        name: "func".to_string(),
        file: None,
        suggestions: vec!["function".to_string()],
    };
    let msg = err.to_string();
    assert!(msg.contains("Did you mean:"));
    assert!(msg.contains("function"));
}

#[test]
fn test_suggestions_many_items() {
    let suggestions: Vec<String> = (0..100).map(|i| format!("suggestion{}", i)).collect();
    let err = TldrError::FunctionNotFound {
        name: "func".to_string(),
        file: None,
        suggestions,
    };
    let msg = err.to_string();
    // All suggestions should be present
    assert!(msg.contains("suggestion0"));
    assert!(msg.contains("suggestion99"));
}

#[test]
fn test_exit_code_ranges() {
    // Verify exit codes are in expected ranges
    let fs_codes = vec![
        TldrError::PathNotFound(PathBuf::from("/")).exit_code(),
        TldrError::PermissionDenied(PathBuf::from("/")).exit_code(),
    ];
    for code in fs_codes {
        assert!((2..=9).contains(&code));
    }

    let parse_codes = vec![
        TldrError::parse_error(PathBuf::from("/"), None, "err").exit_code(),
        TldrError::UnsupportedLanguage("x".to_string()).exit_code(),
    ];
    for code in parse_codes {
        assert!((10..=19).contains(&code));
    }

    let analysis_codes = vec![
        TldrError::function_not_found("x").exit_code(),
        TldrError::InvalidArgs {
            arg: "x".to_string(),
            message: "err".to_string(),
            suggestion: None,
        }
        .exit_code(),
    ];
    for code in analysis_codes {
        assert!((20..=29).contains(&code));
    }
}
