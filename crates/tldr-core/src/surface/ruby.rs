//! Ruby-specific API surface extraction.
//!
//! Ruby methods are public by default. Methods that fall under `private` or
//! `protected` sections are excluded unless `include_private` is set.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{FieldInfo, Language};
use crate::TldrResult;

use super::language_profile::is_noise_dir;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Ruby API surface for a resolved package.
pub fn extract_ruby_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let file_selection = select_ruby_surface_files(&resolved.root_dir, &resolved.package_name);
    let mut apis = Vec::new();

    for file_path in &file_selection.files {
        apis.extend(extract_from_ruby_file(
            file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?);
    }

    sort_ruby_apis(&mut apis, &file_selection);

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "ruby".to_string(),
        total,
        apis,
    })
}

#[derive(Debug, Default)]
struct RubyFileSelection {
    files: Vec<PathBuf>,
    entrypoint: Option<PathBuf>,
    depth_by_relative_path: HashMap<PathBuf, usize>,
}

fn select_ruby_surface_files(root_dir: &Path, package_name: &str) -> RubyFileSelection {
    if root_dir.is_file() {
        return RubyFileSelection {
            files: vec![root_dir.to_path_buf()],
            ..RubyFileSelection::default()
        };
    }

    let Some(entrypoint) = find_ruby_package_entrypoint(root_dir, package_name) else {
        return RubyFileSelection {
            files: find_ruby_files(root_dir),
            ..RubyFileSelection::default()
        };
    };

    let entrypoint_relative = relative_ruby_path(root_dir, &entrypoint);
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([(entrypoint.clone(), 0usize)]);
    let mut files = Vec::new();
    let mut depth_by_relative_path: HashMap<PathBuf, usize> = HashMap::new();

    while let Some((file_path, depth)) = queue.pop_front() {
        if !visited.insert(file_path.clone()) {
            continue;
        }

        let relative_path = relative_ruby_path(root_dir, &file_path);
        depth_by_relative_path
            .entry(relative_path)
            .and_modify(|existing_depth: &mut usize| *existing_depth = (*existing_depth).min(depth))
            .or_insert(depth);
        files.push(file_path.clone());

        let Ok(source) = std::fs::read_to_string(&file_path) else {
            continue;
        };

        for dependency in parse_ruby_internal_requires(&source)
            .into_iter()
            .filter_map(|require| {
                resolve_ruby_internal_require(root_dir, &file_path, package_name, &require)
            })
        {
            queue.push_back((dependency, depth + 1));
        }
    }

    files.sort();

    RubyFileSelection {
        files,
        entrypoint: Some(entrypoint_relative),
        depth_by_relative_path,
    }
}

fn find_ruby_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "rb")
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !is_noise_dir(Language::Ruby, name) {
                        files.extend(find_ruby_files(&path));
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rb") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RubyRequireKind {
    Relative,
    Load,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RubyRequire {
    kind: RubyRequireKind,
    target: String,
}

fn parse_ruby_internal_requires(source: &str) -> Vec<RubyRequire> {
    let mut requires = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }

        if let Some(target) = parse_ruby_require_literal(trimmed, "require_relative") {
            requires.push(RubyRequire {
                kind: RubyRequireKind::Relative,
                target,
            });
            continue;
        }

        if let Some(target) = parse_ruby_require_literal(trimmed, "require") {
            requires.push(RubyRequire {
                kind: RubyRequireKind::Load,
                target,
            });
        }
    }

    requires
}

fn parse_ruby_require_literal(line: &str, keyword: &str) -> Option<String> {
    let mut rest = line.strip_prefix(keyword)?.trim_start();

    if let Some(stripped) = rest.strip_prefix('(') {
        rest = stripped.trim_start();
    }

    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }

    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

fn resolve_ruby_internal_require(
    root_dir: &Path,
    current_file: &Path,
    package_name: &str,
    require: &RubyRequire,
) -> Option<PathBuf> {
    match require.kind {
        RubyRequireKind::Relative => {
            resolve_ruby_relative_require(root_dir, current_file, &require.target)
        }
        RubyRequireKind::Load => resolve_ruby_load_require(root_dir, package_name, &require.target),
    }
}

fn resolve_ruby_relative_require(
    root_dir: &Path,
    current_file: &Path,
    target: &str,
) -> Option<PathBuf> {
    let base_dir = current_file.parent().unwrap_or(root_dir);
    let candidate = ensure_ruby_extension(&normalize_path(&base_dir.join(target)));
    is_ruby_file_within_root(root_dir, &candidate).then_some(candidate)
}

fn resolve_ruby_load_require(root_dir: &Path, package_name: &str, target: &str) -> Option<PathBuf> {
    let target = target.trim_end_matches(".rb");
    let prefixes = ruby_package_require_prefixes(package_name);
    if !prefixes
        .iter()
        .any(|prefix| target == prefix || target.starts_with(&format!("{prefix}/")))
    {
        return None;
    }

    let candidate = if root_dir.file_name().and_then(|name| name.to_str()) == Some("lib") {
        ensure_ruby_extension(&normalize_path(&root_dir.join(target)))
    } else {
        ensure_ruby_extension(&normalize_path(&root_dir.join("lib").join(target)))
    };

    is_ruby_file_within_root(root_dir, &candidate).then_some(candidate)
}

fn is_ruby_file_within_root(root_dir: &Path, candidate: &Path) -> bool {
    candidate.extension().and_then(|ext| ext.to_str()) == Some("rb")
        && candidate.is_file()
        && candidate.starts_with(root_dir)
}

fn ensure_ruby_extension(path: &Path) -> PathBuf {
    if path.extension().and_then(|ext| ext.to_str()) == Some("rb") {
        path.to_path_buf()
    } else {
        path.with_extension("rb")
    }
}

fn find_ruby_package_entrypoint(root_dir: &Path, package_name: &str) -> Option<PathBuf> {
    ruby_package_entrypoint_candidates(package_name)
        .into_iter()
        .map(|candidate| normalize_path(&root_dir.join(candidate)))
        .find(|candidate| candidate.is_file())
}

fn ruby_package_entrypoint_candidates(package_name: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for variant in ruby_package_path_variants(package_name) {
        let path = variant.join("/");
        if path.is_empty() {
            continue;
        }

        for candidate in [format!("lib/{path}.rb"), format!("{path}.rb")] {
            if seen.insert(candidate.clone()) {
                candidates.push(PathBuf::from(candidate));
            }
        }
    }

    candidates
}

fn ruby_package_path_variants(package_name: &str) -> Vec<Vec<String>> {
    let mut seen = HashSet::new();
    let mut variants = Vec::new();

    let candidates = [
        package_name.to_string(),
        package_name.replace('-', "_"),
        package_name.replace('-', "/"),
    ];

    for candidate in candidates {
        let parts: Vec<String> = candidate
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
        if parts.is_empty() {
            continue;
        }

        let key = parts.join("\0");
        if seen.insert(key) {
            variants.push(parts);
        }
    }

    variants
}

fn ruby_package_require_prefixes(package_name: &str) -> Vec<String> {
    ruby_package_path_variants(package_name)
        .into_iter()
        .map(|parts| parts.join("/"))
        .collect()
}

fn relative_ruby_path(root_dir: &Path, file_path: &Path) -> PathBuf {
    file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf()
}

fn sort_ruby_apis(apis: &mut [ApiEntry], selection: &RubyFileSelection) {
    apis.sort_by(|left, right| {
        ruby_api_rank(left, selection)
            .cmp(&ruby_api_rank(right, selection))
            .then_with(|| ruby_api_path(left).cmp(&ruby_api_path(right)))
            .then_with(|| ruby_api_line(left).cmp(&ruby_api_line(right)))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            .then_with(|| left.kind.to_string().cmp(&right.kind.to_string()))
    });
}

fn ruby_api_rank(api: &ApiEntry, selection: &RubyFileSelection) -> (usize, usize) {
    let Some(location) = api.location.as_ref() else {
        return (usize::MAX, usize::MAX);
    };

    let entrypoint_rank = selection
        .entrypoint
        .as_ref()
        .map(|entrypoint| usize::from(&location.file != entrypoint))
        .unwrap_or(1);
    let depth_rank = selection
        .depth_by_relative_path
        .get(&location.file)
        .copied()
        .unwrap_or(usize::MAX);

    (entrypoint_rank, depth_rank)
}

fn ruby_api_path(api: &ApiEntry) -> String {
    api.location
        .as_ref()
        .map(|location| location.file.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn ruby_api_line(api: &ApiEntry) -> usize {
    api.location
        .as_ref()
        .map(|location| location.line)
        .unwrap_or_default()
}

fn extract_from_ruby_file(
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

    let tree = parse(&source, Language::Ruby)?;
    let module_info = extract_from_tree(&tree, &source, Language::Ruby, file_path, Some(root_dir))?;
    let module_path = compute_ruby_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for func in &module_info.functions {
        let qualified_name = format!("{}.{}", module_path, func.name);
        let params = convert_ruby_params(&func.params);
        let return_type = func.return_type.clone();
        let kind = if func.decorators.iter().any(|decorator| decorator == "self") {
            ApiKind::ClassMethod
        } else {
            ApiKind::Function
        };

        apis.push(ApiEntry {
            qualified_name,
            kind,
            module: module_path.clone(),
            signature: Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: false,
                is_generator: false,
            }),
            docstring: func.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(generate_ruby_call_example(
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
        let qualified_name = format!("{}.{}", module_path, class.name);
        let is_module = class
            .decorators
            .iter()
            .any(|decorator| decorator == "module");

        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind: ApiKind::Class,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(if is_module {
                format!("{}::{}", module_path, class.name)
            } else {
                format!("{} = {}.new", class.name.to_lowercase(), class.name)
            }),
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
            if !include_private
                && ruby_visibility_for_range(
                    &source,
                    class.line_number as usize,
                    method.line_number as usize,
                ) != RubyVisibility::Public
            {
                continue;
            }

            let params = convert_ruby_params(&method.params);
            let return_type = method.return_type.clone();
            let kind = if method
                .decorators
                .iter()
                .any(|decorator| decorator == "self")
            {
                ApiKind::ClassMethod
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
                example: Some(generate_ruby_method_example(
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
    }

    for constant in &module_info.constants {
        if !include_private && !is_public_ruby_constant(constant) {
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

fn compute_ruby_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let stem = relative.with_extension("");
    let mut parts: Vec<String> = stem
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .filter(|part| part != "lib")
        .collect();

    if let Some(prefix_len) = ruby_package_path_variants(package_name)
        .into_iter()
        .find_map(|variant| starts_with_segments(&parts, &variant).then_some(variant.len()))
    {
        parts.drain(..prefix_len);
    }

    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RubyVisibility {
    Public,
    Private,
    Protected,
}

fn ruby_visibility_for_range(
    source: &str,
    start_line: usize,
    target_line: usize,
) -> RubyVisibility {
    let lines: Vec<&str> = source.lines().collect();
    let start = start_line.saturating_sub(1).min(lines.len());
    let end = target_line.saturating_sub(1).min(lines.len());
    let mut visibility = RubyVisibility::Public;

    for line in &lines[start..end] {
        match line.trim() {
            "public" => visibility = RubyVisibility::Public,
            "private" => visibility = RubyVisibility::Private,
            "protected" => visibility = RubyVisibility::Protected,
            _ => {}
        }
    }

    visibility
}

fn is_public_ruby_constant(field: &FieldInfo) -> bool {
    field
        .name
        .chars()
        .next()
        .map(|ch| ch.is_uppercase())
        .unwrap_or(false)
}

fn convert_ruby_params(raw_params: &[String]) -> Vec<Param> {
    raw_params
        .iter()
        .filter(|param| !param.is_empty())
        .map(|param| Param {
            name: param
                .trim_start_matches('*')
                .trim_start_matches('*')
                .trim_start_matches('&')
                .to_string(),
            type_annotation: None,
            default: None,
            is_variadic: param.starts_with('*') && !param.starts_with("**"),
            is_keyword: param.starts_with("**") || param.ends_with(':'),
        })
        .collect()
}

fn truncate_docstring(doc: &str) -> String {
    let first_para = doc.split("\n\n").next().unwrap_or(doc);
    let cleaned = first_para
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.len() <= 200 {
        cleaned
    } else {
        format!("{}...", &cleaned[..197])
    }
}

fn generate_ruby_call_example(module_path: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", module_path, func_name, args)
}

fn generate_ruby_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.new.{}({})", class_name, method_name, args)
}

fn starts_with_segments(parts: &[String], prefix: &[String]) -> bool {
    parts.len() >= prefix.len()
        && parts
            .iter()
            .zip(prefix.iter())
            .all(|(left, right)| left == right)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }

    normalized
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
    fn test_find_ruby_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "lib/root.rb", "module Root; end");
        write_file(&dir, "lib/sub/nested.rb", "module Nested; end");

        let files = find_ruby_files(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_find_ruby_files_skips_examples_directory() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "lib/root.rb", "module Root; end");
        write_file(&dir, "examples/demo.rb", "module Demo; end");

        let files = find_ruby_files(dir.path());
        let relative_files: Vec<PathBuf> = files
            .iter()
            .map(|file| file.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect();

        assert_eq!(relative_files, vec![PathBuf::from("lib/root.rb")]);
    }

    #[test]
    fn test_extract_ruby_surface_filters_private_methods() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/example.rb",
            r#"
class Greeter
  def hello(name)
  end

  private

  def secret(token)
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ruby_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
    }

    #[test]
    fn test_extract_ruby_surface_includes_private_methods_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/example.rb",
            r#"
class Vault
  private

  def secret(key)
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ruby_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with("Vault.secret")));
    }

    #[test]
    fn test_extract_ruby_surface_prefers_entrypoint_load_tree_over_actions_helpers() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/example.rb",
            r#"
require_relative "example/base"

module Example
  class CLI
    def run
    end
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/example/base.rb",
            r#"
class Base
  def call
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/example/actions.rb",
            r#"
class ActionsHelper
  def create_file
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ruby_api_surface(&resolved, false, None).unwrap();
        let files: Vec<PathBuf> = surface
            .apis
            .iter()
            .filter_map(|api| api.location.as_ref().map(|location| location.file.clone()))
            .collect();

        assert!(
            files.iter().any(|file| file == Path::new("lib/example.rb")),
            "expected package entrypoint APIs in surface, got files: {:?}",
            files
        );
        assert!(
            files
                .iter()
                .any(|file| file == Path::new("lib/example/base.rb")),
            "expected entrypoint load-tree APIs in surface, got files: {:?}",
            files
        );
        assert!(
            files.iter().all(|file| file != Path::new("lib/example/actions.rb")),
            "internal actions helper should be excluded from package-facing surface, got files: {:?}",
            files
        );

        let limited = extract_ruby_api_surface(&resolved, false, Some(1)).unwrap();
        assert_eq!(limited.apis.len(), 1);
        assert_eq!(
            limited.apis[0]
                .location
                .as_ref()
                .map(|location| location.file.as_path()),
            Some(Path::new("lib/example.rb")),
            "limited surface should prefer package entrypoint API, got: {:?}",
            limited.apis[0].location
        );
    }

    #[test]
    fn test_compute_ruby_module_path_dedupes_package_entrypoint_segment() {
        let root = Path::new("/tmp/example");

        assert_eq!(
            compute_ruby_module_path(&root.join("lib/thor.rb"), root, "thor"),
            "thor"
        );
        assert_eq!(
            compute_ruby_module_path(&root.join("lib/thor/actions.rb"), root, "thor"),
            "thor.actions"
        );
    }
}
