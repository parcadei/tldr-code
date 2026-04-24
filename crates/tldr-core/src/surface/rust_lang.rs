//! Rust-specific API surface extraction.
//!
//! Extracts the complete public API surface from a Rust crate by:
//! 1. Reading `Cargo.toml` to find the crate root (`src/lib.rs`)
//! 2. Walking all `.rs` files in the source tree
//! 3. Using tree-sitter to parse each file and extract pub functions, structs,
//!    traits, enums, constants, and impl blocks
//! 4. Filtering to `pub` items only (distinguishing `pub(crate)` from `pub`)
//! 5. Extracting derive macros to generate synthetic API entries
//! 6. Generating example usage strings from type signatures

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::language_profile::is_noise_dir;
use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the complete API surface from a Rust crate.
///
/// # Arguments
/// * `resolved` - The resolved package with root directory
/// * `include_private` - Whether to include non-pub items
/// * `limit` - Optional maximum number of APIs
///
/// # Returns
/// * `ApiSurface` with all extracted API entries
pub fn extract_rust_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    // Find all Rust source files
    let rs_files = find_rust_files(&resolved.root_dir);

    // Extract from each file
    for file_path in &rs_files {
        let file_apis = extract_from_rust_file(
            file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?;
        apis.extend(file_apis);
    }

    add_crate_root_reexports(&mut apis, &resolved.root_dir, &resolved.package_name);
    sort_apis_by_static_preference(&mut apis, "rust");

    // Apply limit if specified
    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "rust".to_string(),
        total,
        apis,
    })
}

/// Extract API entries from a single Rust file.
fn extract_from_rust_file(
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

    let tree = parse(&source, Language::Rust)?;

    // Use extract_from_tree to get module info
    let module_info = extract_from_tree(&tree, &source, Language::Rust, file_path, Some(root_dir))?;

    // Compute module path from file path
    let module_path = compute_rust_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    // Extract top-level functions
    for func in &module_info.functions {
        if !include_private && !is_rust_item_public(&source, func.line_number as usize) {
            continue;
        }

        let qualified_name = format!("{}::{}", module_path, func.name);
        let params = convert_rust_params(&func.params);
        let return_type = func.return_type.clone();
        let signature = Some(Signature {
            params: params.clone(),
            return_type: return_type.clone(),
            is_async: func.is_async,
            is_generator: false,
        });

        let example = generate_rust_function_example(
            &module_path,
            &func.name,
            &params,
            return_type.as_deref(),
        );
        let triggers = extract_triggers(&func.name, func.docstring.as_deref());

        apis.push(ApiEntry {
            qualified_name,
            kind: ApiKind::Function,
            module: module_path.clone(),
            signature,
            docstring: func.docstring.clone().map(|d| truncate_docstring(&d)),
            example,
            triggers,
            is_property: false,
            return_type,
            location: Some(Location {
                file: relative_path.clone(),
                line: func.line_number as usize,
                column: None,
            }),
        });
    }

    // Extract structs, traits, enums with their methods
    for class in &module_info.classes {
        let kind = determine_rust_class_kind(class, &source);

        if !include_private && !is_rust_item_public(&source, class.line_number as usize) {
            continue;
        }

        let qualified_name = format!("{}::{}", module_path, class.name);
        let triggers = extract_triggers(&class.name, class.docstring.as_deref());

        // Add the type itself
        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|d| truncate_docstring(&d)),
            example: generate_rust_type_example(&module_path, &class.name, kind),
            triggers,
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        // Add methods
        // For traits, all declared methods are implicitly public (no `pub` keyword).
        // Only structs/enums need per-method visibility checks.
        let is_trait = kind == ApiKind::Trait;
        for method in &class.methods {
            if !include_private
                && !is_trait
                && !is_rust_item_public(&source, method.line_number as usize)
            {
                continue;
            }

            let method_qualified = format!("{}::{}", qualified_name, method.name);
            let params = convert_rust_params(&method.params);
            let return_type = method.return_type.clone();
            let is_static = !method
                .params
                .iter()
                .any(|p| p == "self" || p.contains("self"));

            let method_kind = if is_static {
                ApiKind::StaticMethod
            } else {
                ApiKind::Method
            };

            let signature = Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: method.is_async,
                is_generator: false,
            });

            let example = generate_rust_method_example(
                &class.name,
                &method.name,
                is_static,
                &params,
                return_type.as_deref(),
            );
            let triggers = extract_triggers(&method.name, method.docstring.as_deref());

            apis.push(ApiEntry {
                qualified_name: method_qualified,
                kind: method_kind,
                module: module_path.clone(),
                signature,
                docstring: method.docstring.clone().map(|d| truncate_docstring(&d)),
                example,
                triggers,
                is_property: false,
                return_type,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: method.line_number as usize,
                    column: None,
                }),
            });
        }

        // Extract derive macros and add synthetic entries
        let derives = extract_derives(&source, class.line_number as usize);
        for derive in &derives {
            if let Some(synthetic) =
                synthetic_from_derive(derive, &qualified_name, &module_path, &relative_path)
            {
                apis.push(synthetic);
            }
        }
    }

    // Extract module-level constants
    for field in &module_info.constants {
        if !include_private {
            if let Some(ref vis) = field.visibility {
                if !vis.starts_with("pub") {
                    continue;
                }
            } else {
                continue;
            }
        }

        let qualified_name = format!("{}::{}", module_path, field.name);
        let triggers = extract_triggers(&field.name, None);

        apis.push(ApiEntry {
            qualified_name,
            kind: ApiKind::Constant,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}::{}", module_path, field.name)),
            triggers,
            is_property: false,
            return_type: field.field_type.clone(),
            location: Some(Location {
                file: relative_path.clone(),
                line: field.line_number as usize,
                column: None,
            }),
        });
    }

    Ok(apis)
}

// ============================================================================
// Helpers
// ============================================================================

/// Compute the Rust module path from a file path.
///
/// Examples:
/// - `src/lib.rs` -> `<crate>`
/// - `src/surface/mod.rs` -> `<crate>::surface`
/// - `src/fix/rust_lang.rs` -> `<crate>::fix::rust_lang`
fn compute_rust_module_path(file_path: &Path, root_dir: &Path, crate_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let relative_str = relative.to_string_lossy();

    // Strip "src/" prefix if present
    let module_part = relative_str.strip_prefix("src/").unwrap_or(&relative_str);

    // Strip .rs extension
    let module_part = module_part.strip_suffix(".rs").unwrap_or(module_part);

    // Handle special cases
    if module_part == "lib" || module_part == "main" {
        return crate_name.to_string();
    }

    // Handle mod.rs -> parent directory name
    let module_part = module_part.strip_suffix("/mod").unwrap_or(module_part);

    // Convert path separators to ::
    let module_path = module_part.replace('/', "::");

    format!("{}::{}", crate_name, module_path)
}

/// Convert raw Rust parameter strings to structured Params.
///
/// Raw params look like: `["self", "name: &str", "count: usize"]`
fn convert_rust_params(raw_params: &[String]) -> Vec<Param> {
    raw_params
        .iter()
        .map(|p| {
            let p = p.trim();
            if p == "self" || p == "&self" || p == "&mut self" || p == "mut self" {
                Param {
                    name: "self".to_string(),
                    type_annotation: Some(p.to_string()),
                    default: None,
                    is_variadic: false,
                    is_keyword: false,
                }
            } else if let Some((name, type_ann)) = p.split_once(':') {
                Param {
                    name: name.trim().to_string(),
                    type_annotation: Some(type_ann.trim().to_string()),
                    default: None,
                    is_variadic: false,
                    is_keyword: false,
                }
            } else {
                Param {
                    name: p.to_string(),
                    type_annotation: None,
                    default: None,
                    is_variadic: false,
                    is_keyword: false,
                }
            }
        })
        .collect()
}

/// Determine the kind of a Rust "class" (struct, trait, or enum).
fn determine_rust_class_kind(class: &ClassInfo, source: &str) -> ApiKind {
    // Check the source line at the class definition
    let lines: Vec<&str> = source.lines().collect();
    if class.line_number > 0 && (class.line_number as usize) <= lines.len() {
        let line = lines[class.line_number as usize - 1].trim();
        if line.contains("trait ") {
            return ApiKind::Trait;
        }
        if line.contains("enum ") {
            return ApiKind::Enum;
        }
    }
    ApiKind::Struct
}

/// Check if a Rust item at the given line is public.
fn is_rust_item_public(source: &str, line_number: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }
    let line = lines[line_number - 1].trim();
    line.starts_with("pub ") || line.starts_with("pub(")
}

/// Extract `#[derive(...)]` attributes from the lines before a struct/enum definition.
fn extract_derives(source: &str, struct_line: usize) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut derives = Vec::new();

    // Look at lines before the struct definition for #[derive(...)]
    for i in (0..struct_line.saturating_sub(1)).rev() {
        let line = lines[i].trim();
        if line.starts_with("#[derive(") || line.starts_with("#[derive (") {
            // Extract the derive list
            if let Some(start) = line.find('(') {
                if let Some(end) = line.rfind(')') {
                    let inner = &line[start + 1..end];
                    for item in inner.split(',') {
                        let item = item.trim();
                        if !item.is_empty() {
                            derives.push(item.to_string());
                        }
                    }
                }
            }
        } else if !line.starts_with("#[") && !line.starts_with("///") && !line.is_empty() {
            // Stop when we hit non-attribute/non-doc lines
            break;
        }
    }

    derives
}

/// Create synthetic API entries for derive macros.
///
/// For example, `#[derive(Clone)]` implies `MyStruct::clone()` exists.
fn synthetic_from_derive(
    derive: &str,
    parent_name: &str,
    module: &str,
    file: &Path,
) -> Option<ApiEntry> {
    let (method_name, return_desc) = match derive {
        "Clone" => ("clone", "Self"),
        "Debug" => return None, // Debug is for formatting, not a callable API
        "Default" => ("default", "Self"),
        "Hash" => return None, // Hash::hash() is rarely called directly
        "PartialEq" | "Eq" => return None, // Operators, not methods
        "PartialOrd" | "Ord" => return None,
        "Serialize" => return None, // serde::Serialize is generic, not a direct method
        "Deserialize" => return None,
        _ => return None,
    };

    Some(ApiEntry {
        qualified_name: format!("{}::{}", parent_name, method_name),
        kind: ApiKind::Method,
        module: module.to_string(),
        signature: Some(Signature {
            params: vec![Param {
                name: "self".to_string(),
                type_annotation: Some("&self".to_string()),
                default: None,
                is_variadic: false,
                is_keyword: false,
            }],
            return_type: Some(return_desc.to_string()),
            is_async: false,
            is_generator: false,
        }),
        docstring: Some(format!("Derived from `#[derive({})]`", derive)),
        example: None,
        triggers: vec![method_name.to_string(), "derive".to_string()],
        is_property: false,
        return_type: Some(return_desc.to_string()),
        location: Some(Location {
            file: file.to_path_buf(),
            line: 0,
            column: None,
        }),
    })
}

/// Truncate a docstring to approximately 200 characters, preserving the first paragraph.
fn truncate_docstring(doc: &str) -> String {
    let first_para = doc.split("\n\n").next().unwrap_or(doc);
    let cleaned: String = first_para
        .lines()
        .map(|l| l.trim().trim_start_matches("///").trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    if cleaned.len() > 200 {
        format!("{}...", &cleaned[..197])
    } else {
        cleaned
    }
}

#[derive(Debug)]
struct RustReexport {
    target_path: String,
    public_name: String,
}

fn add_crate_root_reexports(apis: &mut Vec<ApiEntry>, root_dir: &Path, crate_name: &str) {
    let root_file = ["src/lib.rs", "src/main.rs"]
        .into_iter()
        .map(|path| root_dir.join(path))
        .find(|path| path.is_file());
    let Some(root_file) = root_file else {
        return;
    };

    let Ok(source) = std::fs::read_to_string(root_file) else {
        return;
    };

    let reexports = parse_crate_root_reexports(&source);
    if reexports.is_empty() {
        return;
    }

    let existing = apis.clone();
    let mut added_names = std::collections::HashSet::new();
    for api in &existing {
        added_names.insert(api.qualified_name.clone());
    }

    for reexport in reexports {
        let target_prefix = qualify_reexport_target(crate_name, &reexport.target_path);
        let alias_prefix = format!("{crate_name}::{}", reexport.public_name);

        for api in &existing {
            let Some(aliased_name) = rewrite_reexported_qualified_name(
                &api.qualified_name,
                &target_prefix,
                &alias_prefix,
            ) else {
                continue;
            };

            if !added_names.insert(aliased_name.clone()) {
                continue;
            }

            let mut aliased_api = api.clone();
            aliased_api.qualified_name = aliased_name;
            aliased_api.module = crate_name.to_string();
            apis.push(aliased_api);
        }
    }
}

fn parse_crate_root_reexports(source: &str) -> Vec<RustReexport> {
    source
        .lines()
        .filter_map(parse_simple_rust_reexport)
        .collect()
}

fn parse_simple_rust_reexport(line: &str) -> Option<RustReexport> {
    let trimmed = line.trim();
    if !trimmed.starts_with("pub use ") || !trimmed.ends_with(';') {
        return None;
    }

    let body = trimmed
        .strip_prefix("pub use ")?
        .trim_end_matches(';')
        .trim();

    if body.contains('{') || body.contains('}') || body.contains('*') || body.contains(',') {
        return None;
    }

    let (target_path, public_name) = if let Some((target, alias)) = body.rsplit_once(" as ") {
        (target.trim(), alias.trim())
    } else {
        let public_name = body.rsplit("::").next()?.trim();
        (body, public_name)
    };

    let target_path = target_path
        .strip_prefix("crate::")
        .or_else(|| target_path.strip_prefix("self::"))
        .unwrap_or(target_path)
        .trim();

    if target_path.is_empty() || public_name.is_empty() {
        return None;
    }

    Some(RustReexport {
        target_path: target_path.to_string(),
        public_name: public_name.to_string(),
    })
}

fn qualify_reexport_target(crate_name: &str, target_path: &str) -> String {
    if target_path.starts_with(crate_name) {
        target_path.to_string()
    } else {
        format!("{crate_name}::{target_path}")
    }
}

fn rewrite_reexported_qualified_name(
    original_name: &str,
    target_prefix: &str,
    alias_prefix: &str,
) -> Option<String> {
    if original_name == target_prefix {
        return Some(alias_prefix.to_string());
    }

    original_name
        .strip_prefix(target_prefix)
        .filter(|suffix| suffix.starts_with("::"))
        .map(|suffix| format!("{alias_prefix}{suffix}"))
}

/// Walk a directory recursively to find all Rust source files.
pub fn find_rust_files(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return root
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| *ext == "rs")
            .map(|_| vec![root.to_path_buf()])
            .unwrap_or_default();
    }
    let mut files = Vec::new();
    find_rust_files_recursive(root, &mut files);
    files.sort();
    files
}

/// Recursive helper for finding Rust files.
fn find_rust_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !dir_name.starts_with('.') && !is_noise_dir(Language::Rust, dir_name) {
                find_rust_files_recursive(&path, files);
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

/// Generate an example usage string for a Rust function.
fn generate_rust_function_example(
    module: &str,
    name: &str,
    params: &[Param],
    return_type: Option<&str>,
) -> Option<String> {
    let args = rust_example_args(params, false);
    let ret_prefix = if return_type.is_some() {
        "let result = "
    } else {
        ""
    };
    Some(format!("{}{}::{}({})", ret_prefix, module, name, args))
}

/// Generate an example usage string for a Rust method.
fn generate_rust_method_example(
    type_name: &str,
    method_name: &str,
    is_static: bool,
    params: &[Param],
    return_type: Option<&str>,
) -> Option<String> {
    let args = rust_example_args(params, !is_static);
    let ret_prefix = if return_type.is_some() {
        "let result = "
    } else {
        ""
    };

    if is_static {
        Some(format!(
            "{}{}::{}({})",
            ret_prefix, type_name, method_name, args
        ))
    } else {
        let var = type_name.to_lowercase();
        Some(format!("{}{}.{}({})", ret_prefix, var, method_name, args))
    }
}

/// Generate an example for a Rust type (struct/enum/trait).
fn generate_rust_type_example(module: &str, name: &str, kind: ApiKind) -> Option<String> {
    match kind {
        ApiKind::Struct => Some(format!(
            "let {} = {}::{}::new(/* ... */);",
            name.to_lowercase(),
            module,
            name
        )),
        ApiKind::Enum => Some(format!("let val = {}::{}::default();", module, name)),
        ApiKind::Trait => None,
        _ => None,
    }
}

/// Format example arguments for Rust code.
fn rust_example_args(params: &[Param], skip_self: bool) -> String {
    params
        .iter()
        .filter(|p| if skip_self { p.name != "self" } else { true })
        .filter(|p| p.name != "self")
        .map(|p| rust_example_for_type(p.type_annotation.as_deref()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Generate an example value for a Rust type.
fn rust_example_for_type(type_ann: Option<&str>) -> String {
    match type_ann {
        Some("&str") | Some("&'_ str") | Some("&'static str") => "\"example\"".to_string(),
        Some("String") => "\"example\".to_string()".to_string(),
        Some("usize") | Some("u32") | Some("u64") | Some("i32") | Some("i64") => "42".to_string(),
        Some("u8") | Some("i8") => "0".to_string(),
        Some("u16") | Some("i16") => "0".to_string(),
        Some("f32") | Some("f64") => "1.0".to_string(),
        Some("bool") => "true".to_string(),
        Some("char") => "'a'".to_string(),
        Some(t) if t.starts_with("&[") => "&[]".to_string(),
        Some(t) if t.starts_with("Vec<") => "vec![]".to_string(),
        Some(t) if t.starts_with("Option<") => "None".to_string(),
        Some(t) if t.starts_with("&") => "&Default::default()".to_string(),
        Some("Self") => "Self::default()".to_string(),
        _ => "/* ... */".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_rust_surface_dir(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tldr-rust-surface-{test_name}-{unique}"));
        fs::create_dir_all(&dir).expect("create temp rust surface dir");
        dir
    }

    #[test]
    fn test_compute_rust_module_path_lib() {
        assert_eq!(
            compute_rust_module_path(Path::new("src/lib.rs"), Path::new(""), "mycrate"),
            "mycrate"
        );
    }

    #[test]
    fn test_compute_rust_module_path_submodule() {
        assert_eq!(
            compute_rust_module_path(Path::new("src/fix/rust_lang.rs"), Path::new(""), "mycrate"),
            "mycrate::fix::rust_lang"
        );
    }

    #[test]
    fn test_compute_rust_module_path_mod_rs() {
        assert_eq!(
            compute_rust_module_path(Path::new("src/surface/mod.rs"), Path::new(""), "mycrate"),
            "mycrate::surface"
        );
    }

    #[test]
    fn test_convert_rust_params() {
        let raw = vec![
            "&self".to_string(),
            "name: &str".to_string(),
            "count: usize".to_string(),
        ];
        let params = convert_rust_params(&raw);
        assert_eq!(params.len(), 3);
        assert_eq!(params[0].name, "self");
        assert_eq!(params[0].type_annotation, Some("&self".to_string()));
        assert_eq!(params[1].name, "name");
        assert_eq!(params[1].type_annotation, Some("&str".to_string()));
        assert_eq!(params[2].name, "count");
        assert_eq!(params[2].type_annotation, Some("usize".to_string()));
    }

    #[test]
    fn test_extract_derives() {
        let source = "/// A config struct.\n#[derive(Debug, Clone, Default)]\npub struct Config {\n    pub name: String,\n}\n";
        let derives = extract_derives(source, 3);
        assert!(derives.contains(&"Debug".to_string()));
        assert!(derives.contains(&"Clone".to_string()));
        assert!(derives.contains(&"Default".to_string()));
    }

    #[test]
    fn test_extract_derives_no_derive() {
        let source = "pub struct Simple {\n    pub x: i32,\n}\n";
        let derives = extract_derives(source, 1);
        assert!(derives.is_empty());
    }

    #[test]
    fn test_truncate_docstring_short() {
        assert_eq!(truncate_docstring("A short doc."), "A short doc.");
    }

    #[test]
    fn test_truncate_docstring_long() {
        let long_doc = "x".repeat(300);
        let result = truncate_docstring(&long_doc);
        assert!(result.len() <= 203);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_rust_example_for_type() {
        assert_eq!(rust_example_for_type(Some("&str")), "\"example\"");
        assert_eq!(rust_example_for_type(Some("usize")), "42");
        assert_eq!(rust_example_for_type(Some("bool")), "true");
        assert_eq!(rust_example_for_type(Some("Vec<i32>")), "vec![]");
        assert_eq!(rust_example_for_type(Some("Option<String>")), "None");
        assert_eq!(rust_example_for_type(None), "/* ... */");
    }

    #[test]
    fn test_is_rust_item_public() {
        let source = "pub struct Foo {}\nstruct Bar {}\npub fn baz() {}\nfn qux() {}\n";
        assert!(is_rust_item_public(source, 1));
        assert!(!is_rust_item_public(source, 2));
        assert!(is_rust_item_public(source, 3));
        assert!(!is_rust_item_public(source, 4));
    }

    #[test]
    fn test_determine_rust_class_kind() {
        let struct_source = "pub struct Config {}\n";
        let trait_source = "pub trait Greeter {}\n";
        let enum_source = "pub enum Status {}\n";

        let class = ClassInfo {
            name: "Config".to_string(),
            bases: vec![],
            docstring: None,
            methods: vec![],
            fields: vec![],
            decorators: vec![],
            line_number: 1,
        };

        assert_eq!(
            determine_rust_class_kind(&class, struct_source),
            ApiKind::Struct
        );
        assert_eq!(
            determine_rust_class_kind(&class, trait_source),
            ApiKind::Trait
        );
        assert_eq!(
            determine_rust_class_kind(&class, enum_source),
            ApiKind::Enum
        );
    }

    #[test]
    fn test_find_rust_files() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let files = find_rust_files(&fixture_dir);
        assert!(
            !files.is_empty(),
            "Should find .rs files in fixture directory"
        );
        assert!(
            files.iter().any(|f| f.to_string_lossy().contains("lib.rs")),
            "Should find lib.rs"
        );
    }

    #[test]
    fn test_find_rust_files_skips_repo_noise_directories() {
        let root = create_temp_rust_surface_dir("noise-dirs");
        fs::create_dir_all(root.join("src")).expect("create src dir");
        fs::create_dir_all(root.join("examples")).expect("create examples dir");
        fs::create_dir_all(root.join("benches")).expect("create benches dir");
        fs::create_dir_all(root.join("tests")).expect("create tests dir");

        fs::write(root.join("src/lib.rs"), "pub fn public_api() {}\n").expect("write lib.rs");
        fs::write(root.join("src/internal.rs"), "pub fn internal_api() {}\n")
            .expect("write internal.rs");
        fs::write(root.join("examples/demo.rs"), "pub fn example_api() {}\n")
            .expect("write examples/demo.rs");
        fs::write(root.join("benches/bench_api.rs"), "pub fn bench_api() {}\n")
            .expect("write benches/bench_api.rs");
        fs::write(
            root.join("tests/integration.rs"),
            "pub fn integration_api() {}\n",
        )
        .expect("write tests/integration.rs");

        let files = find_rust_files(&root);
        let relative: Vec<String> = files
            .iter()
            .map(|path| {
                path.strip_prefix(&root)
                    .expect("path under temp root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert!(relative.iter().any(|path| path == "src/lib.rs"));
        assert!(relative.iter().any(|path| path == "src/internal.rs"));
        assert!(
            !relative.iter().any(|path| path.starts_with("examples/")),
            "examples should be excluded, got {relative:?}"
        );
        assert!(
            !relative.contains(&"benches/bench_api.rs".to_string()),
            "benches should be excluded, got {relative:?}"
        );
        assert!(
            !relative.contains(&"tests/integration.rs".to_string()),
            "tests should be excluded, got {relative:?}"
        );
    }

    #[test]
    fn test_extract_rust_api_surface_minimal_crate() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let resolved = ResolvedPackage {
            root_dir: fixture_dir,
            package_name: "minimal_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_rust_api_surface(&resolved, false, None);
        assert!(
            surface.is_ok(),
            "Extraction should succeed: {:?}",
            surface.err()
        );
        let surface = surface.unwrap();

        assert_eq!(surface.language, "rust");
        assert_eq!(surface.package, "minimal_crate");

        // Should have at least: Config struct, Config::new, Config::address,
        // greet fn, MAX_RETRIES const, Greeter trait, Status enum
        assert!(
            surface.total >= 5,
            "Should extract at least 5 public APIs, got {}:\n{:?}",
            surface.total,
            surface
                .apis
                .iter()
                .map(|a| &a.qualified_name)
                .collect::<Vec<_>>()
        );

        // Verify specific entries
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Public function
        assert!(
            names.iter().any(|n| n.contains("greet")),
            "Should contain greet function. Got: {:?}",
            names
        );

        // Public constant
        assert!(
            names.iter().any(|n| n.contains("MAX_RETRIES")),
            "Should contain MAX_RETRIES constant. Got: {:?}",
            names
        );

        // Public struct (qualified as <crate>::Config)
        assert!(
            names
                .iter()
                .any(|n| n.ends_with("::Config") && !n.contains("::Config::")),
            "Should contain Config struct. Got: {:?}",
            names
        );

        // Should NOT contain private_function
        assert!(
            !names.iter().any(|n| n.contains("private_function")),
            "Should not contain private_function. Got: {:?}",
            names
        );

        // Should NOT contain internal_helper
        assert!(
            !names.iter().any(|n| n.contains("internal_helper")),
            "Should not contain internal_helper. Got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_rust_api_surface_include_private() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let resolved = ResolvedPackage {
            root_dir: fixture_dir,
            package_name: "minimal_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_rust_api_surface(&resolved, true, None);
        assert!(surface.is_ok());
        let surface = surface.unwrap();

        // With include_private, should have more entries
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("private_function")),
            "Should contain private_function when include_private=true. Got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_rust_api_surface_adds_crate_root_pub_use_entries() {
        let root = create_temp_rust_surface_dir("crate-root-pub-use");
        fs::create_dir_all(root.join("src")).expect("create src dir");
        fs::write(
            root.join("src/lib.rs"),
            "mod internal;\npub use internal::Greeter;\n",
        )
        .expect("write lib.rs");
        fs::write(
            root.join("src/internal.rs"),
            "pub struct Greeter;\n\nimpl Greeter {\n    pub fn new() -> Self {\n        Self\n    }\n}\n",
        )
        .expect("write internal.rs");

        let resolved = ResolvedPackage {
            root_dir: root,
            package_name: "sample_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface =
            extract_rust_api_surface(&resolved, false, None).expect("extract rust surface");
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"sample_crate::Greeter"),
            "crate-root re-exported type should be surfaced, got {names:?}"
        );
        assert!(
            names.contains(&"sample_crate::Greeter::new"),
            "crate-root re-exported methods should be surfaced, got {names:?}"
        );
    }

    #[test]
    fn test_extract_rust_api_surface_with_limit() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let resolved = ResolvedPackage {
            root_dir: fixture_dir,
            package_name: "minimal_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_rust_api_surface(&resolved, false, Some(3));
        assert!(surface.is_ok());
        let surface = surface.unwrap();
        assert!(
            surface.total <= 3,
            "Should respect limit, got {}",
            surface.total
        );
    }

    #[test]
    fn test_extract_rust_api_surface_ranks_crate_root_before_neutral_paths() {
        let root = create_temp_rust_surface_dir("ranking-before-docs");
        fs::create_dir_all(root.join("src")).expect("create src dir");
        fs::create_dir_all(root.join("examples")).expect("create examples dir");

        fs::write(root.join("src/lib.rs"), "pub fn public_api() {}\n").expect("write lib.rs");
        fs::write(root.join("aaa_guide.rs"), "pub fn guide_api() {}\n")
            .expect("write aaa_guide.rs");
        fs::write(root.join("examples/demo.rs"), "pub fn demo_api() {}\n")
            .expect("write examples/demo.rs");

        let resolved = ResolvedPackage {
            root_dir: root.clone(),
            package_name: "ranked_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface =
            extract_rust_api_surface(&resolved, false, Some(1)).expect("extract rust surface");

        assert_eq!(
            surface.apis.first().map(|api| api.qualified_name.as_str()),
            Some("ranked_crate::public_api"),
            "crate-root API should outrank neutral paths, got {:?}",
            surface
                .apis
                .iter()
                .map(|api| (
                    &api.qualified_name,
                    api.location.as_ref().map(|loc| &loc.file)
                ))
                .collect::<Vec<_>>()
        );

        let full_surface =
            extract_rust_api_surface(&resolved, false, None).expect("extract full rust surface");
        assert!(
            !full_surface
                .apis
                .iter()
                .any(|api| api.qualified_name.contains("demo_api")),
            "noise APIs from examples should be excluded, got {:?}",
            full_surface
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
    fn test_synthetic_from_derive_clone() {
        let entry = synthetic_from_derive(
            "Clone",
            "mycrate::Config",
            "mycrate",
            &PathBuf::from("src/lib.rs"),
        );
        assert!(entry.is_some());
        let e = entry.unwrap();
        assert_eq!(e.qualified_name, "mycrate::Config::clone");
        assert_eq!(e.kind, ApiKind::Method);
    }

    #[test]
    fn test_synthetic_from_derive_debug_returns_none() {
        let entry = synthetic_from_derive(
            "Debug",
            "mycrate::Config",
            "mycrate",
            &PathBuf::from("src/lib.rs"),
        );
        assert!(
            entry.is_none(),
            "Debug derive should not produce a synthetic entry"
        );
    }

    // ---- Trait extraction tests ----

    #[test]
    fn test_extract_rust_api_surface_pub_trait() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let resolved = ResolvedPackage {
            root_dir: fixture_dir,
            package_name: "minimal_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_rust_api_surface(&resolved, false, None);
        assert!(
            surface.is_ok(),
            "Extraction should succeed: {:?}",
            surface.err()
        );
        let surface = surface.unwrap();

        // Find the Greeter trait entry
        let trait_entry = surface.apis.iter().find(|a| {
            a.qualified_name.ends_with("::Greeter") && !a.qualified_name.contains("::Greeter::")
        });

        assert!(
            trait_entry.is_some(),
            "Should extract pub trait Greeter. Got: {:?}",
            surface
                .apis
                .iter()
                .map(|a| (&a.qualified_name, &a.kind))
                .collect::<Vec<_>>()
        );

        let t = trait_entry.unwrap();
        assert_eq!(
            t.kind,
            ApiKind::Trait,
            "Greeter should have kind Trait, got {:?}",
            t.kind
        );
    }

    #[test]
    fn test_extract_rust_api_surface_trait_methods() {
        let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/surface/rust/minimal_crate");
        let resolved = ResolvedPackage {
            root_dir: fixture_dir,
            package_name: "minimal_crate".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_rust_api_surface(&resolved, false, None);
        assert!(surface.is_ok());
        let surface = surface.unwrap();

        // The trait's greet method should be extracted
        let trait_method = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("Greeter::greet"));

        assert!(
            trait_method.is_some(),
            "Should extract Greeter::greet method. Got: {:?}",
            surface
                .apis
                .iter()
                .map(|a| &a.qualified_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_rust_api_surface_private_trait_excluded() {
        // Create a source with a private trait
        let source = "trait PrivateHelper {\n    fn help(&self);\n}\n\npub trait PublicApi {\n    fn serve(&self);\n}\n";
        let tree = crate::ast::parser::parse(source, Language::Rust).unwrap();
        let module_info = crate::ast::extract::extract_from_tree(
            &tree,
            source,
            Language::Rust,
            Path::new("test.rs"),
            None,
        )
        .unwrap();

        // Count public traits in classes list
        let public_traits: Vec<_> = module_info
            .classes
            .iter()
            .filter(|c| {
                let line = c.line_number as usize;
                is_rust_item_public(source, line)
            })
            .collect();

        // PrivateHelper should be filtered when include_private=false
        assert!(
            !public_traits.iter().any(|c| c.name == "PrivateHelper"),
            "Private trait should not appear in public-only extraction"
        );

        // PublicApi should be present
        assert!(
            public_traits.iter().any(|c| c.name == "PublicApi"),
            "Public trait should be extracted"
        );
    }
}
