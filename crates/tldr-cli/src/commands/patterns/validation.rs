//! Input validation and path safety utilities for Pattern Analysis commands.
//!
//! Provides security-focused validation functions to mitigate:
//! - **T01 - Path Traversal**: BLOCKED_PREFIXES for system directories
//! - **T02 - Project Root Enforcement**: validate_file_path_in_project()
//! - **T03 - Integer Overflow**: Checked arithmetic for depth calculations
//! - **T08 - Memory Exhaustion**: Resource limit constants
//!
//! All file paths are canonicalized and checked against project boundaries.
//! Resource limits are enforced to prevent denial-of-service conditions.

use std::fs;
use std::path::{Path, PathBuf};

use super::error::{PatternsError, PatternsResult};

// =============================================================================
// Resource Limits (TIGER-08 Mitigations)
// =============================================================================

/// Maximum file size for analysis (10 MB).
/// Files larger than this will be rejected.
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Warning threshold for file size (1 MB).
/// Files larger than this emit a warning but are still processed.
pub const WARN_FILE_SIZE: u64 = 1024 * 1024;

/// Maximum files to scan in directory analysis.
pub const MAX_DIRECTORY_FILES: u32 = 1000;

/// Maximum AST traversal depth.
/// Prevents stack overflow from deeply nested source code.
pub const MAX_AST_DEPTH: usize = 100;

/// Maximum recursion depth for analysis algorithms.
/// Used for CFG path enumeration, temporal mining, etc.
pub const MAX_ANALYSIS_DEPTH: usize = 500;

/// Maximum function name length.
pub const MAX_FUNCTION_NAME_LEN: usize = 256;

/// Maximum constraints to report per file.
pub const MAX_CONSTRAINTS_PER_FILE: usize = 500;

/// Maximum methods per class for cohesion analysis.
pub const MAX_METHODS_PER_CLASS: usize = 200;

/// Maximum fields per class for cohesion analysis.
pub const MAX_FIELDS_PER_CLASS: usize = 100;

/// Maximum classes per file.
pub const MAX_CLASSES_PER_FILE: usize = 500;

/// Maximum CFG paths to enumerate (TIGER-04).
/// Prevents unbounded path enumeration in resources command.
pub const MAX_PATHS: usize = 1000;

/// Maximum trigrams to collect (TIGER-05).
/// Prevents memory exhaustion in temporal mining.
pub const MAX_TRIGRAMS: usize = 10000;

/// Maximum class complexity (methods * fields) for analysis.
pub const MAX_CLASS_COMPLEXITY: usize = 500;

// =============================================================================
// Blocked System Directories (TIGER-01)
// =============================================================================

/// System directories that should never be analyzed (security measure).
/// Note: We specifically target sensitive system directories.
const BLOCKED_PREFIXES: &[&str] = &[
    "/etc/",
    "/etc/passwd",
    "/etc/shadow",
    "/root/",
    "/sys/",
    "/proc/",
    "/dev/",
    "/var/run/",
    "/var/log/",
    "/private/etc/",  // macOS system config
    "C:\\Windows\\",  // Windows
    "C:\\System32\\", // Windows
];

// =============================================================================
// Path Validation (TIGER-01, TIGER-02)
// =============================================================================

/// Validate and canonicalize a file path.
///
/// This function:
/// 1. Checks that the path exists
/// 2. Canonicalizes the path (resolves symlinks, `.`, `..`)
/// 3. Rejects system directories
/// 4. Validates UTF-8 encoding
///
/// # Arguments
///
/// * `path` - The path to validate
///
/// # Returns
///
/// The canonicalized path if valid, or an error.
///
/// # Errors
///
/// - `PatternsError::FileNotFound` if the file doesn't exist
/// - `PatternsError::PathTraversal` if path is a system dir or has invalid encoding
///
/// # Example
///
/// ```ignore
/// let valid = validate_file_path(Path::new("src/main.py"))?;
/// assert!(valid.is_absolute());
/// ```
pub fn validate_file_path(path: &Path) -> PatternsResult<PathBuf> {
    // Check file exists
    if !path.exists() {
        return Err(PatternsError::FileNotFound {
            path: path.to_path_buf(),
        });
    }

    // Canonicalize the path (resolves symlinks, .., .)
    let canonical = fs::canonicalize(path).map_err(|_| PatternsError::FileNotFound {
        path: path.to_path_buf(),
    })?;

    // Check for system directories
    let canonical_str = canonical.to_string_lossy();
    for blocked in BLOCKED_PREFIXES {
        // Check with trailing slash for directories, or exact match for files
        if canonical_str.starts_with(blocked) || canonical_str == blocked.trim_end_matches('/') {
            return Err(PatternsError::PathTraversal {
                path: path.to_path_buf(),
            });
        }
    }

    // Validate UTF-8 (path.to_str() returns None if not valid UTF-8)
    if canonical.to_str().is_none() {
        return Err(PatternsError::PathTraversal {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Validate a file path ensuring it stays within a project root.
///
/// This is stricter than `validate_file_path` - it ensures the resolved
/// path is a descendant of the project root directory.
///
/// # Arguments
///
/// * `path` - The path to validate
/// * `project_root` - The project root directory to stay within
///
/// # Returns
///
/// The canonicalized path if valid and within project root.
///
/// # Errors
///
/// - `PatternsError::FileNotFound` if the file doesn't exist
/// - `PatternsError::PathTraversal` if path escapes project root
pub fn validate_file_path_in_project(path: &Path, project_root: &Path) -> PatternsResult<PathBuf> {
    // First do basic validation
    let canonical = validate_file_path(path)?;

    // Canonicalize project root too
    let canonical_root =
        fs::canonicalize(project_root).map_err(|_| PatternsError::FileNotFound {
            path: project_root.to_path_buf(),
        })?;

    // Check that canonical path starts with canonical root
    if !canonical.starts_with(&canonical_root) {
        return Err(PatternsError::PathTraversal {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Validate and canonicalize a directory path.
///
/// # Arguments
///
/// * `path` - The path to validate
///
/// # Returns
///
/// The canonicalized path if valid and is a directory.
///
/// # Errors
///
/// - `PatternsError::FileNotFound` if the directory doesn't exist
/// - `PatternsError::NotADirectory` if the path is not a directory
pub fn validate_directory_path(path: &Path) -> PatternsResult<PathBuf> {
    let canonical = validate_file_path(path)?;

    if !canonical.is_dir() {
        return Err(PatternsError::NotADirectory {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Check if a path contains path traversal patterns.
///
/// This is a quick check for suspicious patterns before canonicalization.
/// Returns true if the path looks suspicious.
///
/// # Arguments
///
/// * `path` - The path to check
///
/// # Returns
///
/// `true` if the path contains traversal patterns (`..\` or null bytes)
pub fn is_path_traversal_attempt(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    // Check for explicit traversal patterns
    if path_str.contains("..") {
        return true;
    }

    // Check for null bytes (could be used to truncate paths)
    if path_str.contains('\0') {
        return true;
    }

    false
}

// =============================================================================
// File Size Validation (TIGER-08)
// =============================================================================

/// Validate file size against limits.
///
/// # Arguments
///
/// * `path` - The path to the file
///
/// # Returns
///
/// The file size in bytes if within limits.
///
/// # Errors
///
/// - `PatternsError::FileNotFound` if file doesn't exist
/// - `PatternsError::FileTooLarge` if file exceeds MAX_FILE_SIZE
pub fn validate_file_size(path: &Path) -> PatternsResult<u64> {
    let canonical = validate_file_path(path)?;

    let metadata = fs::metadata(&canonical)?;
    let size = metadata.len();

    if size > MAX_FILE_SIZE {
        return Err(PatternsError::FileTooLarge {
            path: path.to_path_buf(),
            bytes: size,
            max_bytes: MAX_FILE_SIZE,
        });
    }

    Ok(size)
}

/// Safely read a file with size limits and UTF-8 validation.
///
/// This function:
/// 1. Validates the file path
/// 2. Checks file size against limits
/// 3. Reads the file content
/// 4. Validates UTF-8 encoding
///
/// # Arguments
///
/// * `path` - The path to the file to read
///
/// # Returns
///
/// The file contents as a String if successful.
///
/// # Errors
///
/// - `PatternsError::FileNotFound` if file doesn't exist
/// - `PatternsError::FileTooLarge` if file exceeds MAX_FILE_SIZE
/// - `PatternsError::ParseError` if file is not valid UTF-8
/// - `PatternsError::Io` for other IO errors
pub fn read_file_safe(path: &Path) -> PatternsResult<String> {
    // Validate path and size
    let canonical = validate_file_path(path)?;

    let metadata = fs::metadata(&canonical)?;
    let size = metadata.len();

    if size > MAX_FILE_SIZE {
        return Err(PatternsError::FileTooLarge {
            path: path.to_path_buf(),
            bytes: size,
            max_bytes: MAX_FILE_SIZE,
        });
    }

    // Read the file
    let content = fs::read(&canonical)?;

    // Validate UTF-8
    String::from_utf8(content).map_err(|_| PatternsError::ParseError {
        file: path.to_path_buf(),
        message: "file is not valid UTF-8".to_string(),
    })
}

// =============================================================================
// Depth Checking (TIGER-03)
// =============================================================================

/// Check if AST depth limit has been exceeded.
///
/// Uses checked comparison to avoid any overflow issues.
///
/// # Arguments
///
/// * `current_depth` - The current traversal depth
///
/// # Returns
///
/// `Ok(())` if within limits, error otherwise.
///
/// # Errors
///
/// - `PatternsError::DepthLimitExceeded` if depth >= MAX_AST_DEPTH
pub fn check_ast_depth(current_depth: usize) -> PatternsResult<()> {
    if current_depth >= MAX_AST_DEPTH {
        Err(PatternsError::DepthLimitExceeded {
            depth: current_depth.min(u32::MAX as usize) as u32,
            max_depth: MAX_AST_DEPTH as u32,
        })
    } else {
        Ok(())
    }
}

/// Check if analysis depth limit has been exceeded.
///
/// Uses saturating arithmetic to prevent overflow.
///
/// # Arguments
///
/// * `current_depth` - The current analysis depth
///
/// # Returns
///
/// `Ok(())` if within limits, error otherwise.
///
/// # Errors
///
/// - `PatternsError::DepthLimitExceeded` if depth >= MAX_ANALYSIS_DEPTH
pub fn check_analysis_depth(current_depth: usize) -> PatternsResult<()> {
    if current_depth >= MAX_ANALYSIS_DEPTH {
        Err(PatternsError::DepthLimitExceeded {
            depth: current_depth.min(u32::MAX as usize) as u32,
            max_depth: MAX_ANALYSIS_DEPTH as u32,
        })
    } else {
        Ok(())
    }
}

/// Check if directory file count limit has been exceeded.
///
/// # Arguments
///
/// * `count` - The current file count
///
/// # Returns
///
/// `Ok(())` if within limits, error otherwise.
///
/// # Errors
///
/// - `PatternsError::TooManyFiles` if count > MAX_DIRECTORY_FILES
pub fn check_directory_file_count(count: usize) -> PatternsResult<()> {
    if count > MAX_DIRECTORY_FILES as usize {
        Err(PatternsError::TooManyFiles {
            count: count.min(u32::MAX as usize) as u32,
            max_files: MAX_DIRECTORY_FILES,
        })
    } else {
        Ok(())
    }
}

// =============================================================================
// Function Name Validation
// =============================================================================

/// Validate a function name for safety.
///
/// Ensures the name:
/// - Is not empty
/// - Contains only valid identifier characters
/// - Doesn't exceed maximum length
/// - Doesn't contain suspicious characters
///
/// # Arguments
///
/// * `name` - The function name to validate
///
/// # Returns
///
/// `Ok(())` if valid, error otherwise.
///
/// # Errors
///
/// - `PatternsError::InvalidParameter` for invalid names
pub fn validate_function_name(name: &str) -> PatternsResult<()> {
    // Check empty
    if name.is_empty() {
        return Err(PatternsError::InvalidParameter {
            message: "function name cannot be empty".to_string(),
        });
    }

    // Check length
    if name.len() > MAX_FUNCTION_NAME_LEN {
        return Err(PatternsError::InvalidParameter {
            message: format!(
                "function name too long ({} chars, max {})",
                name.len(),
                MAX_FUNCTION_NAME_LEN
            ),
        });
    }

    // Check for suspicious characters that could be used for injection
    // Valid identifiers: letters, digits, underscore (and some languages allow $)
    let suspicious_chars = [
        ';', '(', ')', '{', '}', '[', ']', '`', '"', '\'', '\\', '/', '\0',
    ];
    for c in name.chars() {
        if suspicious_chars.contains(&c) {
            return Err(PatternsError::InvalidParameter {
                message: format!("function name contains invalid character: '{}'", c),
            });
        }
    }

    // First character should be letter or underscore (standard identifier rules)
    if let Some(first) = name.chars().next() {
        if !first.is_alphabetic() && first != '_' {
            return Err(PatternsError::InvalidParameter {
                message: "function name must start with letter or underscore".to_string(),
            });
        }
    }

    Ok(())
}

// =============================================================================
// Checked Arithmetic Utilities (TIGER-03)
// =============================================================================

/// Safely increment a depth counter with overflow protection.
///
/// Returns the incremented value or saturates at usize::MAX.
///
/// # Arguments
///
/// * `depth` - The current depth value
///
/// # Returns
///
/// The incremented depth (or usize::MAX if overflow would occur)
#[inline]
pub fn saturating_depth_increment(depth: usize) -> usize {
    depth.saturating_add(1)
}

/// Safely add to a counter with overflow protection.
///
/// Returns the sum or saturates at the type maximum.
///
/// # Arguments
///
/// * `count` - The current count
/// * `add` - The amount to add
///
/// # Returns
///
/// The sum (or type max if overflow would occur)
#[inline]
pub fn saturating_count_add(count: u32, add: u32) -> u32 {
    count.saturating_add(add)
}

/// Check if a value is within a limit using checked arithmetic.
///
/// # Arguments
///
/// * `value` - The value to check
/// * `limit` - The maximum allowed value
///
/// # Returns
///
/// `true` if value < limit
#[inline]
pub fn within_limit(value: usize, limit: usize) -> bool {
    value < limit
}

// =============================================================================
// Warning Utilities
// =============================================================================

/// Check if a file size is large enough to warrant a warning.
///
/// # Arguments
///
/// * `size` - The file size in bytes
///
/// # Returns
///
/// `true` if size > WARN_FILE_SIZE
#[inline]
pub fn should_warn_file_size(size: u64) -> bool {
    size > WARN_FILE_SIZE
}

/// Format a warning message for a large file.
///
/// # Arguments
///
/// * `path` - The file path
/// * `size` - The file size in bytes
///
/// # Returns
///
/// A formatted warning string
pub fn format_large_file_warning(path: &Path, size: u64) -> String {
    format!(
        "Warning: {} is large ({:.1} MB), analysis may be slow",
        path.display(),
        size as f64 / 1024.0 / 1024.0
    )
}

// =============================================================================
// Near-Limit Warning Utilities
// =============================================================================

/// Check if a count is approaching a limit (>80%).
///
/// # Arguments
///
/// * `count` - The current count
/// * `limit` - The maximum limit
///
/// # Returns
///
/// `true` if count > 80% of limit
#[inline]
pub fn approaching_limit(count: usize, limit: usize) -> bool {
    // Use checked arithmetic to avoid overflow
    let threshold = limit.saturating_mul(80) / 100;
    count > threshold
}

/// Log a warning if approaching a limit.
///
/// # Arguments
///
/// * `count` - The current count
/// * `limit` - The maximum limit
/// * `resource_name` - Name of the resource for the warning message
pub fn warn_if_approaching_limit(count: usize, limit: usize, resource_name: &str) {
    if approaching_limit(count, limit) {
        eprintln!(
            "Warning: {} count ({}) approaching limit ({})",
            resource_name, count, limit
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use tempfile::{tempdir, NamedTempFile};

    // =========================================================================
    // Resource Limits Constants Tests (TIGER-08)
    // =========================================================================

    #[test]
    fn test_resource_limits_constants() {
        // Verify TIGER mitigation constants have sensible values
        assert_eq!(MAX_FILE_SIZE, 10 * 1024 * 1024); // 10 MB
        assert_eq!(MAX_DIRECTORY_FILES, 1000);
        assert_eq!(MAX_AST_DEPTH, 100); // TIGER-08
        assert_eq!(MAX_ANALYSIS_DEPTH, 500);
        assert_eq!(MAX_PATHS, 1000); // TIGER-04
        assert_eq!(MAX_TRIGRAMS, 10000); // TIGER-05
        assert_eq!(MAX_CLASS_COMPLEXITY, 500);
    }

    // =========================================================================
    // Path Validation Tests (TIGER-01, TIGER-02)
    // =========================================================================

    #[test]
    fn test_validate_file_path_normal() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path();

        let result = validate_file_path(path);
        assert!(result.is_ok());

        let canonical = result.unwrap();
        assert!(canonical.is_absolute());
    }

    #[test]
    fn test_validate_file_path_not_exists() {
        let result = validate_file_path(Path::new("/nonexistent/file.py"));
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::FileNotFound { path } => {
                assert!(path.to_string_lossy().contains("nonexistent"));
            }
            e => panic!("Expected FileNotFound error, got {:?}", e),
        }
    }

    #[test]
    fn test_validate_file_path_traversal_blocked_dotdot() {
        // Check that path with .. pattern is detected
        let suspicious = Path::new("../etc/passwd");
        assert!(is_path_traversal_attempt(suspicious));
    }

    #[test]
    fn test_validate_file_path_traversal_blocked_null() {
        let suspicious = Path::new("file\0.txt");
        assert!(is_path_traversal_attempt(suspicious));
    }

    #[test]
    fn test_validate_file_path_in_project_valid() {
        let project = tempdir().unwrap();
        let file_path = project.path().join("src/main.py");
        fs::create_dir_all(project.path().join("src")).unwrap();
        fs::write(&file_path, "# test").unwrap();

        let result = validate_file_path_in_project(&file_path, project.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_file_path_outside_project() {
        let project = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").unwrap();

        let result = validate_file_path_in_project(&outside_file, project.path());
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::PathTraversal { .. } => {}
            e => panic!("Expected PathTraversal error, got {:?}", e),
        }
    }

    #[test]
    fn test_validate_path_blocked_system_dirs() {
        // Test that system directories are blocked
        let blocked = [
            "/etc/passwd",
            "/root/.bashrc",
            "/sys/kernel/config",
            "/proc/self/status",
        ];

        for path_str in blocked {
            let path = Path::new(path_str);
            // If the file exists on this system, it should be rejected
            if path.exists() {
                let result = validate_file_path(path);
                assert!(result.is_err(), "Should reject system path: {}", path_str);
            }
        }
    }

    #[test]
    fn test_validate_directory_path_exists() {
        let dir = tempdir().unwrap();
        let result = validate_directory_path(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_directory_path_is_file() {
        let file = NamedTempFile::new().unwrap();
        let result = validate_directory_path(file.path());
        assert!(result.is_err());
    }

    // =========================================================================
    // File Size Validation Tests (TIGER-08)
    // =========================================================================

    #[test]
    fn test_validate_file_size_ok() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "small content").unwrap();

        let result = validate_file_size(file.path());
        assert!(result.is_ok());
        assert!(result.unwrap() < MAX_FILE_SIZE);
    }

    #[test]
    fn test_validate_file_size_not_exists() {
        let result = validate_file_size(Path::new("/nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_file_safe_success() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "def hello():\n    print('hello')").unwrap();

        let content = read_file_safe(file.path()).unwrap();
        assert!(content.contains("def hello"));
        assert!(content.contains("print"));
    }

    #[test]
    fn test_read_file_safe_not_utf8() {
        let temp = tempdir().unwrap();
        let binary_file = temp.path().join("binary.bin");

        // Write invalid UTF-8 bytes
        let invalid_utf8 = vec![0xFF, 0xFE, 0x00, 0x01];
        fs::write(&binary_file, invalid_utf8).unwrap();

        let result = read_file_safe(&binary_file);
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::ParseError { message, .. } => {
                assert!(message.contains("UTF-8"));
            }
            e => panic!("Expected ParseError, got {:?}", e),
        }
    }

    // =========================================================================
    // Depth Checking Tests (TIGER-03)
    // =========================================================================

    #[test]
    fn test_check_ast_depth_ok() {
        assert!(check_ast_depth(0).is_ok());
        assert!(check_ast_depth(50).is_ok());
        assert!(check_ast_depth(MAX_AST_DEPTH - 1).is_ok());
    }

    #[test]
    fn test_check_ast_depth_exceeded() {
        let result = check_ast_depth(MAX_AST_DEPTH);
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::DepthLimitExceeded { depth, max_depth } => {
                assert_eq!(depth, MAX_AST_DEPTH as u32);
                assert_eq!(max_depth, MAX_AST_DEPTH as u32);
            }
            e => panic!("Expected DepthLimitExceeded error, got {:?}", e),
        }
    }

    #[test]
    fn test_check_analysis_depth_ok() {
        assert!(check_analysis_depth(0).is_ok());
        assert!(check_analysis_depth(MAX_ANALYSIS_DEPTH - 1).is_ok());
    }

    #[test]
    fn test_check_analysis_depth_exceeded() {
        let result = check_analysis_depth(MAX_ANALYSIS_DEPTH);
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::DepthLimitExceeded { .. } => {}
            e => panic!("Expected DepthLimitExceeded error, got {:?}", e),
        }
    }

    #[test]
    fn test_check_directory_file_count_ok() {
        assert!(check_directory_file_count(0).is_ok());
        assert!(check_directory_file_count(500).is_ok());
        assert!(check_directory_file_count(MAX_DIRECTORY_FILES as usize - 1).is_ok());
    }

    #[test]
    fn test_check_directory_file_count_exceeded() {
        let result = check_directory_file_count(MAX_DIRECTORY_FILES as usize + 1);
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::TooManyFiles { .. } => {}
            e => panic!("Expected TooManyFiles error, got {:?}", e),
        }
    }

    // =========================================================================
    // Function Name Validation Tests
    // =========================================================================

    #[test]
    fn test_validate_function_name_valid() {
        assert!(validate_function_name("my_function").is_ok());
        assert!(validate_function_name("_private").is_ok());
        assert!(validate_function_name("CamelCase").is_ok());
        assert!(validate_function_name("func123").is_ok());
        assert!(validate_function_name("__dunder__").is_ok());
    }

    #[test]
    fn test_validate_function_name_empty() {
        let result = validate_function_name("");
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::InvalidParameter { message } => {
                assert!(message.contains("empty"));
            }
            e => panic!("Expected InvalidParameter error, got {:?}", e),
        }
    }

    #[test]
    fn test_validate_function_name_too_long() {
        let long_name = "a".repeat(MAX_FUNCTION_NAME_LEN + 1);
        let result = validate_function_name(&long_name);
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::InvalidParameter { message } => {
                assert!(message.contains("too long"));
            }
            e => panic!("Expected InvalidParameter error, got {:?}", e),
        }
    }

    #[test]
    fn test_validate_function_name_invalid_chars() {
        let invalid_names = [
            "func;drop",  // semicolon
            "func()",     // parentheses
            "func{}",     // braces
            "func`cmd`",  // backticks
            "func\"name", // quotes
            "func\\name", // backslash
            "func/name",  // forward slash
        ];

        for name in invalid_names {
            let result = validate_function_name(name);
            assert!(result.is_err(), "Should reject: {}", name);
        }
    }

    #[test]
    fn test_validate_function_name_invalid_start() {
        let result = validate_function_name("123func");
        assert!(result.is_err());

        match result.unwrap_err() {
            PatternsError::InvalidParameter { message } => {
                assert!(message.contains("start with"));
            }
            e => panic!("Expected InvalidParameter error, got {:?}", e),
        }
    }

    // =========================================================================
    // Path Traversal Pattern Detection Tests
    // =========================================================================

    #[test]
    fn test_is_path_traversal_attempt_dotdot() {
        assert!(is_path_traversal_attempt(Path::new("../etc/passwd")));
        assert!(is_path_traversal_attempt(Path::new("foo/../bar")));
        assert!(is_path_traversal_attempt(Path::new(
            "..\\Windows\\System32"
        )));
    }

    #[test]
    fn test_is_path_traversal_attempt_normal() {
        assert!(!is_path_traversal_attempt(Path::new("src/main.rs")));
        assert!(!is_path_traversal_attempt(Path::new("/home/user/project")));
        assert!(!is_path_traversal_attempt(Path::new(".")));
    }

    // =========================================================================
    // Checked Arithmetic Tests (TIGER-03)
    // =========================================================================

    #[test]
    fn test_checked_depth_increment() {
        // Test that we use checked arithmetic - verify the function handles edge cases
        let current = usize::MAX - 1;
        // This should not panic due to overflow
        let result = check_analysis_depth(current);
        assert!(result.is_err()); // Should exceed limit, not overflow
    }

    #[test]
    fn test_saturating_depth_increment() {
        assert_eq!(saturating_depth_increment(0), 1);
        assert_eq!(saturating_depth_increment(100), 101);
        assert_eq!(saturating_depth_increment(usize::MAX), usize::MAX);
    }

    #[test]
    fn test_saturating_count_add() {
        assert_eq!(saturating_count_add(0, 1), 1);
        assert_eq!(saturating_count_add(100, 50), 150);
        assert_eq!(saturating_count_add(u32::MAX, 1), u32::MAX);
    }

    #[test]
    fn test_within_limit() {
        assert!(within_limit(0, 100));
        assert!(within_limit(99, 100));
        assert!(!within_limit(100, 100));
        assert!(!within_limit(101, 100));
    }

    #[test]
    fn test_approaching_limit() {
        // 80% of 100 = 80
        assert!(!approaching_limit(79, 100));
        assert!(!approaching_limit(80, 100));
        assert!(approaching_limit(81, 100));
        assert!(approaching_limit(100, 100));
    }

    #[test]
    fn test_should_warn_file_size() {
        assert!(!should_warn_file_size(0));
        assert!(!should_warn_file_size(WARN_FILE_SIZE));
        assert!(should_warn_file_size(WARN_FILE_SIZE + 1));
    }

    #[test]
    fn test_format_large_file_warning() {
        let warning = format_large_file_warning(Path::new("/test/file.py"), 2 * 1024 * 1024);
        assert!(warning.contains("file.py"));
        assert!(warning.contains("2.0 MB"));
        assert!(warning.contains("Warning"));
    }
}
