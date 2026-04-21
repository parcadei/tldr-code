//! Elixir-specific API surface extraction.
//!
//! Elixir uses `def` for public functions and `defp` for private functions.
//! Modules are surfaced as class-like containers.

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::Language;
use crate::TldrResult;

use super::language_profile::{is_noise_dir, static_preference_score};
use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public Elixir API surface for a resolved package.
pub fn extract_elixir_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    for file_path in find_elixir_files(&resolved.root_dir) {
        apis.extend(extract_from_elixir_file(
            &file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?);
    }

    sort_apis_by_static_preference(&mut apis, "elixir");
    apis.sort_by_key(|api| Reverse(elixir_package_preference_score(api, &resolved.package_name)));

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "elixir".to_string(),
        total,
        apis,
    })
}

fn find_elixir_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "ex" || *ext == "exs")
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && !is_noise_dir(Language::Elixir, name) {
                        files.extend(find_elixir_files(&path));
                    }
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("ex" | "exs")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_elixir_file(
    file_path: &Path,
    root_dir: &Path,
    _package_name: &str,
    include_private: bool,
) -> TldrResult<Vec<ApiEntry>> {
    let source = std::fs::read_to_string(file_path).map_err(|e| {
        crate::error::TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            format!("Cannot read: {}", e),
        )
    })?;

    let tree = parse(&source, Language::Elixir)?;
    let module_info =
        extract_from_tree(&tree, &source, Language::Elixir, file_path, Some(root_dir))?;
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    for class in &module_info.classes {
        if !include_private && is_hidden_elixir_docstring(class.docstring.as_deref()) {
            continue;
        }

        let module_name = class.name.clone();
        apis.push(ApiEntry {
            qualified_name: module_name.clone(),
            kind: ApiKind::Class,
            module: module_name.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|doc| truncate_docstring(&doc)),
            example: Some(format!("{}/0", module_name)),
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
                && (is_defp_at_line(&source, method.line_number as usize)
                    || is_hidden_elixir_docstring(method.docstring.as_deref()))
            {
                continue;
            }

            let params = convert_elixir_params(&method.params);
            let return_type = method.return_type.clone();
            apis.push(ApiEntry {
                qualified_name: format!("{}.{}", module_name, method.name),
                kind: ApiKind::Function,
                module: module_name.clone(),
                signature: Some(Signature {
                    params: params.clone(),
                    return_type: return_type.clone(),
                    is_async: false,
                    is_generator: false,
                }),
                docstring: method.docstring.clone().map(|doc| truncate_docstring(&doc)),
                example: Some(generate_elixir_example(&module_name, &method.name, &params)),
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

    Ok(apis)
}

fn elixir_package_preference_score(api: &ApiEntry, package_name: &str) -> i32 {
    let mut score = api
        .location
        .as_ref()
        .map(|location| static_preference_score("elixir", &location.file))
        .unwrap_or_default();

    if api
        .location
        .as_ref()
        .is_some_and(|location| location.file == elixir_root_entry_file(package_name))
    {
        score += 1000;
    }

    if api
        .docstring
        .as_deref()
        .is_some_and(|doc| !is_hidden_elixir_docstring(Some(doc)))
    {
        score += 25;
    }

    if matches!(api.kind, ApiKind::Class) {
        score += 5;
    }

    score
}

fn elixir_root_entry_file(package_name: &str) -> PathBuf {
    PathBuf::from("lib").join(format!("{}.ex", package_name))
}

fn is_hidden_elixir_docstring(doc: Option<&str>) -> bool {
    doc.is_some_and(|doc| {
        let normalized = doc.trim().trim_matches('"').trim_matches('\'');
        normalized == "false"
    })
}

fn is_defp_at_line(source: &str, line_number: usize) -> bool {
    source
        .lines()
        .nth(line_number.saturating_sub(1))
        .map(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("defp ") || trimmed.starts_with("defp(")
        })
        .unwrap_or(false)
}

fn convert_elixir_params(raw_params: &[String]) -> Vec<Param> {
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
        .replace("@doc", "")
        .replace('"', "")
        .replace("~S", "")
        .trim()
        .to_string();

    if cleaned.len() <= 200 {
        cleaned
    } else {
        format!("{}...", &cleaned[..197])
    }
}

fn generate_elixir_example(module_name: &str, func_name: &str, params: &[Param]) -> String {
    let args = params
        .iter()
        .map(|param| param.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}.{}({})", module_name, func_name, args)
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
    fn test_find_elixir_files_recurses() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "lib/app.ex", "defmodule App do\nend\n");
        write_file(&dir, "test/app_test.exs", "defmodule AppTest do\nend\n");

        let files = find_elixir_files(dir.path());
        let relative_files: Vec<PathBuf> = files
            .iter()
            .map(|file| file.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect();

        assert_eq!(relative_files, vec![PathBuf::from("lib/app.ex")]);
    }

    #[test]
    fn test_find_elixir_files_skips_examples_and_installer_directories() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "lib/app.ex", "defmodule App do\nend\n");
        write_file(&dir, "examples/demo.ex", "defmodule Demo do\nend\n");
        write_file(
            &dir,
            "installer/mix/tasks/local.phx.ex",
            "defmodule Mix.Tasks.Local.Phx do\nend\n",
        );

        let files = find_elixir_files(dir.path());
        let relative_files: Vec<PathBuf> = files
            .iter()
            .map(|file| file.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect();

        assert_eq!(relative_files, vec![PathBuf::from("lib/app.ex")]);
    }

    #[test]
    fn test_extract_elixir_surface_filters_defp() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/sample.ex",
            r#"
defmodule Sample do
  @doc "Public function"
  def hello(name) do
    name
  end

  defp secret(token) do
    token
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "sample".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_elixir_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Sample.hello")));
        assert!(!names.iter().any(|name| name.ends_with("Sample.secret")));
    }

    #[test]
    fn test_extract_elixir_surface_includes_defp_when_requested() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/sample.ex",
            r#"
defmodule Sample do
  defp hidden() do
    :ok
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "sample".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_elixir_api_surface(&resolved, true, None).unwrap();
        assert!(surface
            .apis
            .iter()
            .any(|api| api.qualified_name.ends_with("Sample.hidden")));
    }

    #[test]
    fn test_extract_elixir_surface_filters_hidden_modules_and_functions() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/jason.ex",
            r#"
defmodule Jason do
  @moduledoc "Public entrypoint"

  @doc "Decode JSON"
  def decode(input) do
    input
  end

  @doc false
  def format_opts(opts) do
    opts
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/decoder.ex",
            r#"
defmodule Jason.Decoder do
  @moduledoc false

  def parse(input) do
    input
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/codegen.ex",
            r#"
defmodule Jason.Codegen do
  @moduledoc false

  def jump_table(ranges, default) do
    {ranges, default}
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "jason".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_elixir_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(names.contains(&"Jason"));
        assert!(names.contains(&"Jason.decode"));
        assert!(!names.iter().any(|name| name.starts_with("Jason.Decoder")));
        assert!(!names.iter().any(|name| name.starts_with("Jason.Codegen")));
        assert!(!names.contains(&"Jason.format_opts"));
    }

    #[test]
    fn test_extract_elixir_surface_prefers_package_entrypoint_under_limit() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "lib/jason.ex",
            r#"
defmodule Jason do
  @moduledoc "Public entrypoint"

  @doc "Decode JSON"
  def decode(input) do
    input
  end

  @doc "Encode JSON"
  def encode(input) do
    input
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/formatter.ex",
            r#"
defmodule Jason.Formatter do
  @moduledoc "Pretty-printing support"

  @doc "Pretty print encoded JSON"
  def pretty_print(input) do
    input
  end
end
"#,
        );
        write_file(
            &dir,
            "lib/decoder.ex",
            r#"
defmodule Jason.Decoder do
  @moduledoc false

  def parse(input) do
    input
  end
end
"#,
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "jason".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_elixir_api_surface(&resolved, false, Some(2)).unwrap();

        assert_eq!(
            surface.apis.first().map(|api| api.qualified_name.as_str()),
            Some("Jason"),
            "root package module should outrank secondary modules, got {:?}",
            surface
                .apis
                .iter()
                .map(|api| (&api.qualified_name, api.location.as_ref().map(|loc| &loc.file)))
                .collect::<Vec<_>>()
        );
        assert!(
            surface.apis.iter().all(|api| {
                api.location
                    .as_ref()
                    .is_some_and(|loc| loc.file == Path::new("lib/jason.ex"))
            }),
            "limited surface should be dominated by the package entrypoint, got {:?}",
            surface
                .apis
                .iter()
                .map(|api| (&api.qualified_name, api.location.as_ref().map(|loc| &loc.file)))
                .collect::<Vec<_>>()
        );
    }
}
