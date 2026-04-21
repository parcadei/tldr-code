//! File encoding handling (Phase 10)
//!
//! This module provides robust handling of file encodings during analysis.
//!
//! # Mitigations
//!
//! - A34: Silent data corruption on non-UTF8 files
//!   - Detects and handles UTF-8 BOM
//!   - Falls back to lossy decoding with warning
//!   - Detects and skips binary files
//!
//! # Example
//!
//! ```rust,ignore
//! use tldr_core::encoding::{read_source_file, FileReadResult};
//! use std::path::Path;
//!
//! match read_source_file(Path::new("example.py")) {
//!     Ok(FileReadResult::Ok(content)) => {
//!         // Process UTF-8 content
//!     }
//!     Ok(FileReadResult::Lossy { content, warning }) => {
//!         // Process with warning
//!         eprintln!("Warning: {}", warning);
//!     }
//!     Ok(FileReadResult::Binary) => {
//!         // Skip binary file
//!     }
//!     Err(e) => {
//!         // Handle IO error
//!     }
//! }
//! ```

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::TldrError;

// =============================================================================
// File Read Result
// =============================================================================

/// Result of reading a source file with encoding detection.
#[derive(Debug, Clone)]
pub enum FileReadResult {
    /// File was valid UTF-8 (possibly with BOM stripped)
    Ok(String),
    /// File required lossy UTF-8 decoding
    Lossy {
        /// The decoded content (with replacement characters)
        content: String,
        /// Warning message about encoding issues
        warning: String,
    },
    /// File appears to be binary (contains null bytes)
    Binary,
}

impl FileReadResult {
    /// Get the content if available.
    pub fn content(&self) -> Option<&str> {
        match self {
            FileReadResult::Ok(s) => Some(s),
            FileReadResult::Lossy { content, .. } => Some(content),
            FileReadResult::Binary => None,
        }
    }

    /// Check if this result has a warning.
    pub fn has_warning(&self) -> bool {
        matches!(self, FileReadResult::Lossy { .. })
    }

    /// Get the warning message if any.
    pub fn warning(&self) -> Option<&str> {
        match self {
            FileReadResult::Lossy { warning, .. } => Some(warning),
            _ => None,
        }
    }

    /// Check if file is binary.
    pub fn is_binary(&self) -> bool {
        matches!(self, FileReadResult::Binary)
    }
}

// =============================================================================
// Encoding Issues Tracking
// =============================================================================

/// Record of encoding issues encountered during analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EncodingIssues {
    /// Files that required lossy UTF-8 decoding
    pub lossy_files: Vec<EncodingIssue>,
    /// Files that were skipped as binary
    pub binary_files: Vec<String>,
    /// Files with UTF-8 BOM (stripped)
    pub bom_files: Vec<String>,
}

impl EncodingIssues {
    /// Create a new issues tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a lossy decode.
    pub fn add_lossy(&mut self, file: impl Into<String>, issue: impl Into<String>) {
        self.lossy_files.push(EncodingIssue {
            file: file.into(),
            issue: issue.into(),
        });
    }

    /// Record a binary file skip.
    pub fn add_binary(&mut self, file: impl Into<String>) {
        self.binary_files.push(file.into());
    }

    /// Record a BOM file.
    pub fn add_bom(&mut self, file: impl Into<String>) {
        self.bom_files.push(file.into());
    }

    /// Check if any issues were recorded.
    pub fn has_issues(&self) -> bool {
        !self.lossy_files.is_empty() || !self.binary_files.is_empty()
    }

    /// Get total number of issues.
    pub fn total(&self) -> usize {
        self.lossy_files.len() + self.binary_files.len()
    }
}

/// A single encoding issue record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodingIssue {
    /// File path
    pub file: String,
    /// Issue description
    pub issue: String,
}

// =============================================================================
// File Reading Functions
// =============================================================================

/// UTF-8 BOM bytes
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// UTF-16 LE BOM bytes
const UTF16_LE_BOM: &[u8] = &[0xFF, 0xFE];

/// UTF-16 BE BOM bytes
const UTF16_BE_BOM: &[u8] = &[0xFE, 0xFF];

/// Read a source file with encoding detection.
///
/// This function:
/// 1. Reads the file as bytes
/// 2. Checks for and strips UTF-8 BOM
/// 3. Detects UTF-16 BOM and reports unsupported
/// 4. Attempts UTF-8 decoding
/// 5. Falls back to lossy decoding if needed
/// 6. Detects binary files (contains null bytes)
///
/// # Arguments
///
/// * `path` - Path to the file to read
///
/// # Returns
///
/// * `Ok(FileReadResult::Ok(content))` - Valid UTF-8 content
/// * `Ok(FileReadResult::Lossy { content, warning })` - Lossy decoded content with warning
/// * `Ok(FileReadResult::Binary)` - File is binary
/// * `Err(TldrError)` - IO error
pub fn read_source_file(path: &Path) -> Result<FileReadResult, TldrError> {
    let bytes = std::fs::read(path)?;

    // Check for UTF-16 BOM (unsupported, would need conversion)
    if bytes.starts_with(UTF16_LE_BOM) || bytes.starts_with(UTF16_BE_BOM) {
        return Ok(FileReadResult::Lossy {
            content: String::new(),
            warning: format!(
                "File {} appears to be UTF-16 encoded (unsupported), skipping",
                path.display()
            ),
        });
    }

    // Check for and strip UTF-8 BOM
    let (bytes, had_bom) = if bytes.starts_with(UTF8_BOM) {
        (&bytes[3..], true)
    } else {
        (&bytes[..], false)
    };

    // Check for binary file (null bytes in first 8KB)
    let check_len = bytes.len().min(8192);
    if bytes[..check_len].contains(&0) {
        return Ok(FileReadResult::Binary);
    }

    // Try UTF-8 decoding
    match String::from_utf8(bytes.to_vec()) {
        Ok(content) => {
            if had_bom {
                // Valid UTF-8 with BOM stripped (not a warning, just note)
                Ok(FileReadResult::Ok(content))
            } else {
                Ok(FileReadResult::Ok(content))
            }
        }
        Err(_) => {
            // Fall back to lossy decoding
            let content = String::from_utf8_lossy(bytes).into_owned();
            let replacement_count = content.matches('\u{FFFD}').count();
            Ok(FileReadResult::Lossy {
                content,
                warning: format!(
                    "File {} is not valid UTF-8, used lossy decoding ({} replacement characters)",
                    path.display(),
                    replacement_count
                ),
            })
        }
    }
}

/// Read a source file, returning the content or skipping on error.
///
/// This is a convenience function that:
/// - Returns `Some(content)` for valid files
/// - Returns `None` for binary files or errors
/// - Optionally records issues in an EncodingIssues tracker
///
/// # Arguments
///
/// * `path` - Path to the file
/// * `issues` - Optional issues tracker
///
/// # Returns
///
/// * `Some(String)` - File content (may be lossy decoded)
/// * `None` - File was skipped (binary, error, etc.)
pub fn read_source_file_or_skip(
    path: &Path,
    issues: Option<&mut EncodingIssues>,
) -> Option<String> {
    match read_source_file(path) {
        Ok(FileReadResult::Ok(content)) => Some(content),
        Ok(FileReadResult::Lossy { content, warning }) => {
            if let Some(issues) = issues {
                issues.add_lossy(path.display().to_string(), &warning);
            }
            Some(content)
        }
        Ok(FileReadResult::Binary) => {
            if let Some(issues) = issues {
                issues.add_binary(path.display().to_string());
            }
            None
        }
        Err(_) => None,
    }
}

/// Check if a file appears to be binary.
///
/// Reads the first 8KB of the file and checks for null bytes.
pub fn is_binary_file(path: &Path) -> Result<bool, TldrError> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);

    let mut buffer = [0u8; 8192];
    use std::io::Read;
    let bytes_read = reader.read(&mut buffer)?;

    Ok(buffer[..bytes_read].contains(&0))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_utf8_file() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "Hello, world!").unwrap();

        let result = read_source_file(file.path()).unwrap();
        assert!(matches!(result, FileReadResult::Ok(_)));
        assert_eq!(result.content(), Some("Hello, world!"));
    }

    #[test]
    fn test_read_utf8_bom_file() {
        let mut file = NamedTempFile::new().unwrap();
        // Write UTF-8 BOM followed by content
        file.write_all(&[0xEF, 0xBB, 0xBF]).unwrap();
        file.write_all(b"Hello, BOM!").unwrap();

        let result = read_source_file(file.path()).unwrap();
        assert!(matches!(result, FileReadResult::Ok(_)));
        assert_eq!(result.content(), Some("Hello, BOM!"));
    }

    #[test]
    fn test_read_binary_file() {
        let mut file = NamedTempFile::new().unwrap();
        // Write some binary content with null bytes
        file.write_all(&[0x00, 0x01, 0x02, 0x00]).unwrap();

        let result = read_source_file(file.path()).unwrap();
        assert!(matches!(result, FileReadResult::Binary));
        assert!(result.is_binary());
        assert!(result.content().is_none());
    }

    #[test]
    fn test_read_invalid_utf8() {
        let mut file = NamedTempFile::new().unwrap();
        // Write invalid UTF-8 sequence (no null bytes)
        file.write_all(&[0x80, 0x81, 0x82, 0x61, 0x62, 0x63])
            .unwrap();

        let result = read_source_file(file.path()).unwrap();
        assert!(matches!(result, FileReadResult::Lossy { .. }));
        assert!(result.has_warning());
    }

    #[test]
    fn test_encoding_issues_tracker() {
        let mut issues = EncodingIssues::new();
        assert!(!issues.has_issues());

        issues.add_lossy("file1.py", "Invalid UTF-8");
        issues.add_binary("file2.bin");
        issues.add_bom("file3.py");

        assert!(issues.has_issues());
        assert_eq!(issues.total(), 2); // lossy + binary
        assert_eq!(issues.lossy_files.len(), 1);
        assert_eq!(issues.binary_files.len(), 1);
        assert_eq!(issues.bom_files.len(), 1);
    }

    #[test]
    fn test_read_source_file_or_skip_valid() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "def foo(): pass").unwrap();

        let mut issues = EncodingIssues::new();
        let content = read_source_file_or_skip(file.path(), Some(&mut issues));

        assert!(content.is_some());
        assert!(!issues.has_issues());
    }

    #[test]
    fn test_read_source_file_or_skip_binary() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&[0x00, 0x01, 0x02]).unwrap();

        let mut issues = EncodingIssues::new();
        let content = read_source_file_or_skip(file.path(), Some(&mut issues));

        assert!(content.is_none());
        assert_eq!(issues.binary_files.len(), 1);
    }

    #[test]
    fn test_is_binary_file() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&[0x00, 0x01]).unwrap();

        assert!(is_binary_file(file.path()).unwrap());

        let mut text_file = NamedTempFile::new().unwrap();
        write!(text_file, "text content").unwrap();

        assert!(!is_binary_file(text_file.path()).unwrap());
    }
}
