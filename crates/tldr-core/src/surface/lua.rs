//! Lua-specific API surface extraction.
//!
//! Lua surfaces functions that are exported through a returned module table or
//! through keys in a returned table literal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::Language;
use crate::TldrResult;

use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Lua API surface for a resolved package.
pub fn extract_lua_api_surface(
    resolved: &ResolvedPackage,
    _include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_lua_files(&resolved.root_dir) {
        apis.extend(extract_from_lua_file(
            &file_path,
            &resolved.root_dir,
            &resolved.package_name,
        )?);
    }

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "lua".to_string(),
        total,
        apis,
    })
}

fn find_lua_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "lua")
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
                        files.extend(find_lua_files(&path));
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_lua_file(
    file_path: &Path,
    root_dir: &Path,
    package_name: &str,
) -> TldrResult<Vec<ApiEntry>> {
    let source = std::fs::read_to_string(file_path).map_err(|e| {
        crate::error::TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            format!("Cannot read: {}", e),
        )
    })?;

    let tree = parse(&source, Language::Lua)?;
    let module_info = extract_from_tree(&tree, &source, Language::Lua, file_path, Some(root_dir))?;
    let module_path = compute_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let exported_table = returned_module_table(&source);
    let returned_keys = returned_table_keys(&source);
    let mut apis = Vec::new();

    for func in module_info.functions {
        let line = source
            .lines()
            .nth(func.line_number.saturating_sub(1) as usize)
            .unwrap_or("")
            .trim();

        let export_name = if let Some(table_name) = exported_table.as_deref() {
            parse_table_export(line, table_name)
        } else {
            None
        }
        .or_else(|| returned_keys.get(&func.name).cloned());

        if let Some(export_name) = export_name {
            let params = func
                .params
                .into_iter()
                .map(|param| Param {
                    name: param,
                    type_annotation: None,
                    default: None,
                    is_variadic: false,
                    is_keyword: false,
                })
                .collect::<Vec<_>>();

            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", module_path, export_name),
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
                    export_name,
                    params
                        .iter()
                        .map(|p| p.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                triggers: extract_triggers(&export_name, None),
                is_property: false,
                return_type: func.return_type,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: func.line_number as usize,
                    column: None,
                }),
            });
        }
    }

    Ok(apis)
}

fn compute_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
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
    fn test_extract_lua_surface_from_module_table() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lua/app.lua",
            r#"
local M = {}

function M.hello(name)
  return name
end

local function hidden(name)
  return name
end

return M
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_lua_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|name| name.ends_with(".hello")));
        assert!(!names.iter().any(|name| name.ends_with(".hidden")));
    }

    #[test]
    fn test_extract_lua_surface_from_return_table_literal() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lua/app.lua",
            r#"
local function greet(name)
  return name
end

return { greet = greet }
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_lua_api_surface(&resolved, false, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(".greet")));
    }
}
