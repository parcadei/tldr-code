//! Find importers of a module (spec Section 2.2.4)
//!
//! Find all files that import a given module.
//!
//! # Features
//! - Captures line numbers
//! - Captures import statement text
//! - Supports both import and from-import styles
//! - Works with Python, TypeScript, Go

use std::collections::HashSet;
use std::path::Path;

use crate::ast::imports::get_imports;
use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{IgnoreSpec, ImporterInfo, ImportersReport, Language};
use crate::TldrResult;

/// Find all files that import a given module.
///
/// # Arguments
/// * `root` - Project root directory
/// * `module` - Module name to search for
/// * `language` - Programming language
///
/// # Returns
/// * `Ok(ImportersReport)` - List of files importing the module
pub fn find_importers(
    root: &Path,
    module: &str,
    language: Language,
) -> TldrResult<ImportersReport> {
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let tree = get_file_tree(root, Some(&extensions), true, Some(&IgnoreSpec::default()))?;
    let files = collect_files(&tree, root);

    let mut importers = Vec::new();

    for file_path in files {
        match find_import_in_file(&file_path, module, language) {
            Ok(Some(info)) => importers.push(info),
            Ok(None) => {}
            Err(e) => {
                if e.is_recoverable() {
                    // Skip files with parse errors
                    continue;
                }
            }
        }
    }

    let total = importers.len();
    Ok(ImportersReport {
        module: module.to_string(),
        importers,
        total,
    })
}

/// Check if a file imports the specified module
fn find_import_in_file(
    file_path: &Path,
    target_module: &str,
    language: Language,
) -> TldrResult<Option<ImporterInfo>> {
    let imports = get_imports(file_path, language)?;

    for import in &imports {
        if module_matches(&import.module, target_module, language) {
            // Read file to get the import statement text
            let content = std::fs::read_to_string(file_path)?;
            let lines: Vec<&str> = content.lines().collect();

            // Find the line containing this import
            let (line_number, import_statement) =
                find_import_line(&lines, &import.module, import.is_from, language);

            return Ok(Some(ImporterInfo {
                file: file_path.to_path_buf(),
                line: line_number,
                import_statement,
            }));
        }

        // Also check if target is one of the imported names
        if import.is_from {
            // from X import target_module
            if import.names.iter().any(|n| n == target_module) {
                let content = std::fs::read_to_string(file_path)?;
                let lines: Vec<&str> = content.lines().collect();
                let (line_number, import_statement) =
                    find_import_line(&lines, &import.module, true, language);

                return Ok(Some(ImporterInfo {
                    file: file_path.to_path_buf(),
                    line: line_number,
                    import_statement,
                }));
            }
        }
    }

    Ok(None)
}

/// Check if a module name matches the target
fn module_matches(import_module: &str, target: &str, language: Language) -> bool {
    match language {
        Language::Python => {
            // Exact match
            if import_module == target {
                return true;
            }
            // Submodule match: services.auth matches services
            if import_module.starts_with(&format!("{}.", target)) {
                return true;
            }
            // Target is submodule: services matches services.auth
            if target.starts_with(&format!("{}.", import_module)) {
                return true;
            }
            // Handle relative imports
            let cleaned_import = import_module.trim_start_matches('.');
            let cleaned_target = target.trim_start_matches('.');
            cleaned_import == cleaned_target
        }
        Language::TypeScript | Language::JavaScript => {
            // Normalize paths
            let normalized_import = import_module.replace('\\', "/");
            let normalized_target = target.replace('\\', "/");

            if normalized_import == normalized_target {
                return true;
            }
            // Handle ./relative paths
            let import_clean = normalized_import.trim_start_matches("./");
            let target_clean = normalized_target.trim_start_matches("./");
            import_clean == target_clean
        }
        Language::Go => {
            // Package path matching
            import_module == target || import_module.ends_with(&format!("/{}", target))
        }
        _ => import_module == target,
    }
}

/// Find the line number and text of an import statement
fn find_import_line(
    lines: &[&str],
    module: &str,
    is_from: bool,
    language: Language,
) -> (u32, String) {
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        match language {
            Language::Python => {
                if is_from {
                    if trimmed.starts_with("from ") && trimmed.contains(module) {
                        return (i as u32 + 1, trimmed.to_string());
                    }
                } else if trimmed.starts_with("import ") && trimmed.contains(module) {
                    return (i as u32 + 1, trimmed.to_string());
                }
            }
            Language::TypeScript | Language::JavaScript => {
                if trimmed.contains("import") && trimmed.contains(module) {
                    return (i as u32 + 1, trimmed.to_string());
                }
                if trimmed.contains("require") && trimmed.contains(module) {
                    return (i as u32 + 1, trimmed.to_string());
                }
            }
            Language::Go => {
                if trimmed.contains("import") && trimmed.contains(module) {
                    return (i as u32 + 1, trimmed.to_string());
                }
            }
            _ => {
                if trimmed.contains(module) {
                    return (i as u32 + 1, trimmed.to_string());
                }
            }
        }
    }

    // Fallback
    (1, format!("import {}", module))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_matches_python() {
        // Exact match
        assert!(module_matches(
            "services.auth",
            "services.auth",
            Language::Python
        ));

        // Submodule match
        assert!(module_matches(
            "services.auth",
            "services",
            Language::Python
        ));

        // No match
        assert!(!module_matches("utils", "services", Language::Python));

        // Relative import
        assert!(module_matches(".auth", "auth", Language::Python));
    }

    #[test]
    fn test_module_matches_typescript() {
        assert!(module_matches("./utils", "./utils", Language::TypeScript));
        assert!(module_matches("./utils", "utils", Language::TypeScript));
        assert!(module_matches("utils", "./utils", Language::TypeScript));
    }

    #[test]
    fn test_find_import_line() {
        let lines = vec![
            "\"\"\"Module docstring\"\"\"",
            "",
            "from typing import List",
            "from services.auth import authenticate",
            "",
            "def main():",
            "    pass",
        ];

        let (line, stmt) = find_import_line(&lines, "services.auth", true, Language::Python);
        assert_eq!(line, 4);
        assert!(stmt.contains("services.auth"));
    }
}
