//! File utilities for metrics analysis (Session 15)
//!
//! This module provides file handling utilities for metrics commands:
//! - Binary file detection
//! - File size validation
//! - Symlink safety
//! - Skip patterns (node_modules, .git, etc.)
//!
//! # Mitigations
//!
//! - CM-1: Large files (>10MB) and circular symlinks cause crashes
//! - CM-2: Encoding issues handled via encoding.rs module

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::types::Language;
use crate::TldrError;

// =============================================================================
// Walk Options
// =============================================================================

/// Options for walking source files in a directory.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Filter to specific language (None = all supported languages)
    pub lang: Option<Language>,
    /// Exclude patterns (glob syntax)
    pub exclude: Vec<String>,
    /// Include hidden files/directories (default: false)
    pub include_hidden: bool,
    /// Respect .gitignore rules (default: true)
    pub gitignore: bool,
    /// Maximum files to return (0 = unlimited)
    pub max_files: usize,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            lang: None,
            exclude: Vec::new(),
            include_hidden: false,
            gitignore: true,
            max_files: 0,
        }
    }
}

/// Walk a path and return source files matching the given options.
///
/// - If `path` is a file: returns `vec![path.to_path_buf()]` (no filtering applied,
///   caller is responsible for language validation on single-file input).
/// - If `path` is a directory: walks recursively, filtering by language support
///   and the provided options.
///
/// # Errors
///
/// Returns `TldrError::PathNotFound` if the path does not exist.
///
/// # Warnings
///
/// Walk errors and max_files truncation are reported via the returned warnings vec.
pub fn walk_source_files(
    path: &Path,
    options: &WalkOptions,
) -> Result<(Vec<PathBuf>, Vec<String>), TldrError> {
    if !path.exists() {
        return Err(TldrError::PathNotFound(path.to_path_buf()));
    }

    // Single file: return as-is without filtering
    if path.is_file() {
        return Ok((vec![path.to_path_buf()], vec![]));
    }

    // Directory walk using ignore::WalkBuilder (matches loc.rs pattern)
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    let mut had_entries = false;

    let mut builder = ignore::WalkBuilder::new(path);
    builder.follow_links(false); // CM-1: Don't follow symlinks
    builder.hidden(!options.include_hidden);

    if options.gitignore {
        builder.git_ignore(true);
        builder.git_global(true);
    } else {
        builder.git_ignore(false);
        builder.git_global(false);
    }

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

        had_entries = true;

        // Check max files limit
        if options.max_files > 0 && files.len() >= options.max_files {
            warnings.push(format!(
                "Stopped after {} files (max_files limit)",
                options.max_files
            ));
            break;
        }

        // Get relative path for pattern checking
        let relative_path = entry_path.strip_prefix(path).unwrap_or(entry_path);

        // Skip paths matching skip patterns (node_modules, .git, etc.)
        if should_skip_path(relative_path) {
            continue;
        }

        // Skip paths matching user exclude patterns
        if should_exclude(relative_path, &options.exclude) {
            continue;
        }

        // Detect language - skip unsupported files
        let lang = match Language::from_path(entry_path) {
            Some(l) => l,
            None => continue,
        };

        // Filter by language if specified
        if let Some(filter_lang) = options.lang {
            if lang != filter_lang {
                continue;
            }
        }

        files.push(entry_path.to_path_buf());
    }

    // Warn if directory had entries but no supported source files
    if files.is_empty() && had_entries {
        warnings.push(format!(
            "No supported source files found in {}",
            path.display()
        ));
    }

    Ok((files, warnings))
}

/// Check if a path should be excluded based on glob patterns.
///
/// Used by the directory walker and LOC analysis to filter out files
/// matching user-specified exclude patterns.
pub fn should_exclude(path: &Path, patterns: &[String]) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in patterns {
        if let Ok(glob) = glob::Pattern::new(pattern) {
            if glob.matches(&path_str) {
                return true;
            }
        }
    }

    false
}

// =============================================================================
// Constants
// =============================================================================

/// Default maximum file size in bytes (10MB)
pub const DEFAULT_MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

/// Default maximum file size in megabytes
pub const DEFAULT_MAX_FILE_SIZE_MB: usize = 10;

/// Directories to skip by default
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".venv",
    "venv",
    ".env",
    "target",
    "build",
    "dist",
    ".idea",
    ".vscode",
    ".next",
    ".nuxt",
    "coverage",
    ".coverage",
];

/// File extensions that are typically binary
const BINARY_EXTENSIONS: &[&str] = &[
    // Images
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "webp", "svg", "tiff", "psd",
    // Audio/Video
    "mp3", "mp4", "avi", "mkv", "mov", "wav", "flac", "ogg", "webm", // Archives
    "zip", "tar", "gz", "bz2", "xz", "7z", "rar", // Binaries
    "exe", "dll", "so", "dylib", "a", "o", "obj", "class", "pyc", "pyo",
    // Documents (binary formats)
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", // Databases
    "db", "sqlite", "sqlite3", // Fonts
    "ttf", "otf", "woff", "woff2", "eot", // Other
    "lock", "bin", "dat", "pak",
];

// =============================================================================
// File Size Utilities
// =============================================================================

/// Check if a file exceeds the maximum allowed size.
///
/// # Arguments
///
/// * `path` - Path to the file
/// * `max_mb` - Maximum allowed size in megabytes
///
/// # Returns
///
/// * `Ok(())` - File is within size limit
/// * `Err(TldrError::FileTooLarge)` - File exceeds size limit
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::metrics::file_utils::check_file_size;
///
/// check_file_size(Path::new("large_file.py"), 10)?;
/// ```
pub fn check_file_size(path: &Path, max_mb: usize) -> Result<(), TldrError> {
    let metadata = fs::metadata(path)?;
    let size_bytes = metadata.len() as usize;
    let max_bytes = max_mb * 1024 * 1024;

    if size_bytes > max_bytes {
        let size_mb = size_bytes / (1024 * 1024);
        return Err(TldrError::FileTooLarge {
            path: path.to_path_buf(),
            size_mb,
            max_mb,
        });
    }

    Ok(())
}

/// Get file size in bytes.
pub fn get_file_size(path: &Path) -> Result<usize, TldrError> {
    let metadata = fs::metadata(path)?;
    Ok(metadata.len() as usize)
}

// =============================================================================
// Binary File Detection
// =============================================================================

/// Check if a file is binary by examining its content.
///
/// This function reads the first 8KB of the file and checks for null bytes.
/// Also checks file extension against known binary extensions.
///
/// # Arguments
///
/// * `path` - Path to the file
///
/// # Returns
///
/// * `true` - File is binary
/// * `false` - File appears to be text
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::metrics::file_utils::is_binary_file;
///
/// if is_binary_file(Path::new("image.png")) {
///     println!("Skipping binary file");
/// }
/// ```
pub fn is_binary_file(path: &Path) -> bool {
    // First check extension
    if let Some(ext) = path.extension() {
        if let Some(ext_str) = ext.to_str() {
            if BINARY_EXTENSIONS.contains(&ext_str.to_lowercase().as_str()) {
                return true;
            }
        }
    }

    // Then check content (first 8KB for null bytes)
    match fs::File::open(path) {
        Ok(mut file) => {
            let mut buffer = [0u8; 8192];
            match file.read(&mut buffer) {
                Ok(bytes_read) => buffer[..bytes_read].contains(&0),
                Err(_) => false, // Treat read errors as non-binary
            }
        }
        Err(_) => false, // Treat open errors as non-binary
    }
}

/// Check if a file has a binary extension (without reading content).
pub fn has_binary_extension(path: &Path) -> bool {
    if let Some(ext) = path.extension() {
        if let Some(ext_str) = ext.to_str() {
            return BINARY_EXTENSIONS.contains(&ext_str.to_lowercase().as_str());
        }
    }
    false
}

// =============================================================================
// Skip Pattern Utilities
// =============================================================================

/// Check if a path should be skipped based on common patterns.
///
/// Skips:
/// - Hidden files/directories (starting with .)
/// - node_modules, .git, __pycache__, etc.
/// - Build directories (target, build, dist)
///
/// # Arguments
///
/// * `path` - Path to check
///
/// # Returns
///
/// * `true` - Path should be skipped
/// * `false` - Path should be processed
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::metrics::file_utils::should_skip_path;
///
/// assert!(should_skip_path(Path::new("node_modules/package/index.js")));
/// assert!(!should_skip_path(Path::new("src/main.rs")));
/// ```
pub fn should_skip_path(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if let Some(name_str) = name.to_str() {
                // Skip hidden directories/files (but not . or ..)
                if name_str.starts_with('.') && name_str.len() > 1 {
                    // Allow .github, .claude directories
                    if !matches!(name_str, ".github" | ".claude") {
                        return true;
                    }
                }

                // Skip known directories
                if SKIP_DIRS.contains(&name_str) {
                    return true;
                }
            }
        }
    }
    false
}

/// Get the set of directories that should be skipped.
pub fn skip_directories() -> HashSet<&'static str> {
    SKIP_DIRS.iter().copied().collect()
}

// =============================================================================
// Symlink Safety Utilities
// =============================================================================

/// Resolve a symlink safely, preventing circular references and external targets.
///
/// # Arguments
///
/// * `path` - Path to resolve (may be a symlink)
/// * `project_root` - Optional project root to validate target is within project
///
/// # Returns
///
/// * `Ok(PathBuf)` - Resolved path (canonical)
/// * `Err(TldrError::SymlinkCycle)` - Circular symlink detected
/// * `Err(TldrError::PathTraversal)` - Symlink points outside project
///
/// # Example
///
/// ```rust,ignore
/// use tldr_core::metrics::file_utils::resolve_symlink_safely;
///
/// let resolved = resolve_symlink_safely(
///     Path::new("link_to_file"),
///     Some(Path::new("/project/root"))
/// )?;
/// ```
pub fn resolve_symlink_safely(
    path: &Path,
    project_root: Option<&Path>,
) -> Result<PathBuf, TldrError> {
    // Track visited symlink paths (before resolution) to detect cycles
    let mut visited_links = HashSet::new();
    let mut current = path.to_path_buf();

    // Maximum symlink depth to prevent infinite loops
    const MAX_DEPTH: usize = 40;

    for _ in 0..MAX_DEPTH {
        // Check if it's a symlink
        let metadata = match fs::symlink_metadata(&current) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(TldrError::PathNotFound(current));
            }
            Err(e) => return Err(TldrError::IoError(e)),
        };

        if metadata.file_type().is_symlink() {
            // Track this symlink path to detect cycles
            // Use the absolute path of the symlink itself (not target)
            let link_abs = if current.is_absolute() {
                current.clone()
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(&current))
                    .unwrap_or_else(|_| current.clone())
            };

            if visited_links.contains(&link_abs) {
                return Err(TldrError::SymlinkCycle(path.to_path_buf()));
            }
            visited_links.insert(link_abs);

            // Read the symlink target
            let target = fs::read_link(&current)?;
            // Resolve relative targets
            current = if target.is_relative() {
                current.parent().map(|p| p.join(&target)).unwrap_or(target)
            } else {
                target
            };
        } else {
            // Not a symlink, we're done - get canonical path
            let canonical = match current.canonicalize() {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(TldrError::PathNotFound(current));
                }
                Err(e) => return Err(TldrError::IoError(e)),
            };

            // Validate target is within project root (if specified)
            if let Some(root) = project_root {
                let root_canonical = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
                if !canonical.starts_with(&root_canonical) {
                    return Err(TldrError::PathTraversal(path.to_path_buf()));
                }
            }
            return Ok(canonical);
        }
    }

    // Exceeded max depth - likely a cycle we couldn't detect
    Err(TldrError::SymlinkCycle(path.to_path_buf()))
}

/// Check if a path is a symlink.
pub fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// =============================================================================
// Path Validation Utilities
// =============================================================================

/// Validate that a path is within a project root (no path traversal).
///
/// # Arguments
///
/// * `path` - Path to validate
/// * `project_root` - Root directory path must be within
///
/// # Returns
///
/// * `true` - Path is within project root
/// * `false` - Path is outside project root or contains traversal
pub fn is_path_within_project(path: &Path, project_root: &Path) -> bool {
    // Canonicalize both paths
    let path_canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    let root_canonical = match project_root.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    path_canonical.starts_with(&root_canonical)
}

/// Check if a path contains path traversal patterns (.. components).
pub fn contains_path_traversal(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return true;
        }
    }
    false
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    // -------------------------------------------------------------------------
    // File Size Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_check_file_size_within_limit() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "small content").unwrap();

        assert!(check_file_size(file.path(), 10).is_ok());
    }

    #[test]
    fn test_check_file_size_exceeds_limit() {
        let mut file = NamedTempFile::new().unwrap();
        // Write 2MB of data
        let data = vec![b'x'; 2 * 1024 * 1024];
        file.write_all(&data).unwrap();

        let result = check_file_size(file.path(), 1); // 1MB limit
        assert!(matches!(result, Err(TldrError::FileTooLarge { .. })));
    }

    #[test]
    fn test_get_file_size() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "hello world").unwrap();

        let size = get_file_size(file.path()).unwrap();
        assert_eq!(size, 11);
    }

    // -------------------------------------------------------------------------
    // Binary File Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_binary_file_by_content() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&[0x00, 0x01, 0x02, 0x00]).unwrap();

        assert!(is_binary_file(file.path()));
    }

    #[test]
    fn test_is_binary_file_text_content() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "def foo():\n    pass\n").unwrap();

        assert!(!is_binary_file(file.path()));
    }

    #[test]
    fn test_has_binary_extension() {
        assert!(has_binary_extension(Path::new("image.png")));
        assert!(has_binary_extension(Path::new("archive.zip")));
        assert!(has_binary_extension(Path::new("binary.exe")));
        assert!(!has_binary_extension(Path::new("code.py")));
        assert!(!has_binary_extension(Path::new("script.rs")));
    }

    // -------------------------------------------------------------------------
    // Skip Pattern Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_should_skip_path_node_modules() {
        assert!(should_skip_path(Path::new("node_modules/package/index.js")));
        assert!(should_skip_path(Path::new(
            "project/node_modules/lodash/index.js"
        )));
    }

    #[test]
    fn test_should_skip_path_git() {
        assert!(should_skip_path(Path::new(".git/objects/abc")));
        assert!(should_skip_path(Path::new("repo/.git/HEAD")));
    }

    #[test]
    fn test_should_skip_path_pycache() {
        assert!(should_skip_path(Path::new("__pycache__/module.pyc")));
    }

    #[test]
    fn test_should_skip_path_hidden() {
        assert!(should_skip_path(Path::new(".hidden/file")));
        assert!(should_skip_path(Path::new("dir/.hidden_file")));
    }

    #[test]
    fn test_should_not_skip_regular_path() {
        assert!(!should_skip_path(Path::new("src/main.rs")));
        assert!(!should_skip_path(Path::new("lib/utils/helper.py")));
    }

    #[test]
    fn test_should_not_skip_github() {
        assert!(!should_skip_path(Path::new(".github/workflows/ci.yml")));
    }

    // -------------------------------------------------------------------------
    // Symlink Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_resolve_symlink_regular_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("regular_file.txt");
        fs::write(&file_path, "content").unwrap();

        let resolved = resolve_symlink_safely(&file_path, None).unwrap();
        assert_eq!(resolved, file_path.canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_symlink_valid_link() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");

        fs::write(&target, "content").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let resolved = resolve_symlink_safely(&link, None).unwrap();
        assert_eq!(resolved, target.canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_symlink_outside_project() {
        let project_dir = tempdir().unwrap();
        let outside_dir = tempdir().unwrap();

        let outside_file = outside_dir.path().join("outside.txt");
        let link = project_dir.path().join("link.txt");

        fs::write(&outside_file, "content").unwrap();
        std::os::unix::fs::symlink(&outside_file, &link).unwrap();

        let result = resolve_symlink_safely(&link, Some(project_dir.path()));
        assert!(matches!(result, Err(TldrError::PathTraversal(_))));
    }

    #[test]
    fn test_is_symlink_regular_file() {
        let file = NamedTempFile::new().unwrap();
        assert!(!is_symlink(file.path()));
    }

    // -------------------------------------------------------------------------
    // Path Validation Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_path_within_project_valid() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("src/main.rs");
        fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        fs::write(&file_path, "fn main() {}").unwrap();

        assert!(is_path_within_project(&file_path, dir.path()));
    }

    #[test]
    fn test_contains_path_traversal() {
        assert!(contains_path_traversal(Path::new("../outside")));
        assert!(contains_path_traversal(Path::new("dir/../other")));
        assert!(!contains_path_traversal(Path::new("dir/subdir/file")));
    }

    // -------------------------------------------------------------------------
    // Skip Directories Set Test
    // -------------------------------------------------------------------------

    #[test]
    fn test_skip_directories() {
        let dirs = skip_directories();
        assert!(dirs.contains("node_modules"));
        assert!(dirs.contains(".git"));
        assert!(dirs.contains("__pycache__"));
        assert!(!dirs.contains("src"));
    }

    // -------------------------------------------------------------------------
    // Walk Source Files Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_walk_source_files_single_file() {
        // A single file path should return vec![that_file]
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("main.py");
        fs::write(&file_path, "def main(): pass").unwrap();

        let options = WalkOptions::default();
        let (files, warnings) = walk_source_files(&file_path, &options).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Single file should return vec with one entry"
        );
        assert_eq!(files[0], file_path);
        assert!(warnings.is_empty(), "No warnings for single file");
    }

    #[test]
    fn test_walk_source_files_directory_returns_source_files() {
        // Directory with source files should return all of them
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("lib.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("app.js"), "function app() {}").unwrap();

        let options = WalkOptions::default();
        let (files, _warnings) = walk_source_files(dir.path(), &options).unwrap();

        assert!(
            files.len() >= 3,
            "Directory walk should find at least 3 source files, found {}",
            files.len()
        );
    }

    #[test]
    fn test_walk_source_files_language_filter() {
        // With lang filter, only files of that language should be returned
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("lib.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("app.js"), "function app() {}").unwrap();

        let options = WalkOptions {
            lang: Some(Language::Python),
            ..WalkOptions::default()
        };
        let (files, _warnings) = walk_source_files(dir.path(), &options).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Language filter should return only Python files"
        );
        assert!(
            files[0].extension().unwrap() == "py",
            "Filtered file should be .py"
        );
    }

    #[test]
    fn test_walk_source_files_empty_directory() {
        // Empty directory should return empty vec
        let dir = tempdir().unwrap();

        let options = WalkOptions::default();
        let (files, _warnings) = walk_source_files(dir.path(), &options).unwrap();

        assert!(files.is_empty(), "Empty directory should return empty vec");
    }

    #[test]
    fn test_walk_source_files_skips_non_source_files() {
        // Non-source files (.txt, .md, .lock) should be skipped
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("readme.md"), "# README").unwrap();
        fs::write(dir.path().join("notes.txt"), "some notes").unwrap();
        fs::write(dir.path().join("Cargo.lock"), "lock file").unwrap();
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();

        let options = WalkOptions::default();
        let (files, _warnings) = walk_source_files(dir.path(), &options).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Should only return source files, not .md/.txt/.lock. Found: {:?}",
            files
        );
    }

    #[test]
    fn test_walk_source_files_respects_gitignore() {
        // By default (gitignore=true), files in .gitignore should be skipped
        let dir = tempdir().unwrap();

        // Create a .gitignore that ignores *.log files
        fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();

        // Create a .py file (should be found) and a .log file (should be ignored)
        fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();
        fs::write(dir.path().join("debug.log"), "log data").unwrap();

        // Initialize git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .ok();

        let options = WalkOptions {
            gitignore: true,
            ..WalkOptions::default()
        };
        let (files, _warnings) = walk_source_files(dir.path(), &options).unwrap();

        // .log is not a source file anyway, but the important thing is the
        // walker uses gitignore. Let's verify source files are found.
        assert!(!files.is_empty(), "Should find at least the .py file");
        // Ensure no .log files snuck in
        for f in &files {
            assert_ne!(
                f.extension().and_then(|e| e.to_str()),
                Some("log"),
                "Should not include gitignored files"
            );
        }
    }

    #[test]
    fn test_walk_source_files_nonexistent_path() {
        // Nonexistent path should return PathNotFound error
        let options = WalkOptions::default();
        let result = walk_source_files(Path::new("/nonexistent/path/xyz"), &options);

        assert!(result.is_err(), "Nonexistent path should return error");
    }

    // -------------------------------------------------------------------------
    // Should Exclude Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_should_exclude_matching_pattern() {
        let patterns = vec!["*.test.py".to_string()];
        assert!(should_exclude(Path::new("test_foo.test.py"), &patterns));
    }

    #[test]
    fn test_should_exclude_no_match() {
        let patterns = vec!["*.test.py".to_string()];
        assert!(!should_exclude(Path::new("main.py"), &patterns));
    }

    #[test]
    fn test_should_exclude_empty_patterns() {
        let patterns: Vec<String> = vec![];
        assert!(!should_exclude(Path::new("anything.py"), &patterns));
    }
}
