//! PHP-specific API surface extraction.
//!
//! PHP top-level functions are public by default. Class methods are public
//! unless marked `private` or `protected`.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::is_noise_dir;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public PHP API surface for a resolved package.
pub fn extract_php_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_php_files(&resolved.root_dir) {
        apis.extend(extract_from_php_file(
            &file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?);
    }

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "php".to_string(),
        total,
        apis,
    })
}

fn find_php_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "php")
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !is_noise_dir(Language::Php, name) {
                        files.extend(find_php_files(&path));
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("php") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_php_file(
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

    let tree = parse(&source, Language::Php)?;
    let module_info = extract_from_tree(&tree, &source, Language::Php, file_path, Some(root_dir))?;
    let module_path = compute_php_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for func in &module_info.functions {
        let params = convert_php_params(&func.params);
        let return_type = func.return_type.clone();
        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, func.name),
            kind: ApiKind::Function,
            module: module_path.clone(),
            signature: Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: false,
                is_generator: false,
            }),
            docstring: func.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_php_function_example(
                &module_path,
                &func.name,
                &params,
            )),
            triggers: extract_triggers(&func.name, func.docstring.as_deref()),
            is_property: false,
            return_type,
            location: Some(Location {
                file: relative_path.clone(),
                line: func.line_number as usize,
                column: None,
            }),
        });
    }

    for class in &module_info.classes {
        let class_name = effective_php_class_name(class, &source);
        let qualified_name = format!("{}.{}", module_path, class_name);
        let kind = determine_php_kind(class, &source);

        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_php_type_example(&class_name, kind)),
            triggers: extract_triggers(&class_name, class.docstring.as_deref()),
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        for method in &class.methods {
            if !include_private && is_php_method_hidden(method) {
                continue;
            }

            let params = convert_php_params(&method.params);
            let return_type = method.return_type.clone();
            let kind = if method
                .decorators
                .iter()
                .any(|decorator| decorator == "static")
            {
                ApiKind::StaticMethod
            } else {
                ApiKind::Method
            };

            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", qualified_name, method.name),
                kind,
                module: module_path.clone(),
                signature: Some(Signature {
                    params: params.clone(),
                    return_type: return_type.clone(),
                    is_async: false,
                    is_generator: false,
                }),
                docstring: method.docstring.clone().map(|doc| truncate_docstring(&doc)),
                example: Some(generate_php_method_example(
                    &class_name,
                    &method.name,
                    &params,
                )),
                triggers: extract_triggers(&method.name, method.docstring.as_deref()),
                is_property: false,
                return_type,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: method.line_number as usize,
                    column: None,
                }),
            });
        }
    }

    for constant in &module_info.constants {
        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, constant.name),
            kind: ApiKind::Constant,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}::{}", module_path, constant.name)),
            triggers: extract_triggers(&constant.name, None),
            is_property: false,
            return_type: constant.field_type.clone(),
            location: Some(Location {
                file: relative_path.clone(),
                line: constant.line_number as usize,
                column: None,
            }),
        });
    }

    Ok(apis)
}

fn compute_php_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let mut parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();

    while matches!(parts.first().map(|part| part.as_str()), Some("src")) {
        parts.remove(0);
    }

    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

fn is_php_method_hidden(method: &crate::types::FunctionInfo) -> bool {
    method
        .decorators
        .iter()
        .any(|decorator| decorator == "private" || decorator == "protected")
}

fn effective_php_class_name(class: &ClassInfo, source: &str) -> String {
    if !class.name.is_empty() {
        return class.name.clone();
    }

    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("");
    let tokens: Vec<&str> = line
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|token| !token.is_empty())
        .collect();

    for idx in 0..tokens.len() {
        match tokens[idx] {
            "class" | "interface" | "trait" => {
                if let Some(name) = tokens.get(idx + 1) {
                    return (*name).to_string();
                }
            }
            _ => {}
        }
    }

    String::new()
}

fn determine_php_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim_start();

    if line.starts_with("interface ") || line.contains(" interface ") {
        ApiKind::Interface
    } else if line.starts_with("trait ") || line.contains(" trait ") {
        ApiKind::Trait
    } else {
        ApiKind::Class
    }
}

fn convert_php_params(raw_params: &[String]) -> Vec<Param> {
    raw_params
        .iter()
        .filter(|param| !param.is_empty())
        .map(|param| Param {
            name: param.trim_start_matches('$').to_string(),
            type_annotation: None,
            default: None,
            is_variadic: false,
            is_keyword: false,
        })
        .collect()
}

fn truncate_docstring(doc: &str) -> String {
    let first_para = doc.split("\n\n").next().unwrap_or(doc);
    let cleaned = first_para
        .replace("/**", "")
        .replace("*/", "")
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    if cleaned.len() <= 200 {
        cleaned
    } else {
        let mut end = 197;
        while end > 0 && !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &cleaned[..end])
    }
}

fn generate_php_function_example(module_path: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| format!("${}", param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}\\{}({})", module_path, func_name, args)
}

fn generate_php_type_example(class_name: &str, kind: ApiKind) -> String {
    match kind {
        ApiKind::Trait => class_name.to_string(),
        _ => format!("$value = new {}();", class_name),
    }
}

fn generate_php_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| format!("${}", param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("(new {}())->{}({})", class_name, method_name, args)
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
    fn test_find_php_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "src/index.php", "<?php function hi() {}");
        write_file(&dir, "tests/test.php", "<?php function test() {}");

        let files = find_php_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], dir.path().join("src/index.php"));
    }

    #[test]
    fn test_find_php_files_skips_noise_directories() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "src/index.php", "<?php function real_api() {}");
        write_file(&dir, "examples/demo.php", "<?php function sample_api() {}");
        write_file(
            &dir,
            "vendor/pkg/noise.php",
            "<?php function vendored_api() {}",
        );

        let files = find_php_files(dir.path());

        assert_eq!(files, vec![dir.path().join("src/index.php")]);
    }

    #[test]
    fn test_extract_php_surface_filters_private_methods() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/App.php",
            r#"
<?php
function greet($name) { return $name; }

class Greeter {
    public function hello($name) { return $name; }
    protected function hide($name) { return $name; }
    private static function secret($name) { return $name; }
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_php_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with(".greet")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.hide")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
    }

    #[test]
    fn test_extract_php_surface_includes_private_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/App.php",
            r#"
<?php
class Greeter {
    private function secret($name) { return $name; }
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_php_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with("Greeter.secret")));
    }

    #[test]
    fn test_truncate_docstring_handles_unicode_char_boundaries() {
        let doc = format!("/**\n * {}\n */", "─".repeat(67));

        let truncated = truncate_docstring(&doc);

        assert!(truncated.ends_with("..."));
        assert_eq!(truncated, format!("{}...", "─".repeat(65)));
    }
}
