//! Scala-specific API surface extraction.
//!
//! Scala declarations are public by default. `private` and `protected`
//! declarations are excluded unless `include_private` is set.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::strip_layout_segments;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Scala API surface for a resolved package.
pub fn extract_scala_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_scala_files(&resolved.root_dir) {
        apis.extend(extract_from_scala_file(
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
        language: "scala".to_string(),
        total,
        apis,
    })
}

fn find_scala_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "scala")
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
                        files.extend(find_scala_files(&path));
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("scala") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_scala_file(
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

    let tree = parse(&source, Language::Scala)?;
    let module_info =
        extract_from_tree(&tree, &source, Language::Scala, file_path, Some(root_dir))?;
    let module_path = compute_scala_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for func in &module_info.functions {
        if !include_private && is_scala_hidden_at_line(&source, func.line_number as usize) {
            continue;
        }

        let params = convert_scala_params(&func.params);
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
            example: Some(generate_scala_call_example(
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
        if !include_private && is_scala_hidden_at_line(&source, class.line_number as usize) {
            continue;
        }

        let class_name = effective_scala_class_name(class, &source);
        let qualified_name = format!("{}.{}", module_path, class_name);
        let kind = determine_scala_kind(class, &source);

        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_scala_type_example(&class_name, kind)),
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
            if !include_private && is_scala_hidden_at_line(&source, method.line_number as usize) {
                continue;
            }

            let params = convert_scala_params(&method.params);
            let return_type = method.return_type.clone();
            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", qualified_name, method.name),
                kind: ApiKind::Method,
                module: module_path.clone(),
                signature: Some(Signature {
                    params: params.clone(),
                    return_type: return_type.clone(),
                    is_async: false,
                    is_generator: false,
                }),
                docstring: method.docstring.clone().map(|doc| truncate_docstring(&doc)),
                example: Some(generate_scala_method_example(
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
        if !include_private && is_scala_hidden_at_line(&source, constant.line_number as usize) {
            continue;
        }

        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, constant.name),
            kind: ApiKind::Constant,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}.{}", module_path, constant.name)),
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

fn compute_scala_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    let parts = strip_layout_segments(Language::Scala, Path::new(&parts.join("/")));

    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

fn is_scala_hidden_at_line(source: &str, line_number: usize) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| line.contains("private ") || line.contains("protected "))
        .unwrap_or(false)
}

fn effective_scala_class_name(class: &ClassInfo, source: &str) -> String {
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
            "class" | "object" | "trait" => {
                if let Some(name) = tokens.get(idx + 1) {
                    return (*name).to_string();
                }
            }
            _ => {}
        }
    }

    String::new()
}

fn determine_scala_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim_start();

    if line.starts_with("trait ") || line.contains(" trait ") {
        ApiKind::Trait
    } else {
        ApiKind::Class
    }
}

fn convert_scala_params(raw_params: &[String]) -> Vec<Param> {
    raw_params
        .iter()
        .filter(|param| !param.is_empty())
        .map(|param| Param {
            name: param.clone(),
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
        .join(" ");
    if cleaned.len() <= 200 {
        cleaned
    } else {
        format!("{}...", &cleaned[..197])
    }
}

fn generate_scala_call_example(module_path: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", module_path, func_name, args)
}

fn generate_scala_type_example(name: &str, kind: ApiKind) -> String {
    match kind {
        ApiKind::Trait => format!("val value: {} = ???", name),
        _ => format!("val value = new {}()", name),
    }
}

fn generate_scala_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("new {}().{}({})", class_name, method_name, args)
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
    fn test_find_scala_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "src/main/scala/App.scala", "object App");
        write_file(&dir, "test/AppSpec.scala", "object AppSpec");

        let files = find_scala_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_extract_scala_surface_filters_private_and_protected() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/scala/com/example/App.scala",
            r#"
object Utils {
  val Version = "1"
  def hello(name: String): String = name
  private def secret(token: String): String = token
}

trait Service {
  def run(name: String): String
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_scala_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Utils.hello")));
        assert!(names.iter().any(|name| name.ends_with("Service.run")));
        assert!(!names.iter().any(|name| name.ends_with("Utils.secret")));
    }

    #[test]
    fn test_extract_scala_surface_includes_private_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/scala/com/example/App.scala",
            r#"
object Utils {
  private def secret(token: String): String = token
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_scala_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with("Utils.secret")));
    }

    #[test]
    fn test_compute_scala_module_path_strips_nested_src_main_scala_prefix() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/cli/src/main/scala/com/example/tooling/App.scala");

        assert_eq!(
            compute_scala_module_path(file, root, "example_pkg"),
            "example_pkg.cli.com.example.tooling"
        );
    }
}
