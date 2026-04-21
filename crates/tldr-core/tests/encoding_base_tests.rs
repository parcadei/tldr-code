//! Test coverage for tldr-core encoding module
//!
//! Tests all public functions and types from:
//! - crates/tldr-core/src/encoding.rs

use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;

// Import from tldr_core
use tldr_core::encoding::*;
use tldr_core::TldrError;

// =============================================================================
// FileReadResult Enum Tests
// =============================================================================

#[test]
fn test_file_read_result_variants() {
    // Test creating all variants
    let ok_result = FileReadResult::Ok("content".to_string());
    let lossy_result = FileReadResult::Lossy {
        content: "content".to_string(),
        warning: "warning".to_string(),
    };
    let binary_result = FileReadResult::Binary;

    // Just verify they can be created
    assert!(matches!(ok_result, FileReadResult::Ok(_)));
    assert!(matches!(lossy_result, FileReadResult::Lossy { .. }));
    assert!(matches!(binary_result, FileReadResult::Binary));
}

#[test]
fn test_file_read_result_content_ok() {
    let result = FileReadResult::Ok("Hello, world!".to_string());
    assert_eq!(result.content(), Some("Hello, world!"));
}

#[test]
fn test_file_read_result_content_lossy() {
    let result = FileReadResult::Lossy {
        content: "Hello, world!".to_string(),
        warning: "Had issues".to_string(),
    };
    assert_eq!(result.content(), Some("Hello, world!"));
}

#[test]
fn test_file_read_result_content_binary() {
    let result = FileReadResult::Binary;
    assert_eq!(result.content(), None);
}

#[test]
fn test_file_read_result_has_warning_ok() {
    let result = FileReadResult::Ok("content".to_string());
    assert!(!result.has_warning());
}

#[test]
fn test_file_read_result_has_warning_lossy() {
    let result = FileReadResult::Lossy {
        content: "content".to_string(),
        warning: "Had issues".to_string(),
    };
    assert!(result.has_warning());
}

#[test]
fn test_file_read_result_has_warning_binary() {
    let result = FileReadResult::Binary;
    assert!(!result.has_warning());
}

#[test]
fn test_file_read_result_warning_ok() {
    let result = FileReadResult::Ok("content".to_string());
    assert_eq!(result.warning(), None);
}

#[test]
fn test_file_read_result_warning_lossy() {
    let result = FileReadResult::Lossy {
        content: "content".to_string(),
        warning: "Invalid UTF-8 detected".to_string(),
    };
    assert_eq!(result.warning(), Some("Invalid UTF-8 detected"));
}

#[test]
fn test_file_read_result_is_binary_ok() {
    let result = FileReadResult::Ok("content".to_string());
    assert!(!result.is_binary());
}

#[test]
fn test_file_read_result_is_binary_lossy() {
    let result = FileReadResult::Lossy {
        content: "content".to_string(),
        warning: "warning".to_string(),
    };
    assert!(!result.is_binary());
}

#[test]
fn test_file_read_result_is_binary_binary() {
    let result = FileReadResult::Binary;
    assert!(result.is_binary());
}

// =============================================================================
// EncodingIssues Tests
// =============================================================================

#[test]
fn test_encoding_issues_new() {
    let issues = EncodingIssues::new();
    assert!(!issues.has_issues());
    assert_eq!(issues.total(), 0);
    assert!(issues.lossy_files.is_empty());
    assert!(issues.binary_files.is_empty());
    assert!(issues.bom_files.is_empty());
}

#[test]
fn test_encoding_issues_default() {
    let issues: EncodingIssues = Default::default();
    assert!(!issues.has_issues());
}

#[test]
fn test_encoding_issues_add_lossy() {
    let mut issues = EncodingIssues::new();
    issues.add_lossy("file.py", "Invalid UTF-8 at byte 42");

    assert!(issues.has_issues());
    assert_eq!(issues.total(), 1);
    assert_eq!(issues.lossy_files.len(), 1);
    assert_eq!(issues.lossy_files[0].file, "file.py");
    assert_eq!(issues.lossy_files[0].issue, "Invalid UTF-8 at byte 42");
}

#[test]
fn test_encoding_issues_add_binary() {
    let mut issues = EncodingIssues::new();
    issues.add_binary("binary.dll");

    assert!(issues.has_issues());
    assert_eq!(issues.total(), 1);
    assert_eq!(issues.binary_files.len(), 1);
    assert_eq!(issues.binary_files[0], "binary.dll");
}

#[test]
fn test_encoding_issues_add_bom() {
    let mut issues = EncodingIssues::new();
    issues.add_bom("file_with_bom.py");

    // BOM files don't count as "issues" for has_issues()
    assert!(!issues.has_issues());
    assert_eq!(issues.total(), 0);
    assert_eq!(issues.bom_files.len(), 1);
    assert_eq!(issues.bom_files[0], "file_with_bom.py");
}

#[test]
fn test_encoding_issues_multiple() {
    let mut issues = EncodingIssues::new();
    issues.add_lossy("file1.py", "issue1");
    issues.add_lossy("file2.py", "issue2");
    issues.add_binary("binary1.dll");
    issues.add_bom("bom1.py");
    issues.add_bom("bom2.py");

    assert!(issues.has_issues());
    assert_eq!(issues.total(), 3); // 2 lossy + 1 binary (BOM not counted)
    assert_eq!(issues.lossy_files.len(), 2);
    assert_eq!(issues.binary_files.len(), 1);
    assert_eq!(issues.bom_files.len(), 2);
}

#[test]
fn test_encoding_issue_struct() {
    let issue = EncodingIssue {
        file: "test.py".to_string(),
        issue: "Invalid encoding".to_string(),
    };

    assert_eq!(issue.file, "test.py");
    assert_eq!(issue.issue, "Invalid encoding");
}

#[test]
fn test_encoding_issues_serde() {
    let mut issues = EncodingIssues::new();
    issues.add_lossy("file.py", "issue");
    issues.add_binary("binary.dll");
    issues.add_bom("bom.py");

    let json = serde_json::to_string(&issues).unwrap();
    let parsed: EncodingIssues = serde_json::from_str(&json).unwrap();

    assert_eq!(issues.lossy_files.len(), parsed.lossy_files.len());
    assert_eq!(issues.binary_files.len(), parsed.binary_files.len());
    assert_eq!(issues.bom_files.len(), parsed.bom_files.len());
}

// =============================================================================
// read_source_file Tests
// =============================================================================

#[test]
fn test_read_source_file_utf8() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "Hello, world!").unwrap();

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some("Hello, world!"));
    assert!(!result.has_warning());
    assert!(!result.is_binary());
}

#[test]
fn test_read_source_file_empty() {
    let file = NamedTempFile::new().unwrap();
    // Empty file

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some(""));
}

#[test]
fn test_read_source_file_with_unicode() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "Hello, 世界! 🌍 ñáéíóú").unwrap();

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some("Hello, 世界! 🌍 ñáéíóú"));
}

#[test]
fn test_read_source_file_with_newlines() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "line1\nline2\r\nline3\n").unwrap();

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some("line1\nline2\r\nline3\n"));
}

#[test]
fn test_read_source_file_utf8_bom() {
    let mut file = NamedTempFile::new().unwrap();
    // Write UTF-8 BOM followed by content
    file.write_all(&[0xEF, 0xBB, 0xBF]).unwrap();
    file.write_all(b"Hello with BOM!").unwrap();

    let result = read_source_file(file.path()).unwrap();

    // BOM should be stripped
    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some("Hello with BOM!"));
    assert!(!result.has_warning());
}

#[test]
fn test_read_source_file_utf8_bom_only() {
    let mut file = NamedTempFile::new().unwrap();
    // Write only UTF-8 BOM
    file.write_all(&[0xEF, 0xBB, 0xBF]).unwrap();

    let result = read_source_file(file.path()).unwrap();

    // Should be Ok with empty content
    assert!(matches!(result, FileReadResult::Ok(_)));
    assert_eq!(result.content(), Some(""));
}

#[test]
fn test_read_source_file_binary_null_bytes() {
    let mut file = NamedTempFile::new().unwrap();
    // Write binary content with null bytes
    file.write_all(&[0x00, 0x01, 0x02, 0x03]).unwrap();

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Binary));
    assert!(result.is_binary());
    assert_eq!(result.content(), None);
}

#[test]
fn test_read_source_file_binary_null_in_middle() {
    let mut file = NamedTempFile::new().unwrap();
    // Write text with null byte in middle
    file.write_all(b"Hello\x00World").unwrap();

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Binary));
}

#[test]
#[ignore = "BUG-003: is_binary_file only reads 8KB and may miss null bytes beyond that boundary"]
fn test_read_source_file_binary_after_8kb() {
    let mut file = NamedTempFile::new().unwrap();
    // Write 8KB of text followed by null byte
    let text = "a".repeat(8192);
    file.write_all(text.as_bytes()).unwrap();
    file.write_all(&[0x00]).unwrap();

    let result = read_source_file(file.path()).unwrap();

    // BUG: Null byte after 8KB boundary is not detected
    // because only first 8KB is checked
    // When fixed, this should be:
    // assert!(matches!(result, FileReadResult::Binary));
    // Currently it returns Ok (not detecting the null)
    assert!(matches!(result, FileReadResult::Ok(_)));
}

#[test]
fn test_read_source_file_invalid_utf8() {
    let mut file = NamedTempFile::new().unwrap();
    // Write invalid UTF-8 sequence (high bit set without proper continuation)
    file.write_all(&[0x80, 0x81, 0x82, 0x61, 0x62, 0x63])
        .unwrap();

    let result = read_source_file(file.path()).unwrap();

    // Should be lossy with warning
    assert!(matches!(result, FileReadResult::Lossy { .. }));
    assert!(result.has_warning());
    assert!(result.warning().unwrap().contains("not valid UTF-8"));
    assert!(result.content().is_some());
}

#[test]
fn test_read_source_file_invalid_utf8_with_replacement() {
    let mut file = NamedTempFile::new().unwrap();
    // Write invalid UTF-8 sequence
    file.write_all(&[0xC0, 0x80]).unwrap(); // Overlong encoding

    let result = read_source_file(file.path()).unwrap();

    assert!(matches!(result, FileReadResult::Lossy { .. }));
    let content = result.content().unwrap();
    // Should have replacement character
    assert!(content.contains('\u{FFFD}'));
}

#[test]
fn test_read_source_file_utf16_le_bom() {
    let mut file = NamedTempFile::new().unwrap();
    // Write UTF-16 LE BOM
    file.write_all(&[0xFF, 0xFE]).unwrap();
    file.write_all(b"Some content").unwrap();

    let result = read_source_file(file.path()).unwrap();

    // UTF-16 is not supported, returns Lossy with empty content
    assert!(matches!(result, FileReadResult::Lossy { .. }));
    assert!(result.warning().unwrap().contains("UTF-16"));
    assert_eq!(result.content(), Some(""));
}

#[test]
fn test_read_source_file_utf16_be_bom() {
    let mut file = NamedTempFile::new().unwrap();
    // Write UTF-16 BE BOM
    file.write_all(&[0xFE, 0xFF]).unwrap();
    file.write_all(b"Some content").unwrap();

    let result = read_source_file(file.path()).unwrap();

    // UTF-16 is not supported
    assert!(matches!(result, FileReadResult::Lossy { .. }));
    assert!(result.warning().unwrap().contains("UTF-16"));
}

#[test]
fn test_read_source_file_not_found() {
    let result = read_source_file(Path::new("/definitely/nonexistent/path/file.txt"));

    assert!(result.is_err());
    // Should be IO error
    assert!(matches!(result.unwrap_err(), TldrError::IoError(_)));
}

// =============================================================================
// read_source_file_or_skip Tests
// =============================================================================

#[test]
fn test_read_source_file_or_skip_valid() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "def foo(): pass").unwrap();

    let mut issues = EncodingIssues::new();
    let content = read_source_file_or_skip(file.path(), Some(&mut issues));

    assert!(content.is_some());
    assert_eq!(content.unwrap(), "def foo(): pass");
    assert!(!issues.has_issues());
}

#[test]
fn test_read_source_file_or_skip_valid_no_issues() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "content").unwrap();

    // Test with no issues tracker
    let content = read_source_file_or_skip(file.path(), None);

    assert!(content.is_some());
}

#[test]
fn test_read_source_file_or_skip_binary() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&[0x00, 0x01, 0x02]).unwrap();

    let mut issues = EncodingIssues::new();
    let content = read_source_file_or_skip(file.path(), Some(&mut issues));

    assert!(content.is_none());
    assert!(issues.has_issues());
    assert_eq!(issues.binary_files.len(), 1);
}

#[test]
fn test_read_source_file_or_skip_lossy() {
    let mut file = NamedTempFile::new().unwrap();
    // Invalid UTF-8
    file.write_all(&[0x80, 0x81, 0x82]).unwrap();

    let mut issues = EncodingIssues::new();
    let content = read_source_file_or_skip(file.path(), Some(&mut issues));

    // Lossy files still return content
    assert!(content.is_some());
    assert!(issues.has_issues());
    assert_eq!(issues.lossy_files.len(), 1);
}

#[test]
fn test_read_source_file_or_skip_not_found() {
    let mut issues = EncodingIssues::new();
    let content = read_source_file_or_skip(Path::new("/nonexistent/file.txt"), Some(&mut issues));

    // Not found returns None
    assert!(content.is_none());
    // No issues recorded for file not found (IO error path)
    assert!(!issues.has_issues());
}

// =============================================================================
// is_binary_file Tests
// =============================================================================

#[test]
fn test_is_binary_file_true() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&[0x00, 0x01, 0x02]).unwrap();

    let result = is_binary_file(file.path()).unwrap();

    assert!(result);
}

#[test]
fn test_is_binary_file_false() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "This is a text file").unwrap();

    let result = is_binary_file(file.path()).unwrap();

    assert!(!result);
}

#[test]
fn test_is_binary_file_empty() {
    let file = NamedTempFile::new().unwrap();

    let result = is_binary_file(file.path()).unwrap();

    // Empty file is not binary
    assert!(!result);
}

#[test]
fn test_is_binary_file_large_text() {
    let mut file = NamedTempFile::new().unwrap();
    // Write 16KB of text (larger than 8KB buffer)
    let text = "a".repeat(16384);
    write!(file, "{}", text).unwrap();

    let result = is_binary_file(file.path()).unwrap();

    assert!(!result);
}

#[test]
fn test_is_binary_file_null_after_8kb() {
    let mut file = NamedTempFile::new().unwrap();
    // Write exactly 8KB then null byte
    let text = "a".repeat(8192);
    file.write_all(text.as_bytes()).unwrap();
    file.write_all(&[0x00]).unwrap();

    let result = is_binary_file(file.path()).unwrap();

    // Only checks first 8KB, so null after is not detected
    // Note: This is actually checking 8192 bytes, so the null at position 8192
    // is at the boundary. With 8192 bytes + 1 null, reader.read returns 8192 first.
    // Let's verify actual behavior
    assert!(!result); // Should be false since null is at position 8192+
}

#[test]
fn test_is_binary_file_not_found() {
    let result = is_binary_file(Path::new("/nonexistent/file"));

    assert!(result.is_err());
}

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_encoding_workflow_full() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "import os\n\ndef main():\n    pass").unwrap();

    let mut issues = EncodingIssues::new();
    let content = read_source_file_or_skip(file.path(), Some(&mut issues));

    assert!(content.is_some());
    assert_eq!(content.unwrap(), "import os\n\ndef main():\n    pass");
    assert!(!issues.has_issues());
}

#[test]
fn test_encoding_workflow_mixed_files() {
    // Create multiple temp files
    let mut text_file = NamedTempFile::new().unwrap();
    write!(text_file, "print('hello')").unwrap();

    let mut binary_file = NamedTempFile::new().unwrap();
    binary_file.write_all(&[0x00, 0x01, 0x02]).unwrap();

    let mut invalid_utf8 = NamedTempFile::new().unwrap();
    invalid_utf8.write_all(&[0x80, 0x81]).unwrap();

    let mut issues = EncodingIssues::new();

    // Process all files
    let text_content = read_source_file_or_skip(text_file.path(), Some(&mut issues));
    let binary_content = read_source_file_or_skip(binary_file.path(), Some(&mut issues));
    let lossy_content = read_source_file_or_skip(invalid_utf8.path(), Some(&mut issues));

    // Verify results
    assert!(text_content.is_some());
    assert!(binary_content.is_none());
    assert!(lossy_content.is_some());

    // Verify issues
    assert!(issues.has_issues());
    assert_eq!(issues.total(), 2); // 1 binary + 1 lossy
    assert_eq!(issues.binary_files.len(), 1);
    assert_eq!(issues.lossy_files.len(), 1);
}

#[test]
fn test_bom_detection_edge_cases() {
    // Partial BOM (only first byte)
    let mut partial_bom = NamedTempFile::new().unwrap();
    partial_bom.write_all(&[0xEF]).unwrap();
    partial_bom.write_all(b"content").unwrap();

    let result = read_source_file(partial_bom.path()).unwrap();
    // Partial BOM is just invalid UTF-8
    assert!(matches!(result, FileReadResult::Lossy { .. }));
}

#[test]
fn test_bom_detection_partial_second_byte() {
    // Partial BOM (first two bytes)
    let mut partial_bom = NamedTempFile::new().unwrap();
    partial_bom.write_all(&[0xEF, 0xBB]).unwrap();
    partial_bom.write_all(b"content").unwrap();

    let result = read_source_file(partial_bom.path()).unwrap();
    // Still invalid UTF-8
    assert!(matches!(result, FileReadResult::Lossy { .. }));
}
