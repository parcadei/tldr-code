//! C++-specific API surface extraction.
//!
//! When headers are present, public class methods and namespace-level function
//! declarations are derived from header files. If no headers are present, we
//! fall back to function definitions in source files.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::Language;
use crate::TldrResult;

use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public C++ API surface for a resolved package.
pub fn extract_cpp_api_surface(
    resolved: &ResolvedPackage,
    _include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let (headers, sources) = find_cpp_files(&resolved.root_dir);
    let mut apis = if !headers.is_empty() {
        extract_from_cpp_headers(&headers, &resolved.root_dir, &resolved.package_name)?
    } else {
        let mut collected = Vec::new();
        for file_path in sources {
            collected.extend(extract_from_cpp_source_file(
                &file_path,
                &resolved.root_dir,
                &resolved.package_name,
            )?);
        }
        collected
    };

    apis.retain(is_plausible_cpp_public_api);

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "cpp".to_string(),
        total,
        apis,
    })
}

fn find_cpp_files(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut headers = Vec::new();
    let mut sources = Vec::new();

    if dir.is_file() {
        match dir.extension().and_then(|ext| ext.to_str()) {
            Some(ext) if is_cpp_header_ext(ext) => headers.push(dir.to_path_buf()),
            Some(ext) if is_cpp_source_ext(ext) => sources.push(dir.to_path_buf()),
            _ => {}
        }
        return (headers, sources);
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !is_cpp_noise_dir(name) {
                        let (sub_headers, sub_sources) = find_cpp_files(&path);
                        headers.extend(sub_headers);
                        sources.extend(sub_sources);
                    }
                }
            } else {
                match path.extension().and_then(|ext| ext.to_str()) {
                    Some(ext) if is_cpp_header_ext(ext) => headers.push(path),
                    Some(ext) if is_cpp_source_ext(ext) => sources.push(path),
                    _ => {}
                }
            }
        }
    }

    if dir.is_dir() {
        headers = prefer_package_facing_headers(headers, dir);
    }
    headers.sort();
    sources.sort();
    (headers, sources)
}

fn is_plausible_cpp_public_api(api: &ApiEntry) -> bool {
    let segments: Vec<&str> = api.qualified_name.split('.').collect();
    let Some(symbol) = api.qualified_name.rsplit('.').next() else {
        return false;
    };

    if symbol.is_empty()
        || symbol.starts_with('~')
        || !symbol.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
    {
        return false;
    }

    let is_top_level = segments.len() <= 2;

    if is_top_level
        && matches!(
            symbol,
            "unwrap"
                | "node"
                | "typed_node"
                | "push"
                | "push_back"
                | "clear"
                | "reserve"
                | "data"
                | "need_copy"
                | "emplace_arg"
                | "dynamic_arg_list"
                | "void_t_impl"
                | "ignore_unused"
                | "accessor"
        )
    {
        return false;
    }

    true
}

fn is_cpp_header_ext(ext: &str) -> bool {
    matches!(ext, "h" | "hpp" | "hh" | "hxx")
}

fn is_cpp_source_ext(ext: &str) -> bool {
    matches!(ext, "cpp" | "cc" | "cxx")
}

fn is_cpp_noise_dir(name: &str) -> bool {
    matches!(
        name,
        "bench"
            | "benches"
            | "benchmark"
            | "benchmarks"
            | "doc"
            | "docs"
            | "example"
            | "examples"
            | "fuzz"
            | "fuzzing"
            | "test"
            | "tests"
    )
}

fn prefer_package_facing_headers(headers: Vec<PathBuf>, root_dir: &Path) -> Vec<PathBuf> {
    let include_headers: Vec<PathBuf> = headers
        .iter()
        .filter(|path| {
            path.strip_prefix(root_dir)
                .ok()
                .and_then(|relative| relative.components().next())
                .is_some_and(|component| component.as_os_str() == "include")
        })
        .cloned()
        .collect();

    if include_headers.is_empty() {
        headers
    } else {
        include_headers
    }
}

#[derive(Clone)]
enum Scope {
    Namespace { internal: bool },
    Class { name: String, public: bool },
}

fn extract_from_cpp_headers(
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

        let mut scopes: Vec<Scope> = Vec::new();

        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw_line.trim();
            if let Some(namespace_name) = parse_namespace_name(line) {
                let internal = is_internal_cpp_namespace(&namespace_name);
                scopes.push(Scope::Namespace { internal });
                continue;
            }

            if let Some((name, is_struct)) = parse_class_or_struct_name(line) {
                scopes.push(Scope::Class {
                    name: name.clone(),
                    public: is_struct,
                });

                if !is_internal_scope(&scopes) {
                    apis.push(ApiEntry {
                        qualified_name: format!("{}.{}", module_path, name),
                        kind: if is_struct {
                            ApiKind::Struct
                        } else {
                            ApiKind::Class
                        },
                        module: module_path.clone(),
                        signature: None,
                        docstring: None,
                        example: Some(format!("{} value;", name)),
                        triggers: extract_triggers(&name, None),
                        is_property: false,
                        return_type: None,
                        location: Some(Location {
                            file: relative_path.clone(),
                            line: line_no,
                            column: None,
                        }),
                    });
                }
                continue;
            }

            if line == "public:" {
                if let Some(Scope::Class { public, .. }) = scopes.last_mut() {
                    *public = true;
                }
                continue;
            }
            if line == "private:" || line == "protected:" {
                if let Some(Scope::Class { public, .. }) = scopes.last_mut() {
                    *public = false;
                }
                continue;
            }

            if (line.ends_with(';') || line.ends_with('{'))
                && line.contains('(')
                && line.contains(')')
                && !line.contains("operator")
            {
                let candidate = line.trim_end_matches('{').trim();
                if let Some((name, params, return_type)) = parse_cpp_prototype(candidate) {
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

                    match scopes.last() {
                        Some(Scope::Class {
                            name: class_name,
                            public: true,
                        }) => {
                            if !is_internal_scope(&scopes)
                                && !is_cpp_ctor_or_dtor(&name, class_name)
                            {
                                apis.push(ApiEntry {
                                    qualified_name: format!(
                                        "{}.{}.{}",
                                        module_path, class_name, name
                                    ),
                                    kind: ApiKind::Method,
                                    module: module_path.clone(),
                                    signature: Some(Signature {
                                        params: params.clone(),
                                        return_type: return_type.clone(),
                                        is_async: false,
                                        is_generator: false,
                                    }),
                                    docstring: None,
                                    example: Some(format!(
                                        "{}.{}({})",
                                        class_name.to_lowercase(),
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
                                        line: line_no,
                                        column: None,
                                    }),
                                });
                            }
                        }
                        Some(Scope::Class { public: false, .. }) => {}
                        _ => {
                            if !is_internal_scope(&scopes) && !name.starts_with('~') {
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
                                    docstring: None,
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
                                        line: line_no,
                                        column: None,
                                    }),
                                });
                            }
                        }
                    }
                }
            }

            let close_count = line.chars().filter(|ch| *ch == '}').count();
            for _ in 0..close_count {
                scopes.pop();
            }
        }
    }

    Ok(apis)
}

fn parse_namespace_name(line: &str) -> Option<String> {
    if !line.ends_with('{') || !line.contains("namespace ") {
        return None;
    }

    let start = line.find("namespace ")? + "namespace ".len();
    let name = line[start..]
        .trim()
        .trim_end_matches('{')
        .trim()
        .trim_end_matches("::")
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn parse_class_or_struct_name(line: &str) -> Option<(String, bool)> {
    if !line.ends_with('{') {
        return None;
    }

    let (keyword, is_struct) = if let Some(index) = line.find("class ") {
        (index, false)
    } else if let Some(index) = line.find("struct ") {
        (index, true)
    } else {
        return None;
    };

    let after_keyword = &line[keyword + if is_struct { 7 } else { 6 }..];
    let name = after_keyword
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches('{')
        .trim_end_matches(':')
        .trim();
    if is_cpp_api_name(name) {
        Some((name.to_string(), is_struct))
    } else {
        None
    }
}

fn is_internal_scope(scopes: &[Scope]) -> bool {
    scopes
        .iter()
        .any(|scope| matches!(scope, Scope::Namespace { internal: true, .. }))
}

fn is_internal_cpp_namespace(name: &str) -> bool {
    matches!(name, "detail" | "internal" | "impl")
}

fn parse_cpp_prototype(decl: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let decl = decl.trim().trim_end_matches(';').trim();
    if decl.contains("(*")
        || !decl.contains('(')
        || decl.starts_with("return ")
        || decl.starts_with("if ")
        || decl.starts_with("for ")
        || decl.starts_with("while ")
        || decl.starts_with("switch ")
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
        .split("::")
        .last()?
        .to_string();
    if !is_cpp_api_name(&name) || is_cpp_non_api_keyword(&name) {
        return None;
    }
    let return_type = prefix
        .strip_suffix(&name)
        .map(|s| s.trim().trim_end_matches(':').trim().to_string())
        .filter(|s| !s.is_empty());
    if return_type
        .as_deref()
        .is_some_and(|ty| ty.contains("return") || ty.contains('=') || ty.starts_with("using "))
    {
        return None;
    }
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
    if params.iter().any(|param| !is_cpp_param_name(param)) {
        return None;
    }
    Some((name, params, return_type))
}

fn is_cpp_api_name(name: &str) -> bool {
    if name.is_empty()
        || name.contains('.')
        || name.contains("->")
        || name.contains('<')
        || name.contains('>')
        || name.contains('"')
        || name.contains('\'')
        || name.contains('{')
        || name.contains('}')
        || name.contains('[')
        || name.contains(']')
        || name.contains('=')
        || name.contains('!')
    {
        return false;
    }

    if name.chars().all(|ch| ch.is_ascii_uppercase() || ch == '_') {
        return false;
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap_or_default();
    if !(first == '~' || first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_cpp_non_api_keyword(name: &str) -> bool {
    matches!(name, "decltype" | "sizeof" | "static_assert")
}

fn is_cpp_ctor_or_dtor(name: &str, class_name: &str) -> bool {
    name == class_name || name == format!("~{}", class_name)
}

fn is_cpp_param_name(param: &str) -> bool {
    let trimmed = param.trim();
    if trimmed == "..." {
        return true;
    }
    if trimmed.is_empty()
        || trimmed.contains('.')
        || trimmed.contains("->")
        || trimmed.contains('<')
        || trimmed.contains('>')
        || trimmed.contains('(')
        || trimmed.contains(')')
        || trimmed.contains('"')
        || trimmed.contains('\'')
        || trimmed.contains('{')
        || trimmed.contains('}')
        || trimmed.contains('[')
        || trimmed.contains(']')
        || trimmed.contains('=')
        || trimmed.contains(':')
    {
        return false;
    }

    let mut chars = trimmed.chars();
    let first = chars.next().unwrap_or_default();
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }

    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn extract_from_cpp_source_file(
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

    let tree = parse(&source, Language::Cpp)?;
    let module_info = extract_from_tree(&tree, &source, Language::Cpp, file_path, Some(root_dir))?;
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
    let mut parts: Vec<String> = parent
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    if parts
        .first()
        .is_some_and(|part| part == "include" || part == "src")
    {
        parts.remove(0);
    }
    if parts
        .first()
        .is_some_and(|part| part.eq_ignore_ascii_case(package_name))
    {
        parts.remove(0);
    }
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
    fn test_cpp_headers_define_surface() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "include/api.hpp",
            r#"
class Greeter {
public:
  void hello(int name);
private:
  void secret(int name);
};

int add(int a, int b);
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_cpp_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|name| name.ends_with("Greeter")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(names.iter().any(|name| name.ends_with(".add")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
    }

    #[test]
    fn test_cpp_prefers_include_h_headers_over_source_noise() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "include/fmt/format.h",
            r#"
class formatter {
public:
  void format(int value);
};
"#,
        );
        write_file(
            &dir,
            "src/format.cc",
            r#"
int internal_helper(int value) { return value; }
"#,
        );
        write_file(
            &dir,
            "test/format-test.cc",
            r#"
int test_only_noise() { return 0; }
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "fmt".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_cpp_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.contains(&"fmt.formatter"));
        assert!(names.contains(&"fmt.formatter.format"));
        assert!(!names.iter().any(|name| name.contains("internal_helper")));
        assert!(!names.iter().any(|name| name.contains("test_only_noise")));
        assert!(!names.iter().any(|name| name.contains(".include.")));
    }

    #[test]
    fn test_cpp_ignores_inline_statements_and_macro_calls_in_headers() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "include/fmt/base.h",
            r#"
class formatter {
public:
  void clear() {
    data_.clear();
    FMT_ASSERT(value, "msg");
    return helper();
  }

  formatter();
  ~formatter();
};
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "fmt".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_cpp_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.contains(&"fmt.formatter"));
        assert!(names.contains(&"fmt.formatter.clear"));
        assert!(!names.iter().any(|name| name.contains("data_.clear")));
        assert!(!names.iter().any(|name| name.contains("FMT_ASSERT")));
        assert!(!names.iter().any(|name| name.contains("helper")));
    }

    #[test]
    fn test_cpp_skips_detail_namespace_and_constructor_destructor_noise() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "include/fmt/base.h",
            r#"
namespace detail {
template <typename = void> struct node {
  virtual ~node() = default;
};

class dynamic_arg_list {
 public:
  template <typename T> auto push(const T& arg) -> const T& {
    return arg;
  }
};
}  // namespace detail

template <typename Context> class dynamic_format_arg_store {
 private:
  auto data() const -> const int* { return nullptr; }
  template <typename T> void emplace_arg(const T& arg) {}

 public:
  constexpr dynamic_format_arg_store() = default;
  template <typename T> void push_back(const T& arg) {}
};

template <typename T, typename Char = char> struct formatter {
  formatter() = delete;
};
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "fmt".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_cpp_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.contains(&"fmt.dynamic_format_arg_store"));
        assert!(names.contains(&"fmt.formatter"));
        assert!(!names.iter().any(|name| name.contains("~node")));
        assert!(!names.iter().any(|name| name.contains(".push")));
        assert!(!names.iter().any(|name| name.ends_with(".data")));
        assert!(!names.iter().any(|name| name.contains("emplace_arg")));
        assert!(!names
            .iter()
            .any(|name| name.contains("dynamic_format_arg_store.dynamic_format_arg_store")));
        assert!(!names
            .iter()
            .any(|name| name.contains("formatter.formatter")));
    }
}
