//! Import map building, Python import parsing, and re-export tracing.
//!
//! This module handles the construction of import maps from resolved imports,
//! Go module import augmentation, JS/TS default export detection, Python import
//! extraction and parsing, and circular re-export detection.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lazy_static::lazy_static;
use regex::Regex;

use super::cross_file_types::{FileIR, ImportDef, ResolvedImport};
use super::import_resolver::{ImportResolver, ReExportTracer, DEFAULT_MAX_DEPTH};
use super::languages::base::{get_node_text, walk_tree};
use super::types::{parse_source, FuncIndex};

// =============================================================================
// Type Aliases
// =============================================================================

/// Maps local name (or alias) -> (module_path, original_name)
///
/// Used to resolve direct calls where the local name differs from the original
/// name in the source module.
///
/// # Example
/// ```python
/// from pkg.module import MyClass as MC
/// ```
/// Results in: `import_map["MC"] = ("pkg.module", "MyClass")`
pub type ImportMap = HashMap<String, (String, String)>;

/// Maps module alias -> module_path
///
/// Used to resolve attribute access calls like `json.loads()` where
/// `json` is an imported module.
///
/// # Example
/// ```python
/// import json
/// ```
/// Results in: `module_imports["json"] = "json"`
///
/// ```python
/// import numpy as np
/// ```
/// Results in: `module_imports["np"] = "numpy"`
pub type ModuleImports = HashMap<String, String>;

// =============================================================================
// Import Map Building
// =============================================================================

/// Build import map from resolved imports.
///
/// This function processes the output of `ImportResolver::resolve()` and builds
/// two lookup maps:
/// 1. `ImportMap` for "from X import Y" style imports
/// 2. `ModuleImports` for "import X" style imports
///
/// # Spec Reference
/// Section 14.7: Import Map Construction
///
/// # Arguments
/// * `resolved_imports` - Resolved imports from ImportResolver
///
/// # Returns
/// A tuple of (ImportMap, ModuleImports)
///
/// # Example
/// ```rust,ignore
/// let resolved = resolver.resolve(&import_def, current_file);
/// let (import_map, module_imports) = build_import_map(&resolved);
///
/// // "from pkg import Foo as F" -> import_map["F"] = ("pkg", "Foo")
/// // "import json" -> module_imports["json"] = "json"
/// ```
pub fn build_import_map(resolved_imports: &[ResolvedImport]) -> (ImportMap, ModuleImports) {
    let mut import_map = ImportMap::new();
    let mut module_imports = ModuleImports::new();

    for resolved in resolved_imports {
        // Skip external modules - they're not in our project
        if resolved.is_external {
            continue;
        }

        // Skip unresolved imports
        let resolved_name = match &resolved.resolved_name {
            Some(name) => name.clone(),
            None => continue,
        };

        let original = &resolved.original;

        // Determine the module path for this import
        let raw_module_path = original
            .resolved_module
            .as_ref()
            .unwrap_or(&original.module)
            .clone();

        // Normalize JS/TS module paths: strip file extensions (.js, .ts, etc.)
        // TS ESM imports often use ".js" extension (e.g., `from "./errors.js"`)
        // but ModuleIndex stores paths without extension (e.g., "./errors").
        // This normalization ensures import_map keys match func_index keys.
        let module_path = raw_module_path
            .strip_suffix(".js")
            .or_else(|| raw_module_path.strip_suffix(".jsx"))
            .or_else(|| raw_module_path.strip_suffix(".ts"))
            .or_else(|| raw_module_path.strip_suffix(".tsx"))
            .or_else(|| raw_module_path.strip_suffix(".mjs"))
            .or_else(|| raw_module_path.strip_suffix(".cjs"))
            .unwrap_or(&raw_module_path)
            .to_string();

        if original.is_from {
            // "from X import Y" or "from X import Y as Z"
            // Handle each imported name
            for name in &original.names {
                if name == "*" {
                    // Wildcard imports are handled differently - each resolved name
                    // gets its own entry
                    let local_name = resolved_name.clone();
                    import_map.insert(local_name.clone(), (module_path.clone(), local_name));
                } else {
                    // Check if there's an alias for this name
                    let alias_name = original
                        .aliases
                        .as_ref()
                        .and_then(|aliases| aliases.iter().find(|(_, v)| *v == name))
                        .map(|(alias, _)| alias.clone());

                    // BUG FIX 1: Map BOTH alias AND original name (CROSSFILE_SPEC.md Section 4.2)
                    // When `from X import Y as Z`, both `Y` and `Z` should resolve to the same target.
                    // Previously only the alias was mapped, causing calls using the original name to fail.
                    if let Some(alias) = alias_name {
                        // Insert alias -> (module, original_name)
                        import_map.insert(alias, (module_path.clone(), name.clone()));
                    }
                    // Always insert original name -> (module, original_name)
                    import_map.insert(name.clone(), (module_path.clone(), name.clone()));
                }
            }
        } else {
            // "import X" or "import X as Y"
            let local_name = original.alias.as_ref().unwrap_or(&original.module).clone();

            // For module imports, we track the module path for attribute access
            // e.g., json.loads() needs to know json -> "json" module
            module_imports.insert(local_name.clone(), module_path.clone());

            // JS/TS default/require alias: try to resolve a default export name so direct calls map.
            if (original.is_default || (original.alias.is_some() && !original.is_namespace))
                && original.names.is_empty()
            {
                if let Some(resolved_file) = &resolved.resolved_file {
                    if let Some(default_name) = find_js_ts_default_export_name(resolved_file) {
                        import_map.insert(local_name, (module_path.clone(), default_name));
                    }
                }
            }
        }
    }

    (import_map, module_imports)
}

// =============================================================================
// Go Module Import Augmentation
// =============================================================================

/// Augment `module_imports` for Go cross-package function calls.
///
/// Go imports use full module paths (e.g., `"go-callgraph-test/pkg/models"`),
/// but func_index keys use relative directory paths (e.g., `"pkg/models"`).
/// The package alias used in code is the last path component (e.g., `"models"`).
///
/// This function bridges the gap by:
/// 1. Extracting the Go package alias (explicit or last component)
/// 2. Finding a matching func_index module key whose tail matches the import path
/// 3. Adding `alias -> matching_module` to `module_imports`
///
/// # Arguments
/// * `imports` - ImportDefs from the Go source file
/// * `module_imports` - Mutable reference to the module imports map to augment
/// * `func_index` - Function index for finding matching module keys
///
/// # Example
/// ```text
/// import "go-callgraph-test/pkg/models"
/// -> alias = "models", match func_index key "pkg/models"
/// -> module_imports["models"] = "pkg/models"
///
/// import svc "go-callgraph-test/pkg/service"
/// -> alias = "svc", match func_index key "pkg/service"
/// -> module_imports["svc"] = "pkg/service"
/// ```
pub fn augment_go_module_imports(
    imports: &[ImportDef],
    module_imports: &mut ModuleImports,
    func_index: &FuncIndex,
) {
    // Collect all unique module keys from func_index for matching
    let known_modules: HashSet<&str> = func_index.iter().map(|((module, _), _)| module).collect();

    for import_def in imports {
        let module_path = &import_def.module;
        if module_path.is_empty() {
            continue;
        }

        // Determine the Go package alias
        let alias = match &import_def.alias {
            Some(a) if a == "_" || a == "." => continue, // Skip blank and dot imports
            Some(a) => a.clone(),
            None => {
                // Default alias is the last path component
                match module_path.rsplit('/').next() {
                    Some(last) if !last.is_empty() => last.to_string(),
                    _ => continue,
                }
            }
        };

        // Skip if already mapped (e.g., ImportResolver already resolved it)
        if module_imports.contains_key(&alias) {
            continue;
        }

        // Try to find a matching module key in func_index.
        // Strategy: check if any known module key is a suffix of the import path.
        // E.g., import "go-callgraph-test/pkg/models" matches func_index key "pkg/models"
        // because "go-callgraph-test/pkg/models" ends with "/pkg/models"
        let mut best_match: Option<&str> = None;
        let mut best_len = 0;

        for known in &known_modules {
            if known.is_empty() {
                continue;
            }
            // Check if the import path ends with the known module key
            // Either the known module IS the import path, or the import path
            // ends with "/<known_module>"
            if *known == module_path.as_str() {
                best_match = Some(known);
                break; // Exact match, done
            }
            let suffix = format!("/{}", known);
            if module_path.ends_with(&suffix) && known.len() > best_len {
                best_match = Some(known);
                best_len = known.len();
            }
        }

        if let Some(matched) = best_match {
            module_imports.insert(alias, matched.to_string());
        }
    }
}

// =============================================================================
// JS/TS Default Export Detection
// =============================================================================

pub(crate) fn find_js_ts_default_export_name(path: &Path) -> Option<String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !matches!(ext, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs") {
        return None;
    }

    let source = fs::read_to_string(path).ok()?;

    lazy_static! {
        static ref RE_EXPORT_DEFAULT_FN: Regex =
            Regex::new(r"(?m)^\\s*export\\s+default\\s+function\\s+([A-Za-z_$][\\w$]*)").unwrap();
        static ref RE_EXPORT_DEFAULT_CLASS: Regex =
            Regex::new(r"(?m)^\\s*export\\s+default\\s+class\\s+([A-Za-z_$][\\w$]*)").unwrap();
        static ref RE_EXPORT_DEFAULT_IDENT: Regex =
            Regex::new(r"(?m)^\\s*export\\s+default\\s+([A-Za-z_$][\\w$]*)").unwrap();
        static ref RE_EXPORTS_DEFAULT: Regex =
            Regex::new(r"(?m)^\\s*exports\\.default\\s*=\\s*([A-Za-z_$][\\w$]*)").unwrap();
        static ref RE_MODULE_EXPORTS: Regex =
            Regex::new(r"(?m)^\\s*module\\.exports\\s*=\\s*([A-Za-z_$][\\w$]*)").unwrap();
    }

    if let Some(caps) = RE_EXPORT_DEFAULT_FN.captures(&source) {
        return Some(caps[1].to_string());
    }
    if let Some(caps) = RE_EXPORT_DEFAULT_CLASS.captures(&source) {
        return Some(caps[1].to_string());
    }
    if let Some(caps) = RE_EXPORT_DEFAULT_IDENT.captures(&source) {
        let ident = caps[1].to_string();
        if ident != "function" && ident != "class" {
            return Some(ident);
        }
    }
    if let Some(caps) = RE_EXPORTS_DEFAULT.captures(&source) {
        return Some(caps[1].to_string());
    }
    if let Some(caps) = RE_MODULE_EXPORTS.captures(&source) {
        return Some(caps[1].to_string());
    }

    None
}

// =============================================================================
// Import Resolution
// =============================================================================

/// Extract and resolve imports for a single file.
///
/// This function:
/// 1. Extracts import statements from the FileIR
/// 2. Resolves them using the ImportResolver
/// 3. Handles circular re-export detection (M2.2)
/// 4. Handles self-import detection (M2.3)
///
/// # Mitigations Implemented
/// - M2.2: Circular re-export detection with visited set
/// - M2.3: Self-import detection (check if resolved == current file)
///
/// # Arguments
/// * `file_ir` - The FileIR containing import statements
/// * `resolver` - The ImportResolver to use
/// * `root` - Project root for path computation
///
/// # Returns
/// Vector of resolved imports for this file
pub fn resolve_imports_for_file<'a>(
    file_ir: &FileIR,
    resolver: &mut ImportResolver<'a>,
    root: &Path,
) -> Vec<ResolvedImport> {
    let current_file = root.join(&file_ir.path);
    let mut resolved_imports = Vec::new();

    for import_def in &file_ir.imports {
        // Resolve this import using the ImportResolver
        let resolved = resolver.resolve(import_def, &current_file);

        // Filter out self-imports (M2.3)
        for r in resolved {
            // Check if this resolves to the same file (self-import)
            if let Some(ref resolved_file) = r.resolved_file {
                let resolved_canonical: Option<PathBuf> = resolved_file.canonicalize().ok();
                let current_canonical: Option<PathBuf> = current_file.canonicalize().ok();

                if resolved_canonical == current_canonical {
                    // Self-import detected - skip it
                    continue;
                }
            }

            resolved_imports.push(r);
        }
    }

    resolved_imports
}

// =============================================================================
// Python Import Extraction
// =============================================================================

/// Extract imports from Python source code.
///
/// Parses import statements from Python source and populates FileIR.imports.
///
/// # Arguments
/// * `source` - Python source code
/// * `file_ir` - FileIR to populate with imports
///
/// # Returns
/// Number of imports extracted
pub fn extract_python_imports(source: &str, file_ir: &mut FileIR) -> usize {
    // Parse the source
    let tree = match parse_source(source, "python") {
        Ok(t) => t,
        Err(_) => return 0,
    };

    let source_bytes = source.as_bytes();
    let root = tree.root_node();

    let mut import_count = 0;

    for node in walk_tree(root) {
        match node.kind() {
            "import_statement" => {
                // import X or import X as Y
                if let Some(import_def) = parse_python_import_statement(&node, source_bytes) {
                    file_ir.imports.push(import_def);
                    import_count += 1;
                }
            }
            "import_from_statement" => {
                // from X import Y
                if let Some(import_def) = parse_python_from_import(&node, source_bytes) {
                    file_ir.imports.push(import_def);
                    import_count += 1;
                }
            }
            _ => {}
        }
    }

    // Explicitly drop tree to free memory
    drop(tree);

    import_count
}

/// Parse a Python "import X" statement.
pub(crate) fn parse_python_import_statement(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<ImportDef> {
    // import X
    // import X as Y
    // import X.Y.Z as Z

    let mut module = String::new();
    let mut alias = None;

    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            match child.kind() {
                "dotted_name" => {
                    module = get_node_text(&child, source).to_string();
                }
                "aliased_import" => {
                    // X as Y
                    if let Some(name_node) = child.child_by_field_name("name") {
                        module = get_node_text(&name_node, source).to_string();
                    }
                    if let Some(alias_node) = child.child_by_field_name("alias") {
                        alias = Some(get_node_text(&alias_node, source).to_string());
                    }
                }
                _ => {}
            }
        }
    }

    if module.is_empty() {
        return None;
    }

    let mut import_def = ImportDef::simple_import(&module);
    import_def.alias = alias;

    Some(import_def)
}

/// Parse a Python "from X import Y" statement.
pub(crate) fn parse_python_from_import(
    node: &tree_sitter::Node,
    source: &[u8],
) -> Option<ImportDef> {
    // from X import Y
    // from X import Y as Z
    // from . import Y
    // from .. import Y
    // from ...X import Y

    let mut module = String::new();
    let mut level = 0u8;
    let mut names = Vec::new();
    let mut aliases = HashMap::new();
    let mut is_wildcard = false;

    // Handle relative imports
    // tree-sitter-python uses a "relative_import" node containing dots and module
    // e.g., "from . import X" has relative_import="."
    // e.g., "from ..utils import X" has relative_import="..utils"
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "relative_import" {
                let text = get_node_text(&child, source);
                // Count leading dots
                for c in text.chars() {
                    if c == '.' {
                        level += 1;
                    } else {
                        break;
                    }
                }
                // Extract module name (part after dots)
                let module_part: String = text.chars().skip_while(|&c| c == '.').collect();
                if !module_part.is_empty() {
                    module = module_part;
                }
                break;
            }
        }
    }

    // For non-relative imports, get module name from module_name field
    if level == 0 {
        if let Some(module_node) = node.child_by_field_name("module_name") {
            module = get_node_text(&module_node, source).to_string();
        }
    }

    // Extract imported names
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            match child.kind() {
                "dotted_name"
                    if child != node.child_by_field_name("module_name").unwrap_or(child) =>
                {
                    let name = get_node_text(&child, source).to_string();
                    names.push(name);
                }
                "aliased_import" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = get_node_text(&name_node, source).to_string();
                        names.push(name.clone());

                        if let Some(alias_node) = child.child_by_field_name("alias") {
                            let alias = get_node_text(&alias_node, source).to_string();
                            aliases.insert(alias, name);
                        }
                    }
                }
                "wildcard_import" => {
                    is_wildcard = true;
                    names.push("*".to_string());
                }
                _ => {}
            }
        }
    }

    // If no names found, try to get them from the node text directly
    if names.is_empty() && !is_wildcard {
        // Check for import_list or individual names
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "import_list" {
                    for j in 0..child.named_child_count() {
                        if let Some(name_child) = child.named_child(j) {
                            if name_child.kind() == "dotted_name"
                                || name_child.kind() == "identifier"
                            {
                                names.push(get_node_text(&name_child, source).to_string());
                            } else if name_child.kind() == "aliased_import" {
                                if let Some(name_node) = name_child.child_by_field_name("name") {
                                    let name = get_node_text(&name_node, source).to_string();
                                    names.push(name.clone());

                                    if let Some(alias_node) =
                                        name_child.child_by_field_name("alias")
                                    {
                                        let alias = get_node_text(&alias_node, source).to_string();
                                        aliases.insert(alias, name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if names.is_empty() && !is_wildcard {
        return None;
    }

    let mut import_def = if level > 0 {
        ImportDef::relative_import(&module, names, level)
    } else {
        ImportDef::from_import(&module, names)
    };

    if !aliases.is_empty() {
        import_def.aliases = Some(aliases);
    }

    Some(import_def)
}

// =============================================================================
// Re-export Tracing
// =============================================================================

/// Trace circular re-exports with visited set.
///
/// Per Mitigation M2.2: Use a proper visited set passed through recursive calls,
/// detect cycles explicitly, and log the full cycle path for debugging.
///
/// # Arguments
/// * `tracer` - ReExportTracer to use
/// * `module` - Module to start from
/// * `name` - Name to trace
/// * `visited` - Already visited (module, name) pairs
///
/// # Returns
/// The final (module, name) location, or None if cycle detected or max depth reached
pub fn trace_reexport_with_cycle_detection(
    tracer: &mut ReExportTracer<'_>,
    module: &str,
    name: &str,
    visited: &mut HashSet<(String, String)>,
) -> Option<(PathBuf, String)> {
    let key = (module.to_string(), name.to_string());

    // Check for cycle (M2.2)
    if visited.contains(&key) {
        // Circular re-export detected - return None
        return None;
    }

    visited.insert(key);

    // Use the tracer's trace method with the visited set size as implicit depth limit
    if visited.len() > DEFAULT_MAX_DEPTH {
        // Max depth exceeded
        return None;
    }

    // Trace the re-export
    tracer
        .trace(module, name, DEFAULT_MAX_DEPTH - visited.len())
        .map(|traced| (traced.definition_file, traced.qualified_name))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::types::{FuncEntry, FuncIndex};
    use super::*;
    use crate::callgraph::cross_file_types::{FileIR, ImportDef, ResolvedImport};
    use std::collections::HashMap;
    use std::path::PathBuf;

    // =========================================================================
    // Import Map Tests
    // =========================================================================

    /// Test: build_import_map correctly maps from-imports
    #[test]
    fn test_build_import_map_from_import() {
        // Create a resolved import: from pkg.module import MyClass
        let import_def = ImportDef::from_import("pkg.module", vec!["MyClass".to_string()]);
        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("pkg/module.py")),
            resolved_name: Some("MyClass".to_string()),
            is_external: false,
            confidence: 1.0,
        };

        let (import_map, module_imports) = build_import_map(&[resolved]);

        assert!(
            import_map.contains_key("MyClass"),
            "Should have MyClass in import_map"
        );
        assert_eq!(
            import_map.get("MyClass"),
            Some(&("pkg.module".to_string(), "MyClass".to_string()))
        );
        assert!(
            module_imports.is_empty(),
            "module_imports should be empty for from-imports"
        );
    }

    /// Test: build_import_map handles aliases correctly
    #[test]
    fn test_build_import_map_with_alias() {
        // Create: from pkg import MyClass as MC
        let mut import_def = ImportDef::from_import("pkg", vec!["MyClass".to_string()]);
        let mut aliases = HashMap::new();
        aliases.insert("MC".to_string(), "MyClass".to_string());
        import_def.aliases = Some(aliases);

        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("pkg/__init__.py")),
            resolved_name: Some("MyClass".to_string()),
            is_external: false,
            confidence: 1.0,
        };

        let (import_map, _) = build_import_map(&[resolved]);

        assert!(
            import_map.contains_key("MC"),
            "Should have alias MC in import_map"
        );
        assert_eq!(
            import_map.get("MC"),
            Some(&("pkg".to_string(), "MyClass".to_string()))
        );
    }

    /// Test: build_import_map handles module imports
    #[test]
    fn test_build_import_map_module_import() {
        // Create: import json
        let import_def = ImportDef::simple_import("json");
        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: None, // stdlib, not in project
            resolved_name: Some("json".to_string()),
            is_external: true, // External module
            confidence: 1.0,
        };

        let (import_map, module_imports) = build_import_map(&[resolved]);

        // External modules are skipped
        assert!(import_map.is_empty());
        assert!(module_imports.is_empty());
    }

    /// Test: build_import_map handles import X as Y
    #[test]
    fn test_build_import_map_import_as() {
        // Create: import numpy as np (project module)
        let mut import_def = ImportDef::simple_import("mylib.core");
        import_def.alias = Some("mc".to_string());

        let resolved = ResolvedImport {
            original: import_def,
            resolved_file: Some(PathBuf::from("mylib/core.py")),
            resolved_name: Some("mylib.core".to_string()),
            is_external: false,
            confidence: 1.0,
        };

        let (import_map, module_imports) = build_import_map(&[resolved]);

        assert!(
            import_map.is_empty(),
            "import_map should be empty for module imports"
        );
        assert!(
            module_imports.contains_key("mc"),
            "Should have alias in module_imports"
        );
        assert_eq!(module_imports.get("mc"), Some(&"mylib.core".to_string()));
    }

    /// Test: build_import_map filters external modules
    #[test]
    fn test_build_import_map_filters_external() {
        let external_import = ImportDef::from_import("os.path", vec!["join".to_string()]);
        let resolved = ResolvedImport {
            original: external_import,
            resolved_file: None,
            resolved_name: Some("join".to_string()),
            is_external: true,
            confidence: 1.0,
        };

        let (import_map, module_imports) = build_import_map(&[resolved]);

        assert!(import_map.is_empty(), "External imports should be filtered");
        assert!(module_imports.is_empty());
    }

    // =========================================================================
    // Python Import Extraction Tests
    // =========================================================================

    /// Test: extract_python_imports extracts simple imports
    #[test]
    fn test_extract_python_imports_simple() {
        let source = r#"
import json
import os
"#;
        let mut file_ir = FileIR::new(PathBuf::from("test.py"));
        let count = extract_python_imports(source, &mut file_ir);

        assert_eq!(count, 2, "Should extract 2 imports");
        assert_eq!(file_ir.imports.len(), 2);
    }

    /// Test: extract_python_imports extracts from imports
    #[test]
    fn test_extract_python_imports_from() {
        let source = r#"
from pkg.module import MyClass
from os import path
"#;
        let mut file_ir = FileIR::new(PathBuf::from("test.py"));
        let count = extract_python_imports(source, &mut file_ir);

        assert_eq!(count, 2, "Should extract 2 from-imports");
        assert!(file_ir.imports.iter().any(|i| i.module == "pkg.module"));
        assert!(file_ir
            .imports
            .iter()
            .any(|i| i.names.contains(&"MyClass".to_string())));
    }

    /// Test: extract_python_imports handles aliases
    #[test]
    fn test_extract_python_imports_alias() {
        let source = r#"
import numpy as np
from typing import List as L
"#;
        let mut file_ir = FileIR::new(PathBuf::from("test.py"));
        let count = extract_python_imports(source, &mut file_ir);

        assert_eq!(count, 2, "Should extract 2 imports with aliases");

        // Check import numpy as np
        let np_import = file_ir.imports.iter().find(|i| i.module == "numpy");
        assert!(np_import.is_some());
        assert_eq!(np_import.unwrap().alias, Some("np".to_string()));
    }

    /// Test: extract_python_imports handles relative imports
    #[test]
    fn test_extract_python_imports_relative() {
        let source = r#"
from . import types
from ..utils import helper
from ...core.base import Base
"#;
        let mut file_ir = FileIR::new(PathBuf::from("pkg/sub/module.py"));
        let count = extract_python_imports(source, &mut file_ir);

        assert!(count >= 2, "Should extract relative imports");

        // Check levels
        let level1_import = file_ir.imports.iter().find(|i| i.level == 1);
        assert!(level1_import.is_some(), "Should have level 1 import");

        let level2_import = file_ir.imports.iter().find(|i| i.level == 2);
        assert!(level2_import.is_some(), "Should have level 2 import");

        let level3_import = file_ir.imports.iter().find(|i| i.level == 3);
        assert!(level3_import.is_some(), "Should have level 3 import");
    }

    // =========================================================================
    // Type Alias Tests
    // =========================================================================

    /// Test: ImportMap type alias works correctly
    #[test]
    fn test_import_map_type() {
        let mut map: ImportMap = HashMap::new();
        map.insert(
            "MC".to_string(),
            ("pkg.module".to_string(), "MyClass".to_string()),
        );

        assert_eq!(map.get("MC").unwrap().0, "pkg.module");
        assert_eq!(map.get("MC").unwrap().1, "MyClass");
    }

    /// Test: ModuleImports type alias works correctly
    #[test]
    fn test_module_imports_type() {
        let mut imports: ModuleImports = HashMap::new();
        imports.insert("np".to_string(), "numpy".to_string());

        assert_eq!(imports.get("np"), Some(&"numpy".to_string()));
    }

    // =========================================================================
    // Go Module Import Augmentation Tests
    // =========================================================================

    /// Test: augment_go_module_imports maps package alias to func_index module key
    #[test]
    fn test_augment_go_module_imports_basic() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/models",
            "NewUser",
            FuncEntry::function(PathBuf::from("pkg/models/user.go"), 12, 14),
        );

        let imports = vec![ImportDef::simple_import("go-callgraph-test/pkg/models")];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert_eq!(
            module_imports.get("models"),
            Some(&"pkg/models".to_string()),
            "Should map 'models' alias to 'pkg/models' func_index key"
        );
    }

    /// Test: augment_go_module_imports with explicit alias
    #[test]
    fn test_augment_go_module_imports_explicit_alias() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/service",
            "NewUserService",
            FuncEntry::function(PathBuf::from("pkg/service/service.go"), 10, 13),
        );

        let mut import_def = ImportDef::simple_import("go-callgraph-test/pkg/service");
        import_def.alias = Some("svc".to_string());

        let imports = vec![import_def];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert_eq!(
            module_imports.get("svc"),
            Some(&"pkg/service".to_string()),
            "Should map explicit alias 'svc' to 'pkg/service'"
        );
        assert!(
            !module_imports.contains_key("service"),
            "Should NOT map default alias when explicit alias is given"
        );
    }

    /// Test: augment_go_module_imports skips blank imports
    #[test]
    fn test_augment_go_module_imports_skip_blank() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/effects",
            "Init",
            FuncEntry::function(PathBuf::from("pkg/effects/init.go"), 1, 5),
        );

        let mut import_def = ImportDef::simple_import("pkg/effects");
        import_def.alias = Some("_".to_string());

        let imports = vec![import_def];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert!(
            module_imports.is_empty(),
            "Blank imports (_) should be skipped"
        );
    }

    /// Test: augment_go_module_imports skips dot imports
    #[test]
    fn test_augment_go_module_imports_skip_dot() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/utils",
            "Helper",
            FuncEntry::function(PathBuf::from("pkg/utils/utils.go"), 1, 5),
        );

        let mut import_def = ImportDef::simple_import("pkg/utils");
        import_def.alias = Some(".".to_string());

        let imports = vec![import_def];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert!(
            module_imports.is_empty(),
            "Dot imports (.) should be skipped"
        );
    }

    /// Test: augment_go_module_imports does not overwrite existing entries
    #[test]
    fn test_augment_go_module_imports_no_overwrite() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/models",
            "NewUser",
            FuncEntry::function(PathBuf::from("pkg/models/user.go"), 12, 14),
        );

        let imports = vec![ImportDef::simple_import("go-callgraph-test/pkg/models")];
        let mut module_imports = ModuleImports::new();
        // Pre-populate with an existing mapping (e.g., from ImportResolver)
        module_imports.insert("models".to_string(), "already/resolved".to_string());

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert_eq!(
            module_imports.get("models"),
            Some(&"already/resolved".to_string()),
            "Should NOT overwrite existing module_imports entries"
        );
    }

    /// Test: augment_go_module_imports handles unresolvable imports gracefully
    #[test]
    fn test_augment_go_module_imports_external() {
        let func_index = FuncIndex::new(); // Empty - no project modules

        let imports = vec![
            ImportDef::simple_import("github.com/gin-gonic/gin"),
            ImportDef::simple_import("fmt"),
        ];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert!(
            module_imports.is_empty(),
            "External/stdlib imports should not be added"
        );
    }

    /// Test: augment_go_module_imports handles multiple imports
    #[test]
    fn test_augment_go_module_imports_multiple() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/models",
            "NewUser",
            FuncEntry::function(PathBuf::from("pkg/models/user.go"), 12, 14),
        );
        func_index.insert(
            "pkg/service",
            "NewUserService",
            FuncEntry::function(PathBuf::from("pkg/service/service.go"), 10, 13),
        );

        let imports = vec![
            ImportDef::simple_import("myapp/pkg/models"),
            ImportDef::simple_import("myapp/pkg/service"),
            ImportDef::simple_import("fmt"), // stdlib, no match
        ];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        assert_eq!(
            module_imports.get("models"),
            Some(&"pkg/models".to_string()),
        );
        assert_eq!(
            module_imports.get("service"),
            Some(&"pkg/service".to_string()),
        );
        assert!(!module_imports.contains_key("fmt"));
        assert_eq!(module_imports.len(), 2);
    }

    /// Test: augment_go_module_imports prefers longest suffix match
    #[test]
    fn test_augment_go_module_imports_longest_match() {
        let mut func_index = FuncIndex::new();
        // Two modules where one's name is a suffix of the other
        func_index.insert(
            "models",
            "Func1",
            FuncEntry::function(PathBuf::from("models/m.go"), 1, 5),
        );
        func_index.insert(
            "internal/models",
            "Func2",
            FuncEntry::function(PathBuf::from("internal/models/m.go"), 1, 5),
        );

        let imports = vec![ImportDef::simple_import("myapp/internal/models")];
        let mut module_imports = ModuleImports::new();

        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        // Should match "internal/models" (longer suffix) not just "models"
        assert_eq!(
            module_imports.get("models"),
            Some(&"internal/models".to_string()),
            "Should prefer longest suffix match"
        );
    }
}
