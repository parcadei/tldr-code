//! Java-specific API surface extraction.
//!
//! Java surfaces only `public` declarations by default. When `include_private`
//! is set, package-private, protected, and private members are also returned.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::strip_layout_segments;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Java API surface for a resolved package.
pub fn extract_java_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_java_files(&resolved.root_dir) {
        apis.extend(extract_from_java_file(
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
        language: "java".to_string(),
        total,
        apis,
    })
}

fn find_java_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "java")
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
                        files.extend(find_java_files(&path));
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("java") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_java_file(
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

    let tree = parse(&source, Language::Java)?;
    let module_info = extract_from_tree(&tree, &source, Language::Java, file_path, Some(root_dir))?;
    let module_path = compute_java_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for class in &module_info.classes {
        if !include_private && !is_java_public_at_line(&source, class.line_number as usize) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, class.name);
        let kind = determine_java_kind(class, &source);

        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_java_type_example(&class.name, kind)),
            triggers: extract_triggers(&class.name, class.docstring.as_deref()),
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        for method in &class.methods {
            if !include_private && !is_java_public_at_line(&source, method.line_number as usize) {
                continue;
            }

            let params = convert_java_params(&method.params);
            let return_type = method.return_type.clone();
            let kind = if line_contains_word(&source, method.line_number as usize, "static") {
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
                example: Some(generate_java_method_example(
                    &class.name,
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

        for field in &class.fields {
            let is_public = field.visibility.as_deref() == Some("public");
            if !include_private && !is_public {
                continue;
            }

            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", qualified_name, field.name),
                kind: if field.is_constant {
                    ApiKind::Constant
                } else {
                    ApiKind::Property
                },
                module: module_path.clone(),
                signature: None,
                docstring: None,
                example: Some(format!("{}.{}", class.name, field.name)),
                triggers: extract_triggers(&field.name, None),
                is_property: !field.is_constant,
                return_type: field.field_type.clone(),
                location: Some(Location {
                    file: relative_path.clone(),
                    line: field.line_number as usize,
                    column: None,
                }),
            });
        }
    }

    Ok(apis)
}

fn compute_java_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    let parts = strip_layout_segments(Language::Java, Path::new(&parts.join("/")));

    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

fn is_java_public_at_line(source: &str, line_number: usize) -> bool {
    line_contains_word(source, line_number, "public")
}

fn line_contains_word(source: &str, line_number: usize, word: &str) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| {
            line.split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                .any(|part| part == word)
        })
        .unwrap_or(false)
}

fn determine_java_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("");

    if line.contains(" interface ") || line.trim_start().starts_with("interface ") {
        ApiKind::Interface
    } else if line.contains(" enum ") || line.trim_start().starts_with("enum ") {
        ApiKind::Enum
    } else {
        ApiKind::Class
    }
}

fn convert_java_params(raw_params: &[String]) -> Vec<Param> {
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

fn generate_java_type_example(name: &str, kind: ApiKind) -> String {
    match kind {
        ApiKind::Interface => format!("{} value = null;", name),
        ApiKind::Enum => format!("{} value = {}.values()[0];", name, name),
        _ => format!("{} value = new {}();", name, name),
    }
}

fn generate_java_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
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
    fn test_find_java_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/java/com/example/App.java",
            "public class App {}",
        );
        write_file(
            &dir,
            "src/test/java/com/example/AppTest.java",
            "class AppTest {}",
        );

        let files = find_java_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_extract_java_surface_filters_package_private_members() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/java/com/example/Greeter.java",
            r#"
public class Greeter {
    public static final String VERSION = "1";
    String hidden = "no";

    public String hello(String name) {
        return name;
    }

    String secret(String token) {
        return token;
    }
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_java_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Greeter")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.VERSION")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.hidden")));
    }

    #[test]
    fn test_extract_java_surface_includes_non_public_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "src/main/java/com/example/Helper.java",
            r#"
class Helper {
    String value() {
        return "ok";
    }
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_java_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with("Helper.value")));
    }

    #[test]
    fn test_compute_java_module_path_strips_nested_src_main_java_prefix() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/module-a/src/main/java/com/example/http/Client.java");

        assert_eq!(
            compute_java_module_path(file, root, "example_pkg"),
            "example_pkg.module-a.com.example.http"
        );
    }
}
