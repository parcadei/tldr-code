//! Lines of Code (LOC) analysis module (Session 15, Phase 2)
//!
//! This module provides language-aware line counting with breakdowns by type:
//! - Code lines: Lines containing executable code
//! - Comment lines: Lines containing only comments
//! - Blank lines: Empty lines or lines with only whitespace
//!
//! # Invariants
//!
//! - `code_lines + comment_lines + blank_lines == total_lines`
//! - Binary files are skipped with warning
//! - Files > MAX_FILE_SIZE are skipped with warning
//!
//! # Supported Languages
//!
//! - Python: `#` single-line, `"""` / `'''` multi-line (docstrings)
//! - Rust: `//` single-line, `/* */` multi-line
//! - Go: `//` single-line, `/* */` multi-line
//! - JavaScript/TypeScript: `//` single-line, `/* */` multi-line
//! - Java: `//` single-line, `/* */` multi-line
//! - C/C++: `//` single-line, `/* */` multi-line
//! - Ruby: `#` single-line, `=begin` / `=end` multi-line
//!
//! # References
//!
//! - Spec: session15/spec.md Section 1 (LOC Command)
//! - Phased Plan: session15/phased-plan.yaml Phase 2

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::metrics::file_utils::{
    check_file_size, has_binary_extension, is_binary_file, should_exclude, should_skip_path,
    DEFAULT_MAX_FILE_SIZE_MB,
};
use crate::metrics::types::LocInfo;
use crate::types::Language;
use crate::TldrError;

// =============================================================================
// Public Types
// =============================================================================

/// LOC analysis report for a directory or file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocReport {
    /// Summary totals across all files
    pub summary: LocSummary,
    /// Breakdown by language
    pub by_language: Vec<LanguageLocEntry>,
    /// Per-file breakdown (optional, populated with --by-file)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_file: Option<Vec<FileLocEntry>>,
    /// Per-directory breakdown (optional, populated with --by-dir)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_directory: Option<Vec<DirectoryLocEntry>>,
    /// Warnings encountered during analysis
    pub warnings: Vec<String>,
}

/// Summary totals for LOC analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocSummary {
    /// Total files analyzed
    pub total_files: usize,
    /// Total lines across all files
    pub total_lines: usize,
    /// Total code lines
    pub code_lines: usize,
    /// Total comment lines
    pub comment_lines: usize,
    /// Total blank lines
    pub blank_lines: usize,
    /// Code percentage (0.0 - 100.0)
    pub code_percent: f64,
    /// Comment percentage (0.0 - 100.0)
    pub comment_percent: f64,
    /// Blank percentage (0.0 - 100.0)
    pub blank_percent: f64,
}

impl LocSummary {
    /// Create a summary from totals.
    pub fn from_totals(
        total_files: usize,
        code_lines: usize,
        comment_lines: usize,
        blank_lines: usize,
    ) -> Self {
        let total_lines = code_lines + comment_lines + blank_lines;
        let (code_percent, comment_percent, blank_percent) = if total_lines == 0 {
            (0.0, 0.0, 0.0)
        } else {
            (
                (code_lines as f64 / total_lines as f64) * 100.0,
                (comment_lines as f64 / total_lines as f64) * 100.0,
                (blank_lines as f64 / total_lines as f64) * 100.0,
            )
        };

        Self {
            total_files,
            total_lines,
            code_lines,
            comment_lines,
            blank_lines,
            code_percent,
            comment_percent,
            blank_percent,
        }
    }
}

/// LOC entry for a single language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageLocEntry {
    /// Language name
    pub language: String,
    /// Number of files
    pub files: usize,
    /// Code lines
    pub code_lines: usize,
    /// Comment lines
    pub comment_lines: usize,
    /// Blank lines
    pub blank_lines: usize,
    /// Total lines
    pub total_lines: usize,
}

/// LOC entry for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileLocEntry {
    /// File path (relative)
    pub path: PathBuf,
    /// Detected language
    pub language: String,
    /// Code lines
    pub code_lines: usize,
    /// Comment lines
    pub comment_lines: usize,
    /// Blank lines
    pub blank_lines: usize,
    /// Total lines
    pub total_lines: usize,
}

/// LOC entry for a directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryLocEntry {
    /// Directory path (relative)
    pub path: PathBuf,
    /// Code lines
    pub code_lines: usize,
    /// Comment lines
    pub comment_lines: usize,
    /// Blank lines
    pub blank_lines: usize,
    /// Total lines
    pub total_lines: usize,
}

/// Options for LOC analysis.
#[derive(Debug, Clone, Default)]
pub struct LocOptions {
    /// Filter to specific language
    pub lang: Option<Language>,
    /// Include per-file breakdown
    pub by_file: bool,
    /// Include per-directory breakdown
    pub by_dir: bool,
    /// Exclude patterns (glob syntax)
    pub exclude: Vec<String>,
    /// Include hidden files
    pub include_hidden: bool,
    /// Respect .gitignore (default: true)
    pub gitignore: bool,
    /// Maximum files to process (0 = unlimited)
    pub max_files: usize,
    /// Maximum file size in MB (default: 10)
    pub max_file_size_mb: usize,
}

impl LocOptions {
    /// Create default options.
    pub fn new() -> Self {
        Self {
            gitignore: true,
            max_file_size_mb: DEFAULT_MAX_FILE_SIZE_MB,
            ..Default::default()
        }
    }
}

// =============================================================================
// Line Classification
// =============================================================================

/// State machine state for multi-line comment/string tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    /// Normal code state
    Normal,
    /// Inside a multi-line comment
    InMultiLineComment,
    /// Inside a Python triple-quoted string (docstring)
    InTripleQuotedString(TripleQuoteType),
}

/// Type of triple quote in Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TripleQuoteType {
    /// Triple double quotes """
    Double,
    /// Triple single quotes '''
    Single,
}

/// Classify a single line given the language and current state.
///
/// Returns the line type and the new parse state.
fn classify_line(line: &str, lang: Language, state: ParseState) -> (LineType, ParseState) {
    let trimmed = line.trim();

    // Handle blank lines
    if trimmed.is_empty() {
        return (LineType::Blank, state);
    }

    // Handle state-dependent parsing
    match state {
        ParseState::InMultiLineComment => classify_in_multiline_comment(trimmed, lang),
        ParseState::InTripleQuotedString(quote_type) => {
            classify_in_triple_quoted_string(trimmed, quote_type)
        }
        ParseState::Normal => classify_normal_line(trimmed, lang),
    }
}

/// Classify a line when in a multi-line comment.
fn classify_in_multiline_comment(trimmed: &str, lang: Language) -> (LineType, ParseState) {
    let end_marker = match lang {
        Language::Ruby => "=end",
        _ => "*/",
    };

    if trimmed.contains(end_marker) {
        // Check if there's code after the comment ends
        let after_end = match lang {
            Language::Ruby => {
                if let Some(pos) = trimmed.find(end_marker) {
                    let after = trimmed[pos + end_marker.len()..].trim();
                    !after.is_empty() && !is_single_line_comment(after, lang)
                } else {
                    false
                }
            }
            _ => {
                if let Some(pos) = trimmed.find(end_marker) {
                    let after = trimmed[pos + end_marker.len()..].trim();
                    !after.is_empty() && !is_single_line_comment(after, lang)
                } else {
                    false
                }
            }
        };

        if after_end {
            // Line has code after comment ends
            (LineType::Code, ParseState::Normal)
        } else {
            // Pure comment line
            (LineType::Comment, ParseState::Normal)
        }
    } else {
        // Still inside multi-line comment
        (LineType::Comment, ParseState::InMultiLineComment)
    }
}

/// Classify a line when inside a Python triple-quoted string.
fn classify_in_triple_quoted_string(
    trimmed: &str,
    quote_type: TripleQuoteType,
) -> (LineType, ParseState) {
    let marker = match quote_type {
        TripleQuoteType::Double => "\"\"\"",
        TripleQuoteType::Single => "'''",
    };

    // Check if the string ends on this line
    // We need to find the closing marker, but not at position 0 (which would be the opener)
    // unless the line is just the closing marker
    if trimmed == marker {
        // Just the closing marker
        (LineType::Comment, ParseState::Normal)
    } else if trimmed.ends_with(marker) {
        // Ends with the marker, check if it's part of an opening on same line
        let without_end = &trimmed[..trimmed.len() - 3];
        if without_end.contains(marker) {
            // Opening and closing on same line - already handled in normal
            (LineType::Comment, ParseState::Normal)
        } else {
            // Just the closing
            (LineType::Comment, ParseState::Normal)
        }
    } else if trimmed.contains(marker) {
        // Contains marker somewhere (closing)
        let pos = trimmed.find(marker).unwrap();
        let after = trimmed[pos + 3..].trim();
        if after.is_empty() || is_single_line_comment(after, Language::Python) {
            (LineType::Comment, ParseState::Normal)
        } else {
            // Code after the string
            (LineType::Code, ParseState::Normal)
        }
    } else {
        // Still inside triple-quoted string (treated as comment/docstring)
        (
            LineType::Comment,
            ParseState::InTripleQuotedString(quote_type),
        )
    }
}

/// Classify a line in normal state.
fn classify_normal_line(trimmed: &str, lang: Language) -> (LineType, ParseState) {
    // Check for Python triple-quoted strings first
    if lang == Language::Python {
        // Check for triple-quoted string start
        if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
            let quote_type = if trimmed.starts_with("\"\"\"") {
                TripleQuoteType::Double
            } else {
                TripleQuoteType::Single
            };
            let marker = if quote_type == TripleQuoteType::Double {
                "\"\"\""
            } else {
                "'''"
            };

            // Check if it closes on the same line
            let rest = &trimmed[3..];
            if rest.contains(marker) {
                // Opens and closes on same line - it's a docstring/comment
                let after_close_pos = rest.find(marker).unwrap() + 3;
                let after = rest[after_close_pos..].trim();
                if after.is_empty() || is_single_line_comment(after, lang) {
                    return (LineType::Comment, ParseState::Normal);
                } else {
                    return (LineType::Code, ParseState::Normal);
                }
            } else {
                // Opens but doesn't close
                return (
                    LineType::Comment,
                    ParseState::InTripleQuotedString(quote_type),
                );
            }
        }
    }

    // Check for single-line comments
    if is_single_line_comment(trimmed, lang) {
        return (LineType::Comment, ParseState::Normal);
    }

    // Check for multi-line comment start
    let (start_marker, end_marker) = match lang {
        Language::Ruby => ("=begin", "=end"),
        Language::Python => ("", ""), // Python uses triple quotes, handled above
        _ => ("/*", "*/"),
    };

    if !start_marker.is_empty() && trimmed.starts_with(start_marker) {
        // Multi-line comment start
        if trimmed.contains(end_marker) && trimmed.find(end_marker) > trimmed.find(start_marker) {
            // Opens and closes on same line
            let after_close_pos = trimmed.find(end_marker).unwrap() + end_marker.len();
            let after = trimmed[after_close_pos..].trim();
            if after.is_empty() || is_single_line_comment(after, lang) {
                return (LineType::Comment, ParseState::Normal);
            } else {
                return (LineType::Code, ParseState::Normal);
            }
        } else {
            // Opens but doesn't close
            return (LineType::Comment, ParseState::InMultiLineComment);
        }
    }

    // Check for inline multi-line comment (e.g., code /* comment */ more code)
    if !start_marker.is_empty() && trimmed.contains(start_marker) {
        // Line has code with embedded comment
        return (LineType::Code, ParseState::Normal);
    }

    // Default: it's code
    (LineType::Code, ParseState::Normal)
}

/// Check if a line (trimmed) is a single-line comment.
fn is_single_line_comment(trimmed: &str, lang: Language) -> bool {
    match lang {
        Language::Python | Language::Ruby => trimmed.starts_with('#'),
        Language::Rust
        | Language::Go
        | Language::TypeScript
        | Language::JavaScript
        | Language::Java
        | Language::C
        | Language::Cpp
        | Language::Swift
        | Language::Kotlin
        | Language::CSharp
        | Language::Scala
        | Language::Php => trimmed.starts_with("//"),
        Language::Lua | Language::Luau => trimmed.starts_with("--"),
        Language::Elixir => trimmed.starts_with('#'),
        Language::Ocaml => trimmed.starts_with("(*") || trimmed.starts_with('*'),
    }
}

/// Line classification result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineType {
    Code,
    Comment,
    Blank,
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Count lines of code in a source string.
///
/// Returns LocInfo with code, comment, and blank line counts.
///
/// # Arguments
///
/// * `source` - Source code string
/// * `lang` - Programming language for comment detection
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::metrics::loc::count_lines;
/// use tldr_core::Language;
///
/// let source = "def foo():\n    # comment\n    pass\n";
/// let info = count_lines(source, Language::Python);
/// assert_eq!(info.code_lines, 2);
/// assert_eq!(info.comment_lines, 1);
/// ```
pub fn count_lines(source: &str, lang: Language) -> LocInfo {
    let mut code_lines = 0;
    let mut comment_lines = 0;
    let mut blank_lines = 0;
    let mut state = ParseState::Normal;

    for line in source.lines() {
        let (line_type, new_state) = classify_line(line, lang, state);
        state = new_state;

        match line_type {
            LineType::Code => code_lines += 1,
            LineType::Comment => comment_lines += 1,
            LineType::Blank => blank_lines += 1,
        }
    }

    LocInfo::new(code_lines, comment_lines, blank_lines)
}

/// Analyze a single file for LOC.
///
/// Returns LocInfo or error if file cannot be read.
///
/// # Arguments
///
/// * `path` - Path to the file
/// * `lang` - Optional language override (auto-detect if None)
/// * `max_file_size_mb` - Maximum file size in MB
///
/// # Errors
///
/// Returns `TldrError` if:
/// - File not found
/// - File too large
/// - File is binary
/// - I/O error reading file
pub fn analyze_file(
    path: &Path,
    lang: Option<Language>,
    max_file_size_mb: usize,
) -> Result<(LocInfo, Language), TldrError> {
    // Check file exists
    if !path.exists() {
        return Err(TldrError::PathNotFound(path.to_path_buf()));
    }

    // Check file size
    check_file_size(path, max_file_size_mb)?;

    // Check for binary
    if has_binary_extension(path) || is_binary_file(path) {
        return Err(TldrError::UnsupportedLanguage(format!(
            "Binary file: {}",
            path.display()
        )));
    }

    // Detect language
    let detected_lang = lang.or_else(|| Language::from_path(path));
    let language = match detected_lang {
        Some(l) => l,
        None => {
            return Err(TldrError::UnsupportedLanguage(format!(
                "{}",
                path.display()
            )))
        }
    };

    // Read file contents
    let source = std::fs::read_to_string(path)?;

    // Count lines
    let info = count_lines(&source, language);

    Ok((info, language))
}

/// Analyze a directory for LOC.
///
/// Recursively walks the directory, analyzing supported files.
///
/// # Arguments
///
/// * `path` - Path to the directory
/// * `options` - Analysis options
///
/// # Returns
///
/// Returns `LocReport` with summary, language breakdown, and optional per-file/per-directory data.
///
/// # Errors
///
/// Returns `TldrError` if path doesn't exist or is not a directory.
pub fn analyze_directory(path: &Path, options: &LocOptions) -> Result<LocReport, TldrError> {
    if !path.exists() {
        return Err(TldrError::PathNotFound(path.to_path_buf()));
    }

    let mut by_language: HashMap<Language, (usize, LocInfo)> = HashMap::new(); // (file_count, loc_info)
    let mut by_file: Vec<FileLocEntry> = Vec::new();
    let mut by_directory: HashMap<PathBuf, LocInfo> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut files_processed = 0;

    // Build walker with options
    let mut builder = ignore::WalkBuilder::new(path);
    builder.follow_links(false); // CM-1: Don't follow symlinks
    builder.hidden(!options.include_hidden);

    // Handle gitignore
    if options.gitignore {
        builder.git_ignore(true);
        builder.git_global(true);
    } else {
        builder.git_ignore(false);
        builder.git_global(false);
    }

    // Walk directory
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("Walk error: {}", e));
                continue;
            }
        };

        let entry_path = entry.path();

        // Skip directories
        if entry_path.is_dir() {
            continue;
        }

        // Check max files limit
        if options.max_files > 0 && files_processed >= options.max_files {
            warnings.push(format!(
                "Stopped after {} files (max_files limit)",
                options.max_files
            ));
            break;
        }

        // Get relative path for pattern checking
        let relative_path = entry_path.strip_prefix(path).unwrap_or(entry_path);

        // Skip paths matching patterns (using relative path to avoid skipping
        // hidden temp directories in absolute path)
        if should_skip_path(relative_path) {
            continue;
        }
        if should_exclude(relative_path, &options.exclude) {
            continue;
        }

        // Detect language
        let lang = match Language::from_path(entry_path) {
            Some(l) => l,
            None => continue, // Skip unsupported files
        };

        // Filter by language if specified
        if let Some(filter_lang) = options.lang {
            if lang != filter_lang {
                continue;
            }
        }

        // Analyze file
        match analyze_file(entry_path, Some(lang), options.max_file_size_mb) {
            Ok((info, detected_lang)) => {
                files_processed += 1;

                // Update language totals
                let entry = by_language
                    .entry(detected_lang)
                    .or_insert((0, LocInfo::default()));
                entry.0 += 1;
                entry.1.merge(&info);

                // Update per-file if requested
                if options.by_file {
                    by_file.push(FileLocEntry {
                        path: relative_path.to_path_buf(),
                        language: detected_lang.as_str().to_string(),
                        code_lines: info.code_lines,
                        comment_lines: info.comment_lines,
                        blank_lines: info.blank_lines,
                        total_lines: info.total_lines,
                    });
                }

                // Update per-directory if requested
                if options.by_dir {
                    if let Some(parent) = relative_path.parent() {
                        let dir_path = if parent.as_os_str().is_empty() {
                            PathBuf::from(".")
                        } else {
                            parent.to_path_buf()
                        };
                        let dir_entry = by_directory.entry(dir_path).or_default();
                        dir_entry.merge(&info);
                    }
                }
            }
            Err(TldrError::FileTooLarge {
                path,
                size_mb,
                max_mb,
            }) => {
                warnings.push(format!(
                    "Skipped large file: {} ({}MB > {}MB)",
                    path.display(),
                    size_mb,
                    max_mb
                ));
            }
            Err(TldrError::UnsupportedLanguage(msg)) if msg.contains("Binary file") => {
                warnings.push(format!("Skipped binary file: {}", entry_path.display()));
            }
            Err(TldrError::UnsupportedLanguage(_)) => {
                // Skip unsupported language files silently
            }
            Err(e) => {
                warnings.push(format!("Error reading {}: {}", entry_path.display(), e));
            }
        }
    }

    // Build language breakdown
    let mut by_language_vec: Vec<LanguageLocEntry> = by_language
        .into_iter()
        .map(|(lang, (count, info))| LanguageLocEntry {
            language: lang.as_str().to_string(),
            files: count,
            code_lines: info.code_lines,
            comment_lines: info.comment_lines,
            blank_lines: info.blank_lines,
            total_lines: info.total_lines,
        })
        .collect();

    // Sort by total lines descending
    by_language_vec.sort_by(|a, b| b.total_lines.cmp(&a.total_lines));

    // Calculate summary
    let total_code: usize = by_language_vec.iter().map(|e| e.code_lines).sum();
    let total_comment: usize = by_language_vec.iter().map(|e| e.comment_lines).sum();
    let total_blank: usize = by_language_vec.iter().map(|e| e.blank_lines).sum();
    let total_files: usize = by_language_vec.iter().map(|e| e.files).sum();

    let summary = LocSummary::from_totals(total_files, total_code, total_comment, total_blank);

    // Build directory breakdown if requested
    let by_directory_vec = if options.by_dir {
        let mut vec: Vec<DirectoryLocEntry> = by_directory
            .into_iter()
            .map(|(path, info)| DirectoryLocEntry {
                path,
                code_lines: info.code_lines,
                comment_lines: info.comment_lines,
                blank_lines: info.blank_lines,
                total_lines: info.total_lines,
            })
            .collect();
        vec.sort_by(|a, b| b.total_lines.cmp(&a.total_lines));
        Some(vec)
    } else {
        None
    };

    Ok(LocReport {
        summary,
        by_language: by_language_vec,
        by_file: if options.by_file { Some(by_file) } else { None },
        by_directory: by_directory_vec,
        warnings,
    })
}

/// Analyze a path (file or directory).
///
/// Convenience function that dispatches to `analyze_file` or `analyze_directory`.
pub fn analyze_loc(path: &Path, options: &LocOptions) -> Result<LocReport, TldrError> {
    if path.is_file() {
        // Single file analysis
        let (info, lang) = analyze_file(path, options.lang, options.max_file_size_mb)?;

        let summary =
            LocSummary::from_totals(1, info.code_lines, info.comment_lines, info.blank_lines);

        let by_language = vec![LanguageLocEntry {
            language: lang.as_str().to_string(),
            files: 1,
            code_lines: info.code_lines,
            comment_lines: info.comment_lines,
            blank_lines: info.blank_lines,
            total_lines: info.total_lines,
        }];

        let by_file = if options.by_file {
            Some(vec![FileLocEntry {
                path: path.to_path_buf(),
                language: lang.as_str().to_string(),
                code_lines: info.code_lines,
                comment_lines: info.comment_lines,
                blank_lines: info.blank_lines,
                total_lines: info.total_lines,
            }])
        } else {
            None
        };

        Ok(LocReport {
            summary,
            by_language,
            by_file,
            by_directory: None,
            warnings: vec![],
        })
    } else {
        analyze_directory(path, options)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Line Counting Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_count_lines_python_simple() {
        let source = r#"# Comment
def foo():
    pass
"#;
        let info = count_lines(source, Language::Python);
        assert_eq!(info.code_lines, 2);
        assert_eq!(info.comment_lines, 1);
        assert_eq!(info.blank_lines, 0);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_python_docstring() {
        let source = r#""""Module docstring."""

def foo():
    """Function docstring."""
    pass
"#;
        let info = count_lines(source, Language::Python);
        // Docstrings are comments: line 1 is docstring (1 line, opens and closes)
        // line 2 is blank
        // line 3 is code (def foo())
        // line 4 is docstring
        // line 5 is code (pass)
        assert_eq!(info.comment_lines, 2);
        assert_eq!(info.blank_lines, 1);
        assert_eq!(info.code_lines, 2);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_python_multiline_docstring() {
        let source = r#""""
Multi-line
docstring
"""
def foo():
    pass
"#;
        let info = count_lines(source, Language::Python);
        // Lines 1-4 are docstring (comment)
        // Line 5 is code
        // Line 6 is code
        assert_eq!(info.comment_lines, 4);
        assert_eq!(info.code_lines, 2);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_rust_simple() {
        let source = r#"// Comment
fn main() {
    println!("Hello");
}
"#;
        let info = count_lines(source, Language::Rust);
        assert_eq!(info.code_lines, 3);
        assert_eq!(info.comment_lines, 1);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_rust_multiline_comment() {
        let source = r#"/* Multi
   line
   comment */
fn main() {
    /* inline */ let x = 1;
}
"#;
        let info = count_lines(source, Language::Rust);
        // Lines 1-3: comment
        // Line 4: code
        // Line 5: code (has inline comment but also has code)
        // Line 6: code
        assert_eq!(info.comment_lines, 3);
        assert_eq!(info.code_lines, 3);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_empty() {
        let source = "";
        let info = count_lines(source, Language::Python);
        assert_eq!(info.code_lines, 0);
        assert_eq!(info.comment_lines, 0);
        assert_eq!(info.blank_lines, 0);
        assert_eq!(info.total_lines, 0);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_blank_only() {
        let source = "\n\n\n";
        let info = count_lines(source, Language::Python);
        assert_eq!(info.blank_lines, 3);
        assert_eq!(info.code_lines, 0);
        assert_eq!(info.comment_lines, 0);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_javascript() {
        let source = r#"// Single line comment
/*
 * Multi-line
 */
function hello() {
    console.log("hi");
}
"#;
        let info = count_lines(source, Language::JavaScript);
        assert_eq!(info.comment_lines, 4);
        assert_eq!(info.code_lines, 3);
        assert!(info.is_valid());
    }

    #[test]
    fn test_count_lines_go() {
        let source = r#"// Package main
package main

import "fmt"

// main is the entry point
func main() {
    fmt.Println("Hello")
}
"#;
        let info = count_lines(source, Language::Go);
        assert_eq!(info.comment_lines, 2);
        assert_eq!(info.blank_lines, 2);
        assert_eq!(info.code_lines, 5);
        assert!(info.is_valid());
    }

    #[test]
    fn test_invariant_holds() {
        let source = r#"# Comment
def foo():
    """Docstring"""
    # Another comment
    pass

# End
"#;
        let info = count_lines(source, Language::Python);
        assert!(info.is_valid());
        assert_eq!(
            info.code_lines + info.comment_lines + info.blank_lines,
            info.total_lines
        );
    }

    // -------------------------------------------------------------------------
    // Classification Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_classify_python_hash_comment() {
        let (line_type, state) =
            classify_line("# This is a comment", Language::Python, ParseState::Normal);
        assert_eq!(line_type, LineType::Comment);
        assert_eq!(state, ParseState::Normal);
    }

    #[test]
    fn test_classify_rust_slash_comment() {
        let (line_type, state) =
            classify_line("// This is a comment", Language::Rust, ParseState::Normal);
        assert_eq!(line_type, LineType::Comment);
        assert_eq!(state, ParseState::Normal);
    }

    #[test]
    fn test_classify_blank_line() {
        let (line_type, _) = classify_line("   ", Language::Python, ParseState::Normal);
        assert_eq!(line_type, LineType::Blank);
    }

    #[test]
    fn test_classify_code_line() {
        let (line_type, _) = classify_line("let x = 5;", Language::Rust, ParseState::Normal);
        assert_eq!(line_type, LineType::Code);
    }
}
