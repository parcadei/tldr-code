//! Package path resolution for API surface extraction.
//!
//! Resolves a package name to its source directory on disk:
//! - Python: `python3 -c "import <pkg>; print(<pkg>.__file__)"` -> site-packages path
//! - (Other languages planned for later phases)
//!
//! Also handles `__all__` extraction from Python `__init__.py` files.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::TldrError;
use crate::TldrResult;

use super::types::ResolvedPackage;

/// Resolve a Python package name to its source directory.
///
/// Uses `python3 -c "import <pkg>; print(<pkg>.__file__)"` to find the
/// installed package path, then determines the package root directory.
///
/// # Arguments
/// * `package_name` - Python package name (e.g., "json", "flask", "numpy")
///
/// # Returns
/// * `Ok(ResolvedPackage)` with the root directory and metadata
/// * `Err(TldrError::PackageNotFound)` if the package cannot be imported
pub fn resolve_python_package(package_name: &str) -> TldrResult<ResolvedPackage> {
    // Validate package name to prevent injection
    if !is_valid_python_identifier(package_name) {
        return Err(TldrError::parse_error(
            PathBuf::new(),
            None,
            format!("Invalid Python package name: {}", package_name),
        ));
    }

    let output = Command::new("python3")
        .arg("-c")
        .arg(format!(
            "import {pkg}; print({pkg}.__file__)",
            pkg = package_name
        ))
        .output()
        .map_err(|e| {
            TldrError::parse_error(
                PathBuf::new(),
                None,
                format!("Failed to run python3 to resolve package '{}': {}", package_name, e),
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TldrError::parse_error(
            PathBuf::new(),
            None,
            format!("Cannot import Python package '{}': {}", package_name, stderr.trim()),
        ));
    }

    let file_path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let file_path = PathBuf::from(&file_path_str);

    // Determine the package root directory
    let root_dir = if file_path.ends_with("__init__.py") {
        // Package with __init__.py -> parent directory is the package root
        file_path
            .parent()
            .ok_or_else(|| {
                TldrError::parse_error(
                    file_path.clone(),
                    None,
                    format!("Cannot determine parent directory of '{}'", file_path_str),
                )
            })?
            .to_path_buf()
    } else {
        // Single-file module (e.g., json.py) -> the file itself is the root
        // Treat parent as root to include the file
        file_path
            .parent()
            .ok_or_else(|| {
                TldrError::parse_error(
                    file_path.clone(),
                    None,
                    format!("Cannot determine parent directory of '{}'", file_path_str),
                )
            })?
            .to_path_buf()
    };

    // Check if package has pure Python source
    let is_pure_source = has_python_source(&root_dir, package_name, &file_path);

    // Extract __all__ from __init__.py if it exists
    let init_file = root_dir.join("__init__.py");
    let public_names = if init_file.exists() {
        extract_all_names(&init_file)
    } else {
        None
    };

    Ok(ResolvedPackage {
        root_dir,
        package_name: package_name.to_string(),
        is_pure_source,
        public_names,
    })
}

/// Resolve a target string to a package directory.
///
/// The target can be:
/// - A directory path (e.g., "./src/mylib/")
/// - A Python package name (e.g., "flask")
///
/// # Arguments
/// * `target` - Package name or directory path
/// * `lang` - Optional language hint
///
/// # Returns
/// * `Ok(ResolvedPackage)` with the resolved path
pub fn resolve_target(target: &str, lang: Option<&str>) -> TldrResult<ResolvedPackage> {
    let target_path = Path::new(target);

    // If it's a directory, use it directly
    if target_path.is_dir() {
        let package_name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(target)
            .to_string();

        return Ok(ResolvedPackage {
            root_dir: target_path.to_path_buf(),
            package_name,
            is_pure_source: true,
            public_names: None,
        });
    }

    // Otherwise, resolve as a package name based on language
    match lang {
        Some("python") | None => resolve_python_package(target),
        Some(other) => Err(TldrError::UnsupportedLanguage(format!(
            "Package resolution not yet supported for language: {}",
            other
        ))),
    }
}

/// Check if a Python identifier is valid (prevents injection in import command).
fn is_valid_python_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    // Allow dotted names like "xml.etree.ElementTree"
    for part in name.split('.') {
        if part.is_empty() {
            return false;
        }
        let mut chars = part.chars();
        // First char must be alpha or underscore
        if let Some(first) = chars.next() {
            if !first.is_alphabetic() && first != '_' {
                return false;
            }
        }
        // Rest must be alphanumeric or underscore
        for c in chars {
            if !c.is_alphanumeric() && c != '_' {
                return false;
            }
        }
    }

    true
}

/// Check if a package directory contains pure Python source files.
fn has_python_source(root_dir: &Path, _package_name: &str, init_file: &Path) -> bool {
    // If __init__.py exists and there are .py files, it's pure source
    if init_file.ends_with("__init__.py") && root_dir.join("__init__.py").exists() {
        // Check for at least one .py file beyond __init__.py
        if let Ok(entries) = std::fs::read_dir(root_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("py")
                    && path.file_name().and_then(|n| n.to_str()) != Some("__init__.py")
                {
                    return true;
                }
            }
        }
        // Only __init__.py found, still counts as pure source
        return true;
    }

    // If it's a single .py file
    if init_file.extension().and_then(|e| e.to_str()) == Some("py") {
        return true;
    }

    false
}

/// Extract `__all__` names from a Python `__init__.py` file.
///
/// Parses the file with tree-sitter to find `__all__ = [...]` assignments
/// and returns the list of exported names.
fn extract_all_names(init_file: &Path) -> Option<Vec<String>> {
    let source = std::fs::read_to_string(init_file).ok()?;
    extract_all_names_from_source(&source)
}

/// Extract `__all__` names from Python source code.
///
/// Looks for patterns like:
/// ```python
/// __all__ = ["foo", "bar", "baz"]
/// __all__ = ('foo', 'bar', 'baz')
/// ```
pub fn extract_all_names_from_source(source: &str) -> Option<Vec<String>> {
    use crate::ast::parser::parse;
    use crate::Language;

    let tree = parse(source, Language::Python).ok()?;
    let root = tree.root_node();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            if let Some(inner) = child.child(0) {
                if inner.kind() == "assignment" {
                    if let Some(left) = inner.child_by_field_name("left") {
                        let name = &source[left.byte_range()];
                        if name == "__all__" {
                            if let Some(right) = inner.child_by_field_name("right") {
                                return Some(extract_string_list_elements(
                                    &right, source,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extract string elements from a list or tuple literal node.
fn extract_string_list_elements(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let text = &source[child.byte_range()];
            // Strip quotes
            let unquoted = text
                .trim_start_matches(&['"', '\''][..])
                .trim_end_matches(&['"', '\''][..]);
            if !unquoted.is_empty() {
                names.push(unquoted.to_string());
            }
        }
    }

    names
}

/// Walk a directory recursively to find all Python source files.
///
/// Returns paths sorted for deterministic output.
pub fn find_python_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    find_python_files_recursive(root, &mut files);
    files.sort();
    files
}

/// Recursive helper for finding Python files.
fn find_python_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip common non-source directories
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !dir_name.starts_with('.')
                && dir_name != "__pycache__"
                && dir_name != "node_modules"
                && dir_name != ".git"
            {
                find_python_files_recursive(&path, files);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("py") {
            files.push(path);
        }
    }
}

/// Check if a directory contains C extension files (.so, .pyd).
pub fn has_c_extensions(dir: &Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext == "so" || ext == "pyd" {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_python_identifier() {
        assert!(is_valid_python_identifier("json"));
        assert!(is_valid_python_identifier("flask"));
        assert!(is_valid_python_identifier("my_package"));
        assert!(is_valid_python_identifier("_private"));
        assert!(is_valid_python_identifier("pkg123"));
        assert!(is_valid_python_identifier("xml.etree.ElementTree"));
    }

    #[test]
    fn test_is_valid_python_identifier_rejects_invalid() {
        assert!(!is_valid_python_identifier(""));
        assert!(!is_valid_python_identifier("123abc"));
        assert!(!is_valid_python_identifier("has space"));
        assert!(!is_valid_python_identifier("has;semicolon"));
        assert!(!is_valid_python_identifier("import os; os.system('rm -rf /')"));
        assert!(!is_valid_python_identifier(".dotstart"));
        assert!(!is_valid_python_identifier("dotend."));
    }

    #[test]
    fn test_extract_all_names_from_source() {
        let source = r#"
__all__ = ["loads", "dumps", "JSONEncoder", "JSONDecoder"]
"#;
        let names = extract_all_names_from_source(source);
        assert!(names.is_some());
        let names = names.unwrap();
        assert_eq!(names, vec!["loads", "dumps", "JSONEncoder", "JSONDecoder"]);
    }

    #[test]
    fn test_extract_all_names_single_quotes() {
        let source = r#"
__all__ = ['one', 'two', 'three']
"#;
        let names = extract_all_names_from_source(source);
        assert!(names.is_some());
        let names = names.unwrap();
        assert_eq!(names, vec!["one", "two", "three"]);
    }

    #[test]
    fn test_extract_all_names_tuple() {
        let source = r#"
__all__ = ("alpha", "beta")
"#;
        let names = extract_all_names_from_source(source);
        assert!(names.is_some());
        let names = names.unwrap();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn test_extract_all_names_absent() {
        let source = r#"
import os
def foo(): pass
"#;
        let names = extract_all_names_from_source(source);
        assert!(names.is_none());
    }

    #[test]
    fn test_resolve_target_directory() {
        // Use a temp dir to test directory resolution
        let tmp = std::env::temp_dir().join("tldr_test_resolve");
        let _ = std::fs::create_dir_all(&tmp);
        let result = resolve_target(tmp.to_str().unwrap(), Some("python"));
        assert!(result.is_ok());
        let pkg = result.unwrap();
        assert!(pkg.is_pure_source);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
