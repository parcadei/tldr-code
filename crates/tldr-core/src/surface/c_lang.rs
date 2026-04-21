//! C-specific API surface extraction.
//!
//! When headers are present, the public C surface is derived from function
//! declarations in header files. If no headers are present, we fall back to
//! function definitions in `.c` files.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::Language;
use crate::TldrResult;

use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public C API surface for a resolved package.
pub fn extract_c_api_surface(
    resolved: &ResolvedPackage,
    _include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let (headers, sources) = find_c_files(&resolved.root_dir);
    let mut apis = if !headers.is_empty() {
        extract_from_c_headers(&headers, &resolved.root_dir, &resolved.package_name)?
    } else {
        let mut collected = Vec::new();
        for file_path in sources {
            collected.extend(extract_from_c_source_file(
                &file_path,
                &resolved.root_dir,
                &resolved.package_name,
            )?);
        }
        collected
    };

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "c".to_string(),
        total,
        apis,
    })
}

fn find_c_files(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut headers = Vec::new();
    let mut sources = Vec::new();

    if dir.is_file() {
        match dir.extension().and_then(|ext| ext.to_str()) {
            Some("h") => headers.push(dir.to_path_buf()),
            Some("c") => sources.push(dir.to_path_buf()),
            _ => {}
        }
        return (headers, sources);
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') {
                        let (sub_headers, sub_sources) = find_c_files(&path);
                        headers.extend(sub_headers);
                        sources.extend(sub_sources);
                    }
                }
            } else {
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some("h") => headers.push(path),
                    Some("c") => sources.push(path),
                    _ => {}
                }
            }
        }
    }

    headers.sort();
    sources.sort();
    (headers, sources)
}

fn extract_from_c_headers(
    files: &[PathBuf],
    root_dir: &Path,
    package_name: &str,
) -> TldrResult<Vec<ApiEntry>> {
    let mut apis = Vec::new();

    for file_path in files {
        let source = std::fs::read_to_string(file_path).map_err(|e| {
            crate::error::TldrError::parse_error(
                file_path.to_path_buf(),
                None,
                format!("Cannot read: {}", e),
            )
        })?;

        let module_path = compute_module_path(file_path, root_dir, package_name);
        let relative_path = file_path
            .strip_prefix(root_dir)
            .unwrap_or(file_path)
            .to_path_buf();

        let mut comments = Vec::new();
        let mut current = String::new();
        let mut start_line = 1usize;

        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw_line.trim();

            if line.starts_with("//") {
                comments.push(line.trim_start_matches("//").trim().to_string());
                continue;
            }
            if line.is_empty() {
                comments.clear();
                continue;
            }
            if line.starts_with('#') {
                comments.clear();
                continue;
            }

            if current.is_empty() {
                start_line = line_no;
            }
            current.push_str(line);
            current.push(' ');

            if !line.ends_with(';') {
                continue;
            }

            if let Some((name, params, return_type)) = parse_c_prototype(&current) {
                let params = params
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
                    qualified_name: format!("{}.{}", module_path, name),
                    kind: ApiKind::Function,
                    module: module_path.clone(),
                    signature: Some(Signature {
                        params: params.clone(),
                        return_type: return_type.clone(),
                        is_async: false,
                        is_generator: false,
                    }),
                    docstring: (!comments.is_empty()).then(|| comments.join(" ")),
                    example: Some(format!(
                        "{}({})",
                        name,
                        params
                            .iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    triggers: extract_triggers(&name, None),
                    is_property: false,
                    return_type,
                    location: Some(Location {
                        file: relative_path.clone(),
                        line: start_line,
                        column: None,
                    }),
                });
            }

            current.clear();
            comments.clear();
        }
    }

    Ok(apis)
}

fn parse_c_prototype(decl: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let decl = decl.trim().trim_end_matches(';').trim();
    if decl.starts_with("typedef")
        || decl.starts_with("struct")
        || decl.starts_with("enum")
        || decl.starts_with("union")
        || !decl.contains('(')
        || decl.contains("(*")
    {
        return None;
    }

    let open = decl.find('(')?;
    let close = decl.rfind(')')?;
    let prefix = decl[..open].trim();
    let params_src = decl[open + 1..close].trim();
    let name = prefix
        .split_whitespace()
        .last()?
        .trim_start_matches('*')
        .to_string();

    let return_type = prefix
        .strip_suffix(&name)
        .map(|s| s.trim().trim_end_matches('*').trim().to_string())
        .filter(|s| !s.is_empty());

    let params = if params_src.is_empty() || params_src == "void" {
        Vec::new()
    } else {
        params_src
            .split(',')
            .map(|part| {
                part.split_whitespace()
                    .last()
                    .unwrap_or(part)
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            })
            .collect()
    };

    Some((name, params, return_type))
}

fn extract_from_c_source_file(
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

    let tree = parse(&source, Language::C)?;
    let module_info = extract_from_tree(&tree, &source, Language::C, file_path, Some(root_dir))?;
    let module_path = compute_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();
    for func in module_info.functions {
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
            qualified_name: format!("{}.{}", module_path, func.name),
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
                func.name,
                params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            triggers: extract_triggers(&func.name, None),
            is_property: false,
            return_type: func.return_type,
            location: Some(Location {
                file: relative_path.clone(),
                line: func.line_number as usize,
                column: None,
            }),
        });
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
    fn test_headers_define_c_surface() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "include/api.h", "int add(int a, int b);\n");
        write_file(&dir, "src/api.c", "int hidden(int x) { return x; }\n");

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_c_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|name| name.ends_with(".add")));
        assert!(!names.iter().any(|name| name.ends_with(".hidden")));
    }

    #[test]
    fn test_c_falls_back_to_source_when_no_headers() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/api.c",
            "int add(int a, int b) { return a + b; }\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_c_api_surface(&resolved, false, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(".add")));
    }
}
