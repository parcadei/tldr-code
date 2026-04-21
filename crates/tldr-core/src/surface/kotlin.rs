//! Kotlin-specific API surface extraction.
//!
//! Kotlin declarations are public by default. `private` and `internal`
//! declarations are excluded unless `include_private` is set.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::strip_layout_segments;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Kotlin API surface for a resolved package.
pub fn extract_kotlin_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_kotlin_files(&resolved.root_dir) {
        apis.extend(extract_from_kotlin_file(
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
        language: "kotlin".to_string(),
        total,
        apis,
    })
}

fn find_kotlin_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "kt" || *ext == "kts")
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
                        files.extend(find_kotlin_files(&path));
                    }
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("kt" | "kts")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_kotlin_file(
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

    let tree = parse(&source, Language::Kotlin)?;
    let module_info =
        extract_from_tree(&tree, &source, Language::Kotlin, file_path, Some(root_dir))?;
    let module_path = compute_kotlin_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for func in &module_info.functions {
        if !include_private && is_kotlin_hidden_at_line(&source, func.line_number as usize) {
            continue;
        }

        let params = convert_kotlin_params(&func.params);
        let return_type = func.return_type.clone();
        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, func.name),
            kind: ApiKind::Function,
            module: module_path.clone(),
            signature: Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: func.is_async,
                is_generator: false,
            }),
            docstring: func.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_kotlin_call_example(
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
        if !include_private && is_kotlin_hidden_at_line(&source, class.line_number as usize) {
            continue;
        }

        let class_name = effective_kotlin_class_name(class, &source);
        if class_name.is_empty() {
            continue;
        }
        let qualified_name = format!("{}.{}", module_path, class_name);
        let kind = determine_kotlin_kind(class, &source);

        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_kotlin_type_example(&class_name, kind)),
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
            if !include_private && is_kotlin_hidden_at_line(&source, method.line_number as usize) {
                continue;
            }

            let params = convert_kotlin_params(&method.params);
            let return_type = method.return_type.clone();
            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", qualified_name, method.name),
                kind: ApiKind::Method,
                module: module_path.clone(),
                signature: Some(Signature {
                    params: params.clone(),
                    return_type: return_type.clone(),
                    is_async: method.is_async,
                    is_generator: false,
                }),
                docstring: method.docstring.clone().map(|doc| truncate_docstring(&doc)),
                example: Some(generate_kotlin_method_example(
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
        if !include_private && is_kotlin_hidden_at_line(&source, constant.line_number as usize) {
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

fn compute_kotlin_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    let parts = strip_layout_segments(Language::Kotlin, Path::new(&parts.join("/")));

    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

fn is_kotlin_hidden_at_line(source: &str, line_number: usize) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| line.contains("private ") || line.contains("internal "))
        .unwrap_or(false)
}

fn determine_kotlin_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim_start();

    if line.starts_with("interface ") || line.contains(" interface ") {
        ApiKind::Interface
    } else if line.starts_with("enum class ") || line.contains(" enum class ") {
        ApiKind::Enum
    } else {
        ApiKind::Class
    }
}

fn effective_kotlin_class_name(class: &ClassInfo, source: &str) -> String {
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
            "class" | "object" | "interface" => {
                if let Some(name) = tokens.get(idx + 1) {
                    return (*name).to_string();
                }
            }
            "enum" => {
                if tokens.get(idx + 1) == Some(&"class") {
                    if let Some(name) = tokens.get(idx + 2) {
                        return (*name).to_string();
                    }
                }
            }
            _ => {}
        }
    }

    String::new()
}

fn convert_kotlin_params(raw_params: &[String]) -> Vec<Param> {
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

fn generate_kotlin_call_example(module_path: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", module_path, func_name, args)
}

fn generate_kotlin_type_example(name: &str, kind: ApiKind) -> String {
    match kind {
        ApiKind::Interface => format!("val value: {} = TODO()", name),
        _ => format!("val value = {}()", name),
    }
}

fn generate_kotlin_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{}.{}({})",
        class_name.replace('.', "").to_lowercase(),
        method_name,
        args
    )
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
    fn test_find_kotlin_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "src/main/kotlin/App.kt", "class App");
        write_file(&dir, "build.gradle.kts", "plugins {}");

        let files = find_kotlin_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_extract_kotlin_surface_filters_private_and_internal() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/kotlin/com/example/App.kt",
            r#"
const val VERSION = "1"
private const val HIDDEN = "no"

fun greet(name: String): String = name
internal fun debug(name: String): String = name

class Greeter {
    fun hello(name: String): String = name
    private fun secret(token: String): String = token
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_kotlin_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with(".greet")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(names.iter().any(|name| name.ends_with(".VERSION")));
        assert!(!names.iter().any(|name| name.ends_with(".debug")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
        assert!(!names.iter().any(|name| name.ends_with(".HIDDEN")));
    }

    #[test]
    fn test_extract_kotlin_surface_includes_internal_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/kotlin/com/example/App.kt",
            r#"
internal fun debug(name: String): String = name
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_kotlin_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(".debug")));
    }

    #[test]
    fn test_compute_kotlin_module_path_strips_nested_common_main_kotlin_prefix() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/sdk/src/commonMain/kotlin/com/example/core/Client.kt");

        assert_eq!(
            compute_kotlin_module_path(file, root, "example_pkg"),
            "example_pkg.sdk.com.example.core"
        );
    }

    #[test]
    fn test_compute_kotlin_module_path_strips_nested_jvm_main_kotlin_prefix() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/runtime/src/jvmMain/kotlin/com/example/io/Streams.kt");

        assert_eq!(
            compute_kotlin_module_path(file, root, "example_pkg"),
            "example_pkg.runtime.com.example.io"
        );
    }
}
