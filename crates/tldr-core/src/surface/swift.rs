//! Swift-specific API surface extraction.
//!
//! Swift surfaces only `public` and `open` declarations by default.

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::{is_noise_dir, strip_layout_segments};
use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Swift API surface for a resolved package.
pub fn extract_swift_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_swift_files(&resolved.root_dir) {
        apis.extend(extract_from_swift_file(
            &file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?);
    }

    sort_apis_by_static_preference(&mut apis, "swift");

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "swift".to_string(),
        total,
        apis,
    })
}

fn find_swift_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "swift")
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let package_target_roots = swift_library_target_roots(dir);
    if !package_target_roots.is_empty() {
        let mut files = Vec::new();
        for target_root in package_target_roots {
            find_swift_files_recursive(&target_root, &mut files);
        }
        files.sort();
        return files;
    }

    let package_sources_dir = dir.join("Sources");
    if package_sources_dir.is_dir() {
        let mut files = Vec::new();
        find_swift_files_recursive(&package_sources_dir, &mut files);
        files.sort();
        return files;
    }

    let mut files = Vec::new();
    find_swift_files_recursive(dir, &mut files);
    files.sort();
    files
}

fn find_swift_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !is_noise_dir(Language::Swift, name) {
                        find_swift_files_recursive(&path, files);
                    }
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("swift") {
                files.push(path);
            }
        }
    }
}

fn swift_library_target_roots(root_dir: &Path) -> Vec<PathBuf> {
    let manifest_path = root_dir.join("Package.swift");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return Vec::new();
    };

    parse_swift_library_targets(&manifest)
        .into_iter()
        .map(|target_name| root_dir.join("Sources").join(target_name))
        .filter(|target_root| target_root.is_dir())
        .collect()
}

fn parse_swift_library_targets(manifest: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut search_offset = 0;

    while let Some(relative_start) = manifest[search_offset..].find(".library(") {
        let start = search_offset + relative_start;
        let block_start = start + ".library".len();
        let Some(block) = extract_swift_manifest_call(&manifest[block_start..]) else {
            break;
        };

        targets.extend(parse_targets_array(block));
        search_offset = block_start + block.len();
    }

    targets.sort();
    targets.dedup();
    targets
}

fn extract_swift_manifest_call(source: &str) -> Option<&str> {
    let mut chars = source.char_indices();
    let (_, first_char) = chars.next()?;
    if first_char != '(' {
        return None;
    }

    let mut depth = 0usize;
    for (index, ch) in source.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&source[..=index]);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_targets_array(block: &str) -> Vec<String> {
    let Some(targets_index) = block.find("targets:") else {
        return Vec::new();
    };
    let tail = &block[targets_index + "targets:".len()..];
    let Some(array_start) = tail.find('[') else {
        return Vec::new();
    };
    let tail = &tail[array_start + 1..];
    let Some(array_end) = tail.find(']') else {
        return Vec::new();
    };
    let array_contents = &tail[..array_end];

    array_contents
        .split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim().trim_matches('"');
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn extract_from_swift_file(
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

    let tree = parse(&source, Language::Swift)?;
    let module_info =
        extract_from_tree(&tree, &source, Language::Swift, file_path, Some(root_dir))?;
    let module_path = compute_swift_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for func in &module_info.functions {
        if !include_private && !is_swift_public_at_line(&source, func.line_number as usize) {
            continue;
        }

        let params = convert_swift_params(&func.params);
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
            example: Some(generate_swift_call_example(
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
        let is_extension = is_swift_extension_at_line(&source, class.line_number as usize);
        let class_name = effective_swift_class_name(class, &source);
        let qualified_name = format!("{}.{}", module_path, class_name);

        // For extensions: skip the type entry itself but always process members.
        // For regular types: check visibility on the type declaration.
        if !is_extension {
            if !include_private && !is_swift_public_at_line(&source, class.line_number as usize) {
                continue;
            }

            let kind = determine_swift_kind(class, &source);
            apis.push(ApiEntry {
                qualified_name: qualified_name.clone(),
                kind,
                module: module_path.clone(),
                signature: None,
                docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
                example: Some(generate_swift_type_example(&class_name, kind)),
                triggers: extract_triggers(&class_name, class.docstring.as_deref()),
                is_property: false,
                return_type: None,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: class.line_number as usize,
                    column: None,
                }),
            });
        }

        // Extension members inherit visibility from the extension block if it
        // has an explicit access modifier (e.g. `public extension Foo`), or
        // from each individual member declaration otherwise.
        let extension_is_public =
            is_extension && is_swift_public_at_line(&source, class.line_number as usize);

        for method in &class.methods {
            if !include_private
                && !extension_is_public
                && !is_swift_public_at_line(&source, method.line_number as usize)
            {
                continue;
            }

            let params = convert_swift_params(&method.params);
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
                example: Some(generate_swift_method_example(
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
        if !include_private && !is_swift_public_at_line(&source, constant.line_number as usize) {
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

fn compute_swift_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let raw_parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    if let Some((index, _)) = raw_parts
        .iter()
        .enumerate()
        .find(|(_, part)| part.eq_ignore_ascii_case("sources"))
    {
        if let Some(target_segment) = raw_parts.get(index + 1) {
            let target = normalize_swift_module_segment(target_segment);
            if !target.is_empty() {
                return format!("{}.{}", package_name, target);
            }
        }
        return package_name.to_string();
    }

    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let parts: Vec<String> = strip_layout_segments(Language::Swift, parent)
        .into_iter()
        .map(|part| normalize_swift_module_segment(&part))
        .filter(|part| !part.is_empty())
        .collect();

    parts
        .first()
        .map(|part| format!("{}.{}", package_name, part))
        .unwrap_or_else(|| package_name.to_string())
}

fn normalize_swift_module_segment(segment: &str) -> String {
    segment
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
        .filter(|part| !part.is_empty())
        .collect::<String>()
}

fn is_swift_public_at_line(source: &str, line_number: usize) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| line.contains("public ") || line.contains("open "))
        .unwrap_or(false)
}

fn is_swift_extension_at_line(source: &str, line_number: usize) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("extension ")
                || trimmed.starts_with("public extension ")
                || trimmed.starts_with("open extension ")
                || trimmed.starts_with("@") && trimmed.contains("extension ")
        })
        .unwrap_or(false)
}

fn effective_swift_class_name(class: &ClassInfo, source: &str) -> String {
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
            "class" | "struct" | "enum" | "protocol" | "extension" => {
                if let Some(name) = tokens.get(idx + 1) {
                    return (*name).to_string();
                }
            }
            _ => {}
        }
    }

    String::new()
}

fn determine_swift_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let line = source
        .lines()
        .nth(class.line_number.saturating_sub(1) as usize)
        .unwrap_or("")
        .trim_start();

    let tokens: Vec<&str> = line.split_whitespace().collect();
    for token in &tokens {
        match *token {
            "struct" => return ApiKind::Struct,
            "enum" => return ApiKind::Enum,
            "protocol" => return ApiKind::Interface,
            "class" => return ApiKind::Class,
            _ => {}
        }
    }
    ApiKind::Class
}

fn convert_swift_params(raw_params: &[String]) -> Vec<Param> {
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
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("///")
                .trim_start_matches('*')
                .trim()
        })
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.len() <= 200 {
        cleaned
    } else {
        format!("{}...", &cleaned[..197])
    }
}

fn generate_swift_call_example(module_path: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| format!("{}: {}", param.name, param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", module_path, func_name, args)
}

fn generate_swift_type_example(class_name: &str, kind: ApiKind) -> String {
    match kind {
        ApiKind::Struct => format!("let value = {}()", class_name),
        _ => format!("let value = {}()", class_name),
    }
}

fn generate_swift_method_example(class_name: &str, method_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| format!("{}: {}", param.name, param.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", class_name.lowercase_first(), method_name, args)
}

trait LowercaseFirst {
    fn lowercase_first(&self) -> String;
}

impl LowercaseFirst for str {
    fn lowercase_first(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_lowercase(), chars.as_str()),
            None => String::new(),
        }
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
    fn test_find_swift_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "Sources/App.swift", "public struct App {}");
        write_file(&dir, "Tests/AppTests.swift", "final class AppTests {}");

        let files = find_swift_files(dir.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], dir.path().join("Sources/App.swift"));
    }

    #[test]
    fn test_find_swift_files_prefers_sources_over_examples_and_tools() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "Sources/ArgumentParser/Command.swift",
            "public struct Command {}",
        );
        write_file(
            &dir,
            "Examples/repeat/Repeat.swift",
            "public struct RepeatExample {}",
        );
        write_file(
            &dir,
            "Tools/generate-manual/GenerateManual.swift",
            "public struct GenerateManual {}",
        );
        write_file(
            &dir,
            "Plugins/GenerateDoccReference/GenerateDoccReference.swift",
            "public struct GenerateDoccReference {}",
        );

        let files = find_swift_files(dir.path());
        assert_eq!(
            files,
            vec![dir.path().join("Sources/ArgumentParser/Command.swift")]
        );
    }

    #[test]
    fn test_find_swift_files_prefers_library_targets_from_package_manifest() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "Package.swift",
            r#"
import PackageDescription

let package = Package(
    name: "example",
    products: [
        .library(name: "ArgumentParser", targets: ["ArgumentParser"])
    ],
    targets: [
        .target(name: "ArgumentParser"),
        .target(name: "ArgumentParserToolInfo")
    ]
)
"#,
        );
        write_file(
            &dir,
            "Sources/ArgumentParser/Command.swift",
            "public struct Command {}",
        );
        write_file(
            &dir,
            "Sources/ArgumentParserToolInfo/ToolInfo.swift",
            "public struct ToolInfo {}",
        );

        let files = find_swift_files(dir.path());
        assert_eq!(
            files,
            vec![dir.path().join("Sources/ArgumentParser/Command.swift")]
        );
    }

    #[test]
    fn test_extract_swift_surface_filters_internal_members() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "Sources/App.swift",
            r#"
public func greet(name: String) -> String { name }
internal func debug(name: String) -> String { name }

public struct Greeter {
    public func hello(name: String) -> String { name }
    internal func secret(name: String) -> String { name }
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_swift_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with(".greet")));
        assert!(names.iter().any(|name| name.ends_with("Greeter.hello")));
        assert!(!names.iter().any(|name| name.ends_with(".debug")));
        assert!(!names.iter().any(|name| name.ends_with("Greeter.secret")));
    }

    #[test]
    fn test_extract_swift_surface_includes_internal_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "Sources/App.swift",
            r#"
internal func debug(name: String) -> String { name }
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_swift_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with(".debug")));
    }

    #[test]
    fn test_extract_swift_surface_with_limit_prefers_sources_api_over_examples() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "Examples/Color.swift",
            r#"
public struct ColorOptions {
    public init() {}
}
"#,
        );
        write_file(
            &dir,
            "Sources/ArgumentParser/Command.swift",
            r#"
public struct ParsableCommand {
    public init() {}
}
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "swift-argument-parser".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_swift_api_surface(&resolved, false, Some(1)).unwrap();
        assert_eq!(surface.apis.len(), 1);
        assert_eq!(
            surface.apis[0]
                .location
                .as_ref()
                .map(|location| location.file.as_path()),
            Some(Path::new("Sources/ArgumentParser/Command.swift"))
        );
        assert!(
            surface.apis[0].qualified_name.contains("ParsableCommand"),
            "expected Sources API to outrank examples, got {:?}",
            surface
                .apis
                .iter()
                .map(|api| (
                    &api.qualified_name,
                    api.location.as_ref().map(|loc| &loc.file)
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_compute_swift_module_path_normalizes_spaced_segments() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/Sources/ArgumentParser/Parsable Properties/Argument.swift");

        assert_eq!(
            compute_swift_module_path(file, root, "swift-argument-parser"),
            "swift-argument-parser.ArgumentParser"
        );
    }

    #[test]
    fn test_compute_swift_module_path_keeps_library_target_clean() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/Sources/ArgumentParser/Command.swift");

        assert_eq!(
            compute_swift_module_path(file, root, "swift-argument-parser"),
            "swift-argument-parser.ArgumentParser"
        );
    }

    #[test]
    fn test_compute_swift_module_path_strips_nested_sources_segment() {
        let root = Path::new("/repo");
        let file = Path::new("/repo/package/Sources/Networking/HTTP/Client.swift");

        assert_eq!(
            compute_swift_module_path(file, root, "example_pkg"),
            "example_pkg.Networking"
        );
    }
}
