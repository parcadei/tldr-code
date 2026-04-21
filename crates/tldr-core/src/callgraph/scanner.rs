//! File scanning, language detection, and directory walking for Builder V2.
//!
//! This module handles project file discovery:
//! - Language support detection and normalization
//! - ScannedFile struct with TOCTOU protection
//! - Directory walking with ignore patterns
//! - Symlink cycle detection
//!
//! Depends only on types.rs (BuildConfig, BuildError) and crate::types::Language.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use walkdir::WalkDir;

use super::types::{BuildConfig, BuildError};
use crate::types::Language;

// =============================================================================
// Language Support
// =============================================================================

const SUPPORTED_LANGUAGES: &[&str] = &[
    "python",
    "typescript",
    "javascript",
    "go",
    "rust",
    "java",
    "c",
    "cpp",
    "csharp",
    "kotlin",
    "scala",
    "swift",
    "php",
    "ruby",
    "lua",
    "luau",
    "elixir",
    "ocaml",
];

pub(crate) fn normalize_language_string(language: &str) -> String {
    match language.to_lowercase().as_str() {
        "py" => "python".to_string(),
        "ts" | "tsx" => "typescript".to_string(),
        "js" | "jsx" => "javascript".to_string(),
        "golang" => "go".to_string(),
        "rs" => "rust".to_string(),
        "rb" => "ruby".to_string(),
        "kt" => "kotlin".to_string(),
        "c++" | "cxx" => "cpp".to_string(),
        "c#" | "cs" => "csharp".to_string(),
        "ex" => "elixir".to_string(),
        "ml" => "ocaml".to_string(),
        other => other.to_string(),
    }
}

/// Check if a language is supported.
pub(crate) fn is_supported_language(language: &str) -> bool {
    SUPPORTED_LANGUAGES.contains(&language.to_lowercase().as_str())
}

// =============================================================================
// ScannedFile (Mitigation M2.7 - TOCTOU Protection)
// =============================================================================

/// A scanned file with metadata for TOCTOU protection.
///
/// Per Mitigation M2.7: "Record mtime during scan for later validation."
/// This allows detection of files that change between scanning and parsing.
///
/// # Example
/// ```rust,ignore
/// let files = scan_project_files(root, "python", &config)?;
/// for scanned in files {
///     // Later, before parsing, we can verify the file hasn't changed
///     if scanned.verify_unchanged().is_err() {
///         eprintln!("File modified: {:?}", scanned.path);
///         continue;
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ScannedFile {
    /// Path to the file (absolute).
    pub path: PathBuf,

    /// File modification time at scan time.
    pub mtime: SystemTime,

    /// File size in bytes at scan time.
    pub size: u64,
}

impl ScannedFile {
    /// Create a new ScannedFile from path with current metadata.
    pub fn from_path(path: PathBuf) -> std::io::Result<Self> {
        let metadata = fs::metadata(&path)?;
        Ok(Self {
            path,
            mtime: metadata.modified()?,
            size: metadata.len(),
        })
    }

    /// Verify the file hasn't changed since scanning.
    ///
    /// Returns `Ok(())` if file metadata matches, `Err` with description otherwise.
    pub fn verify_unchanged(&self) -> Result<(), String> {
        let current_meta =
            fs::metadata(&self.path).map_err(|e| format!("Cannot read file: {}", e))?;

        let current_mtime = current_meta
            .modified()
            .map_err(|e| format!("Cannot read mtime: {}", e))?;

        if current_mtime != self.mtime {
            return Err(format!(
                "File modified: scanned at {:?}, now {:?}",
                self.mtime, current_mtime
            ));
        }

        if current_meta.len() != self.size {
            return Err(format!(
                "File size changed: was {} bytes, now {} bytes",
                self.size,
                current_meta.len()
            ));
        }

        Ok(())
    }
}

// =============================================================================
// File Discovery (Spec Section 14.4 Step 2)
// =============================================================================

/// Directories to always skip during file discovery.
const SKIP_DIRECTORIES: &[&str] = &[
    ".git",
    "__pycache__",
    "node_modules",
    ".tox",
    "venv",
    ".venv",
    "__pypackages__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    "target",     // Rust
    "build",      // Various
    "dist",       // Various
    ".next",      // Next.js
    ".nuxt",      // Nuxt.js
    "vendor",     // Go, PHP
    ".bundle",    // Ruby
    "Pods",       // iOS
    ".gradle",    // Gradle
    ".idea",      // JetBrains
    ".vscode",    // VSCode
    ".eggs",      // Python
    "*.egg-info", // Python
    ".coverage",  // Python coverage
    "htmlcov",    // Python coverage
];

fn resolve_scan_roots(root: &Path, config: &BuildConfig) -> Result<Vec<PathBuf>, BuildError> {
    if !config.use_workspace_config {
        return Ok(vec![root.to_path_buf()]);
    }

    if config.workspace_roots.is_empty() {
        return Err(BuildError::WorkspaceConfig(
            "Workspace roots not provided".to_string(),
        ));
    }

    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for workspace_root in &config.workspace_roots {
        let candidate = if workspace_root.is_absolute() {
            workspace_root.clone()
        } else {
            root.join(workspace_root)
        };
        let candidate = dunce::simplified(&candidate).to_path_buf();

        if !candidate.exists() {
            return Err(BuildError::WorkspaceConfig(format!(
                "Workspace root not found: {}",
                candidate.display()
            )));
        }
        if !candidate.is_dir() {
            return Err(BuildError::WorkspaceConfig(format!(
                "Workspace root is not a directory: {}",
                candidate.display()
            )));
        }
        if !candidate.starts_with(root) {
            return Err(BuildError::WorkspaceConfig(format!(
                "Workspace root {} is outside project root {}",
                candidate.display(),
                root.display()
            )));
        }

        if seen.insert(candidate.clone()) {
            roots.push(candidate);
        }
    }

    if roots.is_empty() {
        return Err(BuildError::WorkspaceConfig(
            "Workspace roots resolved to empty set".to_string(),
        ));
    }

    Ok(roots)
}

/// Scan a project directory for source files of the specified language.
///
/// This function discovers all relevant source files in the project,
/// respecting ignore patterns and detecting symlink cycles.
///
/// # Arguments
/// * `root` - Project root directory (must exist and be a directory)
/// * `language` - Language to scan for (e.g., "python", "typescript")
/// * `config` - Build configuration with ignore settings
///
/// # Returns
/// * `Ok(Vec<ScannedFile>)` - List of discovered files with metadata
/// * `Err(BuildError)` - If root doesn't exist or language is unsupported
///
/// # Behavior
/// - Returns empty Vec for empty projects (not an error)
/// - Skips standard ignored directories (.git, __pycache__, node_modules, etc.)
/// - Respects .tldrignore patterns when `config.respect_ignore` is true
/// - Detects and breaks symlink cycles via canonical path comparison
/// - Restricts scanning to workspace roots when `config.use_workspace_config` is true
///
/// # Example
/// ```rust,ignore
/// let config = BuildConfig {
///     language: "python".to_string(),
///     respect_ignore: true,
///     ..Default::default()
/// };
/// let files = scan_project_files(root, "python", &config)?;
/// println!("Found {} Python files", files.len());
/// ```
pub fn scan_project_files(
    root: &Path,
    language: &str,
    config: &BuildConfig,
) -> Result<Vec<ScannedFile>, BuildError> {
    // Validate root exists and is a directory
    if !root.exists() {
        return Err(BuildError::RootNotFound(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(BuildError::RootNotFound(root.to_path_buf()));
    }

    // Get canonical root for symlink cycle detection
    let canonical_root = root.canonicalize().map_err(BuildError::Io)?;

    let scan_roots = resolve_scan_roots(root, config)?;

    // Get language extensions
    let extensions = get_language_extensions(language)?;

    // Load .tldrignore patterns if respect_ignore is enabled
    let gitignore = if config.respect_ignore {
        load_tldrignore(root)
    } else {
        None
    };

    // Track visited canonical paths for symlink cycle detection
    let mut visited_dirs: HashSet<PathBuf> = HashSet::new();
    visited_dirs.insert(canonical_root.clone());
    for scan_root in &scan_roots {
        if let Ok(canonical) = scan_root.canonicalize() {
            visited_dirs.insert(canonical);
        }
    }

    let mut files = Vec::new();
    let mut seen_files: HashSet<PathBuf> = HashSet::new();

    for scan_root in scan_roots {
        // Walk the directory tree
        let walker = WalkDir::new(&scan_root)
            .follow_links(true) // Follow symlinks, but detect cycles
            .into_iter()
            .filter_entry(|entry| {
                // Skip hidden files/dirs and known ignored directories
                let file_name = entry.file_name().to_string_lossy();

                // Skip hidden files/directories (except root)
                if entry.depth() > 0 && file_name.starts_with('.') {
                    return false;
                }

                // Skip known ignored directories
                if entry.file_type().is_dir() && should_skip_directory(&file_name) {
                    return false;
                }

                true
            });

        for entry_result in walker {
            let entry = match entry_result {
                Ok(e) => e,
                Err(err) => {
                    // Log and skip errors (permission denied, broken symlinks, etc.)
                    if config.verbose {
                        eprintln!("Warning: skipping entry: {}", err);
                    }
                    continue;
                }
            };

            // Handle symlink cycle detection for directories
            if entry.file_type().is_dir() {
                if let Ok(canonical) = entry.path().canonicalize() {
                    if !visited_dirs.insert(canonical.clone()) {
                        // Already visited this directory - symlink cycle detected
                        if config.verbose {
                            eprintln!(
                                "Warning: symlink cycle detected at {:?}, skipping",
                                entry.path()
                            );
                        }
                        continue;
                    }
                }
                continue; // Directories don't get added to files list
            }

            // Only process regular files
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();

            // Check file extension matches language
            if !has_matching_extension(path, &extensions) {
                continue;
            }

            // Check .tldrignore patterns
            if let Some(ref gi) = gitignore {
                let relative_path = path.strip_prefix(root).unwrap_or(path);
                if gi.matched(relative_path, false).is_ignore() {
                    continue;
                }
            }

            if !seen_files.insert(path.to_path_buf()) {
                continue;
            }

            // Create ScannedFile with metadata
            match ScannedFile::from_path(path.to_path_buf()) {
                Ok(scanned) => files.push(scanned),
                Err(err) => {
                    if config.verbose {
                        eprintln!("Warning: cannot read metadata for {:?}: {}", path, err);
                    }
                }
            }
        }
    }

    Ok(files)
}

/// Check if a path should be skipped based on configuration.
///
/// This is a utility function for determining whether to process a path.
///
/// # Arguments
/// * `path` - Path to check
/// * `config` - Build configuration
///
/// # Returns
/// * `true` if the path should be skipped
/// * `false` if the path should be processed
pub fn should_skip_path(path: &Path, _config: &BuildConfig) -> bool {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();

    // Skip hidden files/directories
    if file_name.starts_with('.') {
        return true;
    }

    // Skip known directories
    if path.is_dir() && should_skip_directory(&file_name) {
        return true;
    }

    // Note: .tldrignore checking is done separately in scan_project_files
    // because it requires the gitignore matcher and relative path calculation

    false
}

/// Check if a directory name should be skipped.
pub(crate) fn should_skip_directory(name: &str) -> bool {
    SKIP_DIRECTORIES.iter().any(|skip| {
        if skip.contains('*') {
            // Simple glob pattern matching for patterns like "*.egg-info"
            let pattern = skip.replace("*", "");
            name.ends_with(&pattern)
        } else {
            name == *skip
        }
    })
}

/// Get file extensions for a language.
pub(crate) fn get_language_extensions(language: &str) -> Result<Vec<&'static str>, BuildError> {
    // Map language string to Language enum
    let lang = match language.to_lowercase().as_str() {
        "python" => Language::Python,
        "typescript" => Language::TypeScript,
        "javascript" => Language::JavaScript,
        "go" => Language::Go,
        "rust" => Language::Rust,
        "java" => Language::Java,
        "c" => Language::C,
        "cpp" => Language::Cpp,
        "csharp" => Language::CSharp,
        "kotlin" => Language::Kotlin,
        "scala" => Language::Scala,
        "swift" => Language::Swift,
        "php" => Language::Php,
        "ruby" => Language::Ruby,
        "lua" => Language::Lua,
        "luau" => Language::Luau,
        "elixir" => Language::Elixir,
        "ocaml" => Language::Ocaml,
        _ => return Err(BuildError::UnsupportedLanguage(language.to_string())),
    };

    Ok(lang.extensions().to_vec())
}

/// Check if a path has a matching file extension.
fn has_matching_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let with_dot = format!(".{}", ext.to_lowercase());
            extensions.iter().any(|e| e.to_lowercase() == with_dot)
        })
        .unwrap_or(false)
}

/// Load .tldrignore patterns from project root.
fn load_tldrignore(root: &Path) -> Option<Gitignore> {
    let tldrignore_path = root.join(".tldrignore");

    if !tldrignore_path.exists() {
        return None;
    }

    let mut builder = GitignoreBuilder::new(root);

    // Add patterns from .tldrignore
    if builder.add(&tldrignore_path).is_some() {
        // Error adding file, return None
        return None;
    }

    builder.build().ok()
}

/// Filter a list of paths through `.tldrignore` patterns.
///
/// Loads `.tldrignore` from `root` and removes any paths that match its
/// patterns. If no `.tldrignore` file exists, returns the input unchanged.
///
/// Uses `matched_path_or_any_parents` so that directory patterns like
/// `corpus/` correctly filter files nested under that directory.
pub fn filter_tldrignored(root: &Path, paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let ignore = match load_tldrignore(root) {
        Some(ig) => ig,
        None => return paths,
    };

    paths
        .into_iter()
        .filter(|p| {
            let is_dir = p.is_dir();
            !ignore.matched_path_or_any_parents(p, is_dir).is_ignore()
        })
        .collect()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::types::{BuildConfig, BuildError};
    use super::*;
    use std::fs;
    
    use tempfile::TempDir;

    #[test]
    fn test_supported_languages() {
        assert!(is_supported_language("python"));
        assert!(is_supported_language("Python")); // Case insensitive
        assert!(is_supported_language("typescript"));
        assert!(!is_supported_language("brainfuck"));
        assert!(!is_supported_language(""));
    }

    /// Test: Project root validation.
    /// Spec Section 14.10: "RootNotFound if root doesn't exist"
    #[test]
    fn test_root_not_found() {
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = scan_project_files(
            Path::new("/nonexistent/path/that/does/not/exist"),
            "python",
            &config,
        );

        assert!(result.is_err(), "Nonexistent root should fail");
        match result.unwrap_err() {
            BuildError::RootNotFound(path) => {
                assert!(path.to_string_lossy().contains("nonexistent"));
            }
            err => panic!("Expected RootNotFound, got: {:?}", err),
        }
    }

    /// Test: Unsupported language handling.
    /// Spec Section 14.10: "UnsupportedLanguage if language not in registry"
    #[test]
    fn test_unsupported_language() {
        let dir = TempDir::new().unwrap();
        let config = BuildConfig {
            language: "brainfuck".to_string(), // Not supported
            ..Default::default()
        };

        let result = scan_project_files(dir.path(), "brainfuck", &config);

        assert!(result.is_err(), "Unsupported language should fail");
        match result.unwrap_err() {
            BuildError::UnsupportedLanguage(lang) => {
                assert_eq!(lang, "brainfuck");
            }
            err => panic!("Expected UnsupportedLanguage, got: {:?}", err),
        }
    }

    /// Test: RootNotFound also applies to files (not directories)
    #[test]
    fn test_root_must_be_directory() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        std::fs::write(&file_path, "def foo(): pass").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = scan_project_files(&file_path, "python", &config);

        assert!(result.is_err(), "File path (not directory) should fail");
        assert!(matches!(result.unwrap_err(), BuildError::RootNotFound(_)));
    }

    /// Test: ScannedFile creation and verification
    #[test]
    fn test_scanned_file_from_path() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        std::fs::write(&file_path, "def foo(): pass").unwrap();

        let scanned = ScannedFile::from_path(file_path.clone()).unwrap();

        assert_eq!(scanned.path, file_path);
        assert!(scanned.size > 0);
        // mtime should be recent (within last minute)
        let now = std::time::SystemTime::now();
        let elapsed = now.duration_since(scanned.mtime).unwrap();
        assert!(elapsed.as_secs() < 60);
    }

    /// Test: ScannedFile verify_unchanged passes when file unchanged
    #[test]
    fn test_scanned_file_verify_unchanged() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        std::fs::write(&file_path, "def foo(): pass").unwrap();

        let scanned = ScannedFile::from_path(file_path).unwrap();

        // File hasn't changed, should pass
        assert!(scanned.verify_unchanged().is_ok());
    }

    /// Test: scan_project_files finds Python files
    #[test]
    fn test_scan_project_files_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.py"), "def a(): pass").unwrap();
        std::fs::write(dir.path().join("b.py"), "def b(): pass").unwrap();
        std::fs::write(dir.path().join("c.txt"), "not python").unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let files = scan_project_files(dir.path(), "python", &config).unwrap();

        assert_eq!(files.len(), 2, "Should find 2 Python files");
        let paths: Vec<_> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(paths.contains(&"a.py".to_string()));
        assert!(paths.contains(&"b.py".to_string()));
    }

    /// Test: scan_project_files skips __pycache__
    #[test]
    fn test_scan_skips_pycache() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();

        let cache_dir = dir.path().join("__pycache__");
        std::fs::create_dir(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("cached.py"), "# cached").unwrap();

        let config = BuildConfig::default();
        let files = scan_project_files(dir.path(), "python", &config).unwrap();

        assert_eq!(files.len(), 1, "Should only find main.py, not cached file");
    }

    /// Test: scan_project_files skips node_modules
    #[test]
    fn test_scan_skips_node_modules() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.ts"), "export const x = 1;").unwrap();

        let nm_dir = dir.path().join("node_modules");
        std::fs::create_dir(&nm_dir).unwrap();
        std::fs::write(nm_dir.join("dep.ts"), "export const y = 2;").unwrap();

        let config = BuildConfig::default();
        let files = scan_project_files(dir.path(), "typescript", &config).unwrap();

        assert_eq!(files.len(), 1, "Should only find index.ts, not dep.ts");
    }

    /// Test: scan_project_files returns empty for empty directory
    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let config = BuildConfig::default();

        let files = scan_project_files(dir.path(), "python", &config).unwrap();

        assert!(files.is_empty(), "Empty directory should return empty Vec");
    }

    /// Test: scan_project_files skips .git directory
    #[test]
    fn test_scan_skips_git() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.py"), "def main(): pass").unwrap();

        let git_dir = dir.path().join(".git");
        std::fs::create_dir(&git_dir).unwrap();
        std::fs::write(git_dir.join("hooks.py"), "# git hook").unwrap();

        let config = BuildConfig::default();
        let files = scan_project_files(dir.path(), "python", &config).unwrap();

        assert_eq!(
            files.len(),
            1,
            "Should only find main.py, not .git contents"
        );
    }

    /// Test: should_skip_path for hidden files
    #[test]
    fn test_should_skip_hidden_files() {
        let config = BuildConfig::default();

        assert!(should_skip_path(Path::new(".hidden"), &config));
        assert!(should_skip_path(Path::new(".gitignore"), &config));
        assert!(!should_skip_path(Path::new("visible.py"), &config));
    }

    /// Test: should_skip_path for known directories
    #[test]
    fn test_should_skip_known_dirs() {
        let _config = BuildConfig::default();

        // These should be skipped when they are directories
        let skip_names = vec![
            "__pycache__",
            "node_modules",
            ".git",
            "venv",
            "target",
            ".mypy_cache",
            ".tox",
        ];

        for name in skip_names {
            assert!(should_skip_directory(name), "Should skip {}", name);
        }

        // Normal directories should not be skipped
        assert!(!should_skip_directory("src"));
        assert!(!should_skip_directory("tests"));
    }

    /// Test: get_language_extensions returns correct extensions
    #[test]
    fn test_language_extensions() {
        let py_exts = get_language_extensions("python").unwrap();
        assert!(py_exts.contains(&".py"));

        let ts_exts = get_language_extensions("typescript").unwrap();
        assert!(py_exts.len() < ts_exts.len() || ts_exts.contains(&".ts"));

        let result = get_language_extensions("invalid_language");
        assert!(result.is_err());
    }

    /// Test: scan_project_files in subdirectories
    #[test]
    fn test_scan_subdirectories() {
        let dir = TempDir::new().unwrap();

        let subdir = dir.path().join("src").join("pkg");
        std::fs::create_dir_all(&subdir).unwrap();

        std::fs::write(dir.path().join("main.py"), "# main").unwrap();
        std::fs::write(subdir.join("module.py"), "# module").unwrap();

        let config = BuildConfig::default();
        let files = scan_project_files(dir.path(), "python", &config).unwrap();

        assert_eq!(files.len(), 2, "Should find files in subdirectories");
    }

    // =========================================================================
    // filter_tldrignored tests
    // =========================================================================

    #[test]
    fn test_filter_tldrignored_no_ignore_file() {
        let dir = TempDir::new().unwrap();
        let paths = vec![
            dir.path().join("src/main.rs"),
            dir.path().join("corpus/test.rs"),
        ];

        let result = filter_tldrignored(dir.path(), paths.clone());
        assert_eq!(result.len(), 2, "Without .tldrignore, all paths should pass through");
    }

    #[test]
    fn test_filter_tldrignored_excludes_matched_paths() {
        let dir = TempDir::new().unwrap();

        // Create .tldrignore excluding corpus/ and tmp/
        fs::write(dir.path().join(".tldrignore"), "corpus/\ntmp/\n").unwrap();

        // Create the directories so is_dir works
        fs::create_dir_all(dir.path().join("corpus")).unwrap();
        fs::create_dir_all(dir.path().join("tmp")).unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();

        // Create files
        fs::write(dir.path().join("corpus/vendored.py"), "").unwrap();
        fs::write(dir.path().join("tmp/scratch.py"), "").unwrap();
        fs::write(dir.path().join("src/main.py"), "").unwrap();

        let paths = vec![
            dir.path().join("corpus/vendored.py"),
            dir.path().join("tmp/scratch.py"),
            dir.path().join("src/main.py"),
        ];

        let result = filter_tldrignored(dir.path(), paths);
        assert_eq!(result.len(), 1, "Only src/main.py should survive filtering");
        assert!(result[0].ends_with("src/main.py"));
    }

    #[test]
    fn test_filter_tldrignored_glob_patterns() {
        let dir = TempDir::new().unwrap();

        // Exclude all .generated.rs files
        fs::write(dir.path().join(".tldrignore"), "*.generated.rs\n").unwrap();

        fs::write(dir.path().join("real.rs"), "").unwrap();
        fs::write(dir.path().join("types.generated.rs"), "").unwrap();

        let paths = vec![
            dir.path().join("real.rs"),
            dir.path().join("types.generated.rs"),
        ];

        let result = filter_tldrignored(dir.path(), paths);
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("real.rs"));
    }

    #[test]
    fn test_filter_tldrignored_empty_input() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(".tldrignore"), "corpus/\n").unwrap();

        let result = filter_tldrignored(dir.path(), Vec::new());
        assert!(result.is_empty());
    }
}
