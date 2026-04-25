//! Luau-specific API surface extraction.
//!
//! Luau is a typed superset of Lua used by Roblox and others. It adds:
//!
//! - Optional type annotations on parameters and return types
//!   (`function f(x: number): string ... end`)
//! - `local function` (private) vs. `function` (public) at the module level
//! - `export type Name = ...` as a public type alias
//!
//! The public/private heuristic mirrors the language's conventions:
//!
//! - Top-level `function name(...)` (no `local`) is **public**.
//! - `local function name(...)` and `local name = function(...)` are **private**
//!   unless the surrounding module re-exports them via a returned table or
//!   assignment to a module table (the same pattern Lua uses).
//! - A function exported through `M.name = function(...)` or
//!   `function M.name(...)` is **public** when `M` is the returned module table.
//! - Names beginning with `_` are conventionally private.
//!
//! `export type Foo = ...` is surfaced as an [`ApiKind::TypeAlias`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::Language;
use crate::TldrResult;

use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Luau API surface for a resolved package.
///
/// Walks the resolved package's root directory for `.luau` and `.lua` files
/// (Luau is a Lua superset, so `.lua` files in a Luau project are valid).
/// For each file we run the AST-based extractor (which already understands
/// typed parameters and typed return values for Luau) and then apply
/// the language's public/private heuristic plus any module-table re-exports.
pub fn extract_luau_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_luau_files(&resolved.root_dir) {
        apis.extend(extract_from_luau_file(
            &file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?);
    }

    sort_apis_by_static_preference(&mut apis, "luau");

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "luau".to_string(),
        total,
        apis,
    })
}

/// Find all `.luau` and `.lua` files under `dir` (recursively).
fn find_luau_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| matches!(*ext, "luau" | "lua"))
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') {
                        files.extend(find_luau_files(&path));
                    }
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("luau" | "lua")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_luau_file(
    file_path: &Path,
    root_dir: &Path,
    package_name: &str,
    include_private: bool,
) -> TldrResult<Vec<ApiEntry>> {
    let source = std::fs::read_to_string(file_path).map_err(|e| {
        crate::error::TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            format!("Cannot read: {}", e),
        )
    })?;

    // Pick the right grammar: .luau files use the Luau grammar; .lua files
    // in a Luau project are still parsed as Lua (the Luau grammar is a strict
    // superset, so for a `.lua` file the Lua grammar is the correct fit).
    let language = match file_path.extension().and_then(|ext| ext.to_str()) {
        Some("luau") => Language::Luau,
        _ => Language::Lua,
    };

    let tree = parse(&source, language)?;
    let module_info = extract_from_tree(&tree, &source, language, file_path, Some(root_dir))?;
    let module_path = compute_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let exported_table = returned_module_table(&source);
    let returned_keys = returned_table_keys(&source);
    let mut apis = Vec::new();

    for func in module_info.functions {
        let line_text = source
            .lines()
            .nth(func.line_number.saturating_sub(1) as usize)
            .unwrap_or("")
            .trim();

        // Module-table dotted/colon export: `function M.name(...)` or
        // `M.name = function(...)`.
        let module_export_name = if let Some(table_name) = exported_table.as_deref() {
            parse_table_export(line_text, table_name)
        } else {
            None
        }
        .or_else(|| returned_keys.get(&func.name).cloned());

        let is_local = is_local_function(line_text);
        let is_underscore = func.name.starts_with('_');
        let is_publicly_exported = module_export_name.is_some();

        // Decide visibility:
        //   - Module-table exports are always public.
        //   - Top-level `function name(...)` (no `local`) is public unless
        //     the name is conventionally private (`_name`).
        //   - Everything else is private.
        let is_public = is_publicly_exported || (!is_local && !is_underscore);

        if !is_public && !include_private {
            continue;
        }

        // Skip raw nested table-method definitions when no surrounding module
        // table is exported (e.g. `function obj:method(...)` on an internal
        // table) -- those are not meaningful module-level APIs.
        if !is_publicly_exported && line_text.contains('.') && !line_text.starts_with("function ") {
            continue;
        }

        let exposed_name = module_export_name.clone().unwrap_or_else(|| func.name.clone());

        let params: Vec<Param> = func
            .params
            .iter()
            .map(|name| Param {
                name: name.clone(),
                type_annotation: None,
                default: None,
                is_variadic: name == "...",
                is_keyword: false,
            })
            .collect();

        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, exposed_name),
            kind: ApiKind::Function,
            module: module_path.clone(),
            signature: Some(Signature {
                params: params.clone(),
                return_type: func.return_type.clone(),
                is_async: false,
                is_generator: false,
            }),
            docstring: func.docstring,
            example: Some(format!(
                "{}({})",
                exposed_name,
                params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            triggers: extract_triggers(&exposed_name, None),
            is_property: false,
            return_type: func.return_type,
            location: Some(Location {
                file: relative_path.clone(),
                line: func.line_number as usize,
                column: None,
            }),
        });
    }

    // `export type Name = ...` declarations are public type aliases.
    for export in find_exported_types(&source) {
        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, export.name),
            kind: ApiKind::TypeAlias,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}.{}", module_path, export.name)),
            triggers: extract_triggers(&export.name, None),
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: export.line,
                column: None,
            }),
        });
    }

    Ok(apis)
}

fn compute_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let mut parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    // Drop the file name and strip its extension to recover a module-style path.
    if let Some(last) = parts.pop() {
        let stem = Path::new(&last)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or(last);
        if !stem.is_empty() {
            parts.push(stem);
        }
    }
    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

/// Detect if a line declares a `local function ...` (Luau private) or
/// `local name = function(...)`.
fn is_local_function(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("local function") || trimmed.starts_with("local ")
        && trimmed.contains("function")
}

fn returned_module_table(source: &str) -> Option<String> {
    source.lines().rev().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix("return ")
            .map(str::trim)
            .filter(|rest| {
                !rest.contains('{') && rest.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            })
            .map(|name| name.to_string())
    })
}

fn returned_table_keys(source: &str) -> HashMap<String, String> {
    let mut exports = HashMap::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(body) = trimmed
            .strip_prefix("return {")
            .and_then(|s| s.strip_suffix('}'))
        {
            for item in body.split(',') {
                let part = item.trim();
                if let Some((key, value)) = part.split_once('=') {
                    let export_key = key.trim().to_string();
                    let local_name = value.trim().trim_start_matches("M.").to_string();
                    exports.insert(local_name, export_key);
                }
            }
        }
    }
    exports
}

fn parse_table_export(line: &str, table_name: &str) -> Option<String> {
    for separator in [".", ":"] {
        let needle = format!("{table_name}{separator}");
        if let Some(rest) = line.split(&needle).nth(1) {
            let name = rest
                .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .next()
                .unwrap_or("")
                .to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

struct ExportedType {
    name: String,
    line: usize,
}

/// Scan the source text for `export type Foo = ...` declarations.
///
/// Tree-sitter-luau represents these as `type_definition` nodes nested inside
/// an export-style construct, but the surface layer only needs the names and
/// line numbers. A line-based scan is robust against grammar variations and
/// stays in lockstep with how `lua.rs` resolves its module-table re-exports.
fn find_exported_types(source: &str) -> Vec<ExportedType> {
    let mut out = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let rest = match trimmed.strip_prefix("export type ") {
            Some(rest) => rest,
            None => continue,
        };
        let name: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            out.push(ExportedType {
                name,
                line: index + 1,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, rel: &str, source: &str) {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, source).unwrap();
    }

    #[test]
    fn test_extract_luau_typed_top_level_function_is_public() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.luau",
            "function greet(name: string): string\n    return \"Hello \" .. name\nend\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, false, None).unwrap();
        assert_eq!(surface.language, "luau");
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|name| name.ends_with(".greet")),
            "expected greet to be exported, got: {:?}",
            names
        );

        // Verify the typed return type is preserved on the surface entry.
        let greet = surface
            .apis
            .iter()
            .find(|api| api.qualified_name.ends_with(".greet"))
            .unwrap();
        assert!(greet.return_type.is_some());
    }

    #[test]
    fn test_extract_luau_local_function_filtered_when_private_excluded() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.luau",
            "local function helper(): number\n    return 1\nend\n\n\
             function exposed(): number\n    return 2\nend\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|name| name.ends_with(".exposed")),
            "expected `exposed` to surface; got {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.ends_with(".helper")),
            "local `helper` should be filtered when include_private=false; got {:?}",
            names
        );
    }

    #[test]
    fn test_extract_luau_local_function_included_when_private_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.luau",
            "local function helper(): number\n    return 1\nend\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, true, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|name| name.ends_with(".helper")),
            "include_private=true should surface local `helper`; got {:?}",
            names
        );
    }

    #[test]
    fn test_extract_luau_module_table_export() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "util.luau",
            "local M = {}\n\
             function M.b_util(): number\n    return 2\nend\n\
             return M\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, false, None).unwrap();
        assert!(
            surface
                .apis
                .iter()
                .any(|api| api.qualified_name.ends_with(".b_util")),
            "expected M.b_util to surface via module-table export; got: {:?}",
            surface
                .apis
                .iter()
                .map(|a| a.qualified_name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_luau_export_type_surfaces_as_type_alias() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "types.luau",
            "export type Point = { x: number, y: number }\n\
             export type Vec3 = { x: number, y: number, z: number }\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, false, None).unwrap();
        let aliases: Vec<&ApiEntry> = surface
            .apis
            .iter()
            .filter(|api| matches!(api.kind, ApiKind::TypeAlias))
            .collect();
        assert!(
            aliases.iter().any(|api| api.qualified_name.ends_with(".Point")),
            "expected Point to surface as TypeAlias; got: {:?}",
            surface
                .apis
                .iter()
                .map(|a| (a.qualified_name.as_str(), a.kind))
                .collect::<Vec<_>>()
        );
        assert!(aliases.iter().any(|api| api.qualified_name.ends_with(".Vec3")));
    }

    #[test]
    fn test_extract_luau_underscore_prefixed_treated_as_private() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.luau",
            "function _internal(): number\n    return 1\nend\n\n\
             function public_one(): number\n    return 2\nend\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_luau_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|n| n.ends_with(".public_one")));
        assert!(
            !names.iter().any(|n| n.ends_with("._internal")),
            "underscore-prefixed function should be private by default; got: {:?}",
            names
        );
    }
}
