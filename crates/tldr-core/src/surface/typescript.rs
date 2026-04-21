//! TypeScript-specific API surface extraction.
//!
//! Extracts the complete public API surface from a TypeScript package by:
//! 1. Reading `node_modules/<pkg>/package.json` to find the `"types"` or `"typings"` field
//! 2. Parsing the `.d.ts` declaration file with tree-sitter-typescript
//! 3. Extracting every `export function`, `export interface`, `export class`,
//!    `export type`, and `export const` as a public API entry
//! 4. Extracting parameters and type annotations from `.d.ts` declarations
//! 5. Detecting `readonly` properties and getter syntax in interfaces
//!
//! Key advantage: TypeScript's `.d.ts` ecosystem means API surface extraction
//! for TS packages is essentially free -- npm packages ship their own contract DB.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Tree};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::resolve::public_entry_files_for_resolved_package;
use super::language_profile::{is_noise_dir, is_noise_file, strip_layout_segments};
use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the complete API surface from a TypeScript package.
///
/// # Arguments
/// * `resolved` - The resolved package with root directory and metadata
/// * `include_private` - Whether to include non-exported APIs
/// * `limit` - Optional maximum number of APIs to extract
///
/// # Returns
/// * `ApiSurface` with all extracted API entries
pub fn extract_typescript_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    // Find all TypeScript/declaration files
    let ts_files = find_typescript_files(&resolved.root_dir);

    // Extract from each file
    for file_path in &ts_files {
        let file_apis = extract_from_typescript_file(
            file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?;
        apis.extend(file_apis);
    }

    let mut alias_apis =
        synthesize_ts_reexport_aliases(&apis, &ts_files, &resolved.root_dir, &resolved.package_name);
    apis.append(&mut alias_apis);

    if !resolved.is_pure_source {
        let public_entry_files =
            public_entry_files_for_resolved_package(&resolved.root_dir, Language::TypeScript);
        if !public_entry_files.is_empty() {
            apis.retain(|api| {
                api.location
                    .as_ref()
                    .is_some_and(|location| public_entry_files.contains(&location.file))
            });
        }
    }

    sort_apis_by_static_preference(&mut apis, "typescript");

    // Apply limit if specified
    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "typescript".to_string(),
        total,
        apis,
    })
}

/// Extract API entries from a single TypeScript file.
fn extract_from_typescript_file(
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

    let is_dts = file_path
        .to_str()
        .map(|s| s.ends_with(".d.ts"))
        .unwrap_or(false);

    let tree = parse(&source, Language::TypeScript)?;
    let module_info = extract_from_tree(
        &tree,
        &source,
        Language::TypeScript,
        file_path,
        Some(root_dir),
    )?;

    let module_path = compute_ts_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    // Extract top-level functions
    for func in &module_info.functions {
        if !include_private && !is_exported(&source, func.line_number as usize, is_dts) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, func.name);
        let params = convert_ts_params(&func.params);
        let return_type = func.return_type.clone();

        let signature = Some(Signature {
            params: params.clone(),
            return_type: return_type.clone(),
            is_async: func.is_async,
            is_generator: false,
        });

        let example =
            generate_ts_function_example(&module_path, &func.name, &params, return_type.as_deref());
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

    // Extract classes and interfaces with their methods
    for class in &module_info.classes {
        if !include_private && !is_exported(&source, class.line_number as usize, is_dts) {
            continue;
        }

        let kind = determine_ts_class_kind(class, &source);
        let qualified_name = format!("{}.{}", module_path, class.name);
        let triggers = extract_triggers(&class.name, class.docstring.as_deref());

        // Add the class/interface itself
        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|d| truncate_docstring(&d)),
            example: generate_ts_class_example(&module_path, &class.name),
            triggers: triggers.clone(),
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        // Extract methods from the class
        for method in &class.methods {
            let method_qualified = format!("{}.{}", qualified_name, method.name);
            let is_prop = is_readonly_property(&source, method.line_number as usize)
                || is_getter_property(&source, method.line_number as usize);
            let is_static_method = is_static_declaration(&source, method.line_number as usize);
            let method_kind = if is_prop {
                ApiKind::Property
            } else if is_static_method {
                ApiKind::StaticMethod
            } else {
                ApiKind::Method
            };
            let params = convert_ts_params(&method.params);
            let return_type = method.return_type.clone();

            let signature = if !is_prop {
                Some(Signature {
                    params: params.clone(),
                    return_type: return_type.clone(),
                    is_async: method.is_async,
                    is_generator: false,
                })
            } else {
                None
            };

            let method_triggers = extract_triggers(&method.name, method.docstring.as_deref());

            apis.push(ApiEntry {
                qualified_name: method_qualified,
                kind: method_kind,
                module: module_path.clone(),
                signature,
                docstring: method.docstring.clone().map(|d| truncate_docstring(&d)),
                example: None,
                triggers: method_triggers,
                is_property: is_prop,
                return_type,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: method.line_number as usize,
                    column: None,
                }),
            });
        }
    }

    // Extract constants from module_info (includes UPPER_CASE constants from extract.rs)
    let mut seen_constants: std::collections::HashSet<String> = std::collections::HashSet::new();
    for constant in &module_info.constants {
        if !include_private && !is_exported(&source, constant.line_number as usize, is_dts) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, constant.name);
        let triggers = extract_triggers(&constant.name, None);
        seen_constants.insert(constant.name.clone());

        apis.push(ApiEntry {
            qualified_name,
            kind: if is_type_alias(&source, constant.line_number as usize) {
                ApiKind::TypeAlias
            } else {
                ApiKind::Constant
            },
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}.{}", module_path, constant.name)),
            triggers,
            is_property: false,
            return_type: constant.field_type.clone(),
            location: Some(Location {
                file: relative_path.clone(),
                line: constant.line_number as usize,
                column: None,
            }),
        });
    }

    // Supplementary pass: find exported const declarations missed by the
    // UPPER_CASE filter in extract_ts_module_constants. Walk the tree directly
    // for `export_statement > lexical_declaration > variable_declarator` patterns
    // where the name is not UPPER_CASE (those are already captured above).
    let extra_constants = extract_exported_const_names(&tree, &source);
    for (name, line_number, type_ann) in &extra_constants {
        if seen_constants.contains(name) {
            continue;
        }
        if !include_private && !is_exported(&source, *line_number, is_dts) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, name);
        let triggers = extract_triggers(name, None);

        apis.push(ApiEntry {
            qualified_name,
            kind: ApiKind::Constant,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}.{}", module_path, name)),
            triggers,
            is_property: false,
            return_type: type_ann.clone(),
            location: Some(Location {
                file: relative_path.clone(),
                line: *line_number,
                column: None,
            }),
        });
    }

    // Supplementary pass: extract exported enums and their members (variants).
    // We walk the tree directly to find enum_declaration nodes, emit the enum
    // itself as ApiKind::Enum, and emit each member as ApiKind::Constant with
    // qualified name EnumName.MemberName.
    //
    // This is done here rather than relying on module_info.classes because
    // extract_ts_classes_detailed does not handle enum_declaration nodes,
    // keeping this self-contained avoids touching the shared extract.rs path.
    let enums = extract_exported_enums(&tree, &source, is_dts, include_private);
    // Track which enum names we already have from module_info.classes to avoid duplicates.
    let existing_class_names: std::collections::HashSet<String> =
        module_info.classes.iter().map(|c| c.name.clone()).collect();
    for (enum_name, enum_line, members) in &enums {
        let enum_qualified = format!("{}.{}", module_path, enum_name);

        // Emit the enum entry itself only if it wasn't already captured via module_info.classes.
        if !existing_class_names.contains(enum_name) {
            let enum_triggers = extract_triggers(enum_name, None);
            apis.push(ApiEntry {
                qualified_name: enum_qualified.clone(),
                kind: ApiKind::Enum,
                module: module_path.clone(),
                signature: None,
                docstring: None,
                example: generate_ts_class_example(&module_path, enum_name),
                triggers: enum_triggers,
                is_property: false,
                return_type: None,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: *enum_line,
                    column: None,
                }),
            });
        }

        // Emit each member as a Constant.
        for (member_name, member_line, value) in members {
            let member_qualified = format!("{}.{}", enum_qualified, member_name);
            let member_triggers = extract_triggers(member_name, None);

            apis.push(ApiEntry {
                qualified_name: member_qualified,
                kind: ApiKind::Constant,
                module: module_path.clone(),
                signature: None,
                docstring: None,
                example: Some(format!("{}.{}", enum_qualified, member_name)),
                triggers: member_triggers,
                is_property: false,
                return_type: value.clone(),
                location: Some(Location {
                    file: relative_path.clone(),
                    line: *member_line,
                    column: None,
                }),
            });
        }
    }

    Ok(apis)
}

#[derive(Debug)]
enum TsReexport {
    Named {
        from_module: String,
        original: String,
        exported_as: String,
        line: usize,
    },
    All {
        from_module: String,
        line: usize,
    },
}

fn synthesize_ts_reexport_aliases(
    apis: &[ApiEntry],
    ts_files: &[PathBuf],
    root_dir: &Path,
    package_name: &str,
) -> Vec<ApiEntry> {
    let mut aliases = Vec::new();
    let mut seen_names: HashSet<String> = apis.iter().map(|api| api.qualified_name.clone()).collect();

    for file_path in ts_files {
        if compute_ts_module_path(file_path, root_dir, package_name) != package_name {
            continue;
        }

        let Ok(source) = std::fs::read_to_string(file_path) else {
            continue;
        };
        let relative_path = file_path
            .strip_prefix(root_dir)
            .unwrap_or(file_path)
            .to_path_buf();

        for reexport in parse_ts_reexports(&source, file_path, root_dir, package_name) {
            match reexport {
                TsReexport::Named {
                    from_module,
                    original,
                    exported_as,
                    line,
                } => {
                    append_ts_alias_family(
                        &mut aliases,
                        &mut seen_names,
                        apis,
                        &from_module,
                        &original,
                        &exported_as,
                        package_name,
                        &relative_path,
                        line,
                    );
                }
                TsReexport::All { from_module, line } => {
                    for symbol in top_level_ts_symbols(apis, &from_module) {
                        append_ts_alias_family(
                            &mut aliases,
                            &mut seen_names,
                            apis,
                            &from_module,
                            &symbol,
                            &symbol,
                            package_name,
                            &relative_path,
                            line,
                        );
                    }
                }
            }
        }
    }

    aliases.sort_by(|left, right| left.qualified_name.cmp(&right.qualified_name));
    aliases
}

fn parse_ts_reexports(
    source: &str,
    entrypoint_path: &Path,
    root_dir: &Path,
    package_name: &str,
) -> Vec<TsReexport> {
    let mut reexports = Vec::new();

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("export * from ") {
            if let Some(from_module) = parse_ts_reexport_target(
                trimmed,
                "export * from ",
                entrypoint_path,
                root_dir,
                package_name,
            ) {
                reexports.push(TsReexport::All {
                    from_module,
                    line: index + 1,
                });
            }
            continue;
        }

        if let Some(specifiers) = trimmed
            .strip_prefix("export {")
            .or_else(|| trimmed.strip_prefix("export{"))
        {
            let Some((specifiers, rest)) = specifiers.split_once('}') else {
                continue;
            };
            let Some(from_module) = parse_ts_reexport_target(
                rest.trim(),
                "from ",
                entrypoint_path,
                root_dir,
                package_name,
            ) else {
                continue;
            };

            for specifier in specifiers.split(',') {
                let specifier = specifier.trim();
                if specifier.is_empty() {
                    continue;
                }

                let (original, exported_as) = specifier
                    .split_once(" as ")
                    .map_or((specifier, specifier), |(left, right)| {
                        (left.trim(), right.trim())
                    });
                if original.is_empty() || exported_as.is_empty() {
                    continue;
                }

                reexports.push(TsReexport::Named {
                    from_module: from_module.clone(),
                    original: original.to_string(),
                    exported_as: exported_as.to_string(),
                    line: index + 1,
                });
            }
        }
    }

    reexports
}

fn parse_ts_reexport_target(
    statement: &str,
    prefix: &str,
    entrypoint_path: &Path,
    root_dir: &Path,
    package_name: &str,
) -> Option<String> {
    let rest = statement.strip_prefix(prefix)?.trim();
    let specifier = rest
        .trim_end_matches(';')
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    resolve_ts_reexport_module(entrypoint_path, root_dir, package_name, specifier)
}

fn resolve_ts_reexport_module(
    entrypoint_path: &Path,
    root_dir: &Path,
    package_name: &str,
    specifier: &str,
) -> Option<String> {
    if !specifier.starts_with('.') {
        return None;
    }

    let base_dir = entrypoint_path.parent().unwrap_or(root_dir);
    let target = resolve_existing_ts_reexport_path(base_dir, specifier)
        .unwrap_or_else(|| normalize_ts_reexport_path(&base_dir.join(specifier)));
    Some(compute_ts_module_path(&target, root_dir, package_name))
}

fn normalize_ts_reexport_path(path: &Path) -> PathBuf {
    path.components().fold(PathBuf::new(), |mut normalized, component| {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
        normalized
    })
}

fn resolve_existing_ts_reexport_path(base_dir: &Path, specifier: &str) -> Option<PathBuf> {
    let base = normalize_ts_reexport_path(&base_dir.join(specifier));
    let mut candidates = vec![
        base.clone(),
        base.with_extension("ts"),
        base.with_extension("tsx"),
        base.with_extension("d.ts"),
    ];
    candidates.push(base.join("index.ts"));
    candidates.push(base.join("index.tsx"));
    candidates.push(base.join("index.d.ts"));
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn top_level_ts_symbols(apis: &[ApiEntry], from_module: &str) -> Vec<String> {
    let prefix = format!("{from_module}.");
    let mut symbols: Vec<String> = apis
        .iter()
        .filter_map(|api| {
            api.qualified_name
                .strip_prefix(&prefix)
                .and_then(|suffix| suffix.split('.').next())
                .filter(|segment| !segment.is_empty())
                .map(ToOwned::to_owned)
        })
        .collect();
    symbols.sort();
    symbols.dedup();
    symbols
}

#[allow(clippy::too_many_arguments)]
fn append_ts_alias_family(
    aliases: &mut Vec<ApiEntry>,
    seen_names: &mut HashSet<String>,
    apis: &[ApiEntry],
    from_module: &str,
    original: &str,
    exported_as: &str,
    package_name: &str,
    entrypoint_relative_path: &Path,
    line: usize,
) {
    let from_prefix = format!("{from_module}.{original}");
    let to_prefix = format!("{package_name}.{exported_as}");

    for api in apis.iter().filter(|candidate| {
        candidate.qualified_name == from_prefix
            || candidate
                .qualified_name
                .starts_with(&format!("{from_prefix}."))
    }) {
        let aliased_name = api.qualified_name.replacen(&from_prefix, &to_prefix, 1);
        if !seen_names.insert(aliased_name.clone()) {
            continue;
        }

        let mut alias = api.clone();
        alias.qualified_name = aliased_name;
        alias.module = package_name.to_string();
        alias.example = alias
            .example
            .as_ref()
            .map(|example| example.replacen(from_module, package_name, 1));
        alias.location = Some(Location {
            file: entrypoint_relative_path.to_path_buf(),
            line,
            column: None,
        });
        aliases.push(alias);
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Compute module path from a TypeScript file path relative to the root.
///
/// Examples:
/// - `src/index.ts` in package "express" -> "express"
/// - `src/router.ts` in package "express" -> "express.router"
/// - `types/index.d.ts` in package "lodash" -> "lodash"
fn compute_ts_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let stem = relative.with_extension("");
    // Also strip .d from .d.ts files
    let stem_str = stem.to_string_lossy();
    let clean_stem = stem_str.trim_end_matches(".d");

    let filtered = strip_layout_segments(Language::TypeScript, Path::new(clean_stem));

    if filtered.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, filtered.join("."))
    }
}

/// Check if a declaration at a given line is exported.
///
/// For `.d.ts` files, almost everything is exported (declarations are public by default).
/// For regular `.ts` files, look for `export` keyword.
fn is_exported(source: &str, line_number: usize, is_dts: bool) -> bool {
    // In .d.ts files, all declarations are considered exported
    if is_dts {
        return true;
    }

    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }

    let line = lines[line_number - 1].trim();
    line.starts_with("export ") || line.starts_with("export{") || line.starts_with("export default")
}

/// Determine the kind of a TypeScript class definition.
///
/// Distinguishes between `class`, `interface`, and `enum`.
fn determine_ts_class_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let lines: Vec<&str> = source.lines().collect();
    if class.line_number > 0 && (class.line_number as usize) <= lines.len() {
        let line = lines[class.line_number as usize - 1].trim();
        if line.contains("interface ") {
            return ApiKind::Interface;
        }
        if line.contains("enum ") {
            return ApiKind::Enum;
        }
        if line.contains("type ") && line.contains('=') {
            return ApiKind::TypeAlias;
        }
    }
    ApiKind::Class
}

/// Check if a line defines a `readonly` property in an interface/class.
fn is_readonly_property(source: &str, line_number: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }
    let line = lines[line_number - 1].trim();
    line.starts_with("readonly ")
}

/// Check if a line declares a static member.
fn is_static_declaration(source: &str, line_number: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }
    let line = lines[line_number - 1].trim();
    line.starts_with("static ") || line.contains(" static ")
}

/// Check if a line defines a getter property.
fn is_getter_property(source: &str, line_number: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }
    let line = lines[line_number - 1].trim();
    line.starts_with("get ")
}

/// Check if a line defines a type alias.
fn is_type_alias(source: &str, line_number: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    if line_number == 0 || line_number > lines.len() {
        return false;
    }
    let line = lines[line_number - 1].trim();
    line.starts_with("type ") || line.starts_with("export type ")
}

/// Convert function parameters from `Vec<String>` to API surface Params.
///
/// Input format: `["name: string", "age: number = 0", "...rest: string[]"]`
/// Handles TypeScript parameter syntax with type annotations and defaults.
fn convert_ts_params(raw_params: &[String]) -> Vec<Param> {
    raw_params
        .iter()
        .filter(|p| {
            let trimmed = p.trim();
            // Skip `this` parameter in TypeScript
            trimmed != "this" && !trimmed.starts_with("this:")
        })
        .map(|p| {
            let p = p.trim();

            // Check for variadic (...rest)
            let is_variadic = p.starts_with("...");
            let p = if is_variadic { &p[3..] } else { p };

            // Check for optional (name?)
            let has_question = p.contains('?');

            // Split on '=' for defaults
            let (param_part, default) = if let Some(eq_idx) = p.find('=') {
                let lhs = p[..eq_idx].trim();
                let rhs = p[eq_idx + 1..].trim().to_string();
                (lhs, Some(rhs))
            } else {
                (p, None)
            };

            // Split on ':' for type annotation
            let (name, type_annotation) = if let Some(colon_idx) = param_part.find(':') {
                let n = param_part[..colon_idx].trim().trim_end_matches('?');
                let t = param_part[colon_idx + 1..].trim();
                (n.to_string(), Some(t.to_string()))
            } else {
                let n = param_part.trim().trim_end_matches('?');
                (n.to_string(), None)
            };

            let _ = has_question; // Used for future optional param tracking

            Param {
                name,
                type_annotation,
                default,
                is_variadic,
                is_keyword: false,
            }
        })
        .collect()
}

/// Generate an example usage string for a TypeScript function.
fn generate_ts_function_example(
    module: &str,
    name: &str,
    params: &[Param],
    _return_type: Option<&str>,
) -> Option<String> {
    let args: Vec<String> = params
        .iter()
        .map(|p| ts_example_for_type(p.type_annotation.as_deref()))
        .collect();
    Some(format!(
        "const result = {}.{}({});",
        module,
        name,
        args.join(", ")
    ))
}

/// Generate an example usage string for a TypeScript class.
fn generate_ts_class_example(module: &str, class_name: &str) -> Option<String> {
    let var = class_name.to_lowercase();
    Some(format!("const {} = new {}.{}();", var, module, class_name))
}

/// Generate an example argument from a TypeScript type annotation.
fn ts_example_for_type(type_ann: Option<&str>) -> String {
    match type_ann {
        Some("string") => "\"example\"".to_string(),
        Some("number") => "42".to_string(),
        Some("boolean") => "true".to_string(),
        Some("bigint") => "0n".to_string(),
        Some(t) if t.starts_with("Array") || t.ends_with("[]") => "[]".to_string(),
        Some("object") | Some("Record<string, any>") => "{}".to_string(),
        Some("null") => "null".to_string(),
        Some("undefined") | Some("void") => "undefined".to_string(),
        Some(t) if t.starts_with("Promise") => "Promise.resolve()".to_string(),
        Some(t) if t.contains('|') => {
            // Union type: use the first option
            let first = t.split('|').next().unwrap_or("").trim();
            ts_example_for_type(Some(first))
        }
        _ => "undefined".to_string(),
    }
}

/// Truncate a docstring to the first paragraph, max ~200 characters.
fn truncate_docstring(doc: &str) -> String {
    let stripped = doc
        .trim()
        .trim_start_matches("/**")
        .trim_start_matches("/*")
        .trim_end_matches("*/")
        .trim_start_matches("///")
        .trim_start_matches("//")
        .trim();

    let first_para = stripped.split("\n\n").next().unwrap_or(stripped);
    let cleaned: String = first_para
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<&str>>()
        .join(" ");

    if cleaned.len() > 200 {
        format!("{}...", &cleaned[..197])
    } else {
        cleaned
    }
}

/// Walk a directory recursively to find all TypeScript source and declaration files.
///
/// Returns paths sorted for deterministic output.
pub fn find_typescript_files(root: &Path) -> Vec<PathBuf> {
    if root.is_file() {
        return root
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.ends_with(".ts") || n.ends_with(".tsx"))
            .map(|_| vec![root.to_path_buf()])
            .unwrap_or_default();
    }
    let mut files = Vec::new();
    find_ts_files_recursive(root, &mut files);
    files.sort();
    files
}

/// Recursive helper for finding TypeScript files.
fn find_ts_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !dir_name.starts_with('.') && !is_noise_dir(Language::TypeScript, dir_name) {
                find_ts_files_recursive(&path, files);
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if (name.ends_with(".ts") || name.ends_with(".tsx"))
                && !is_noise_file(Language::TypeScript, name)
            {
                files.push(path);
            }
        }
    }
}

/// Extract all exported `const` variable names from a TypeScript tree.
///
/// This supplements `extract_ts_module_constants` in extract.rs, which only
/// captures UPPER_CASE constants. The surface extractor needs ALL exported
/// constants regardless of naming convention.
///
/// Returns `(name, line_number, type_annotation)` tuples.
fn extract_exported_const_names(tree: &Tree, source: &str) -> Vec<(String, usize, Option<String>)> {
    let mut results = Vec::new();
    let root = tree.root_node();
    walk_for_exported_consts(&root, source, &mut results);
    results
}

/// Recursive walker that finds `export_statement > lexical_declaration` patterns.
fn walk_for_exported_consts(
    node: &Node,
    source: &str,
    results: &mut Vec<(String, usize, Option<String>)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export_statement" {
            // Look for lexical_declaration children (const/let)
            let mut export_cursor = child.walk();
            for export_child in child.children(&mut export_cursor) {
                if export_child.kind() == "lexical_declaration" {
                    let text = node_text(&export_child, source);
                    if text.starts_with("const ") {
                        extract_const_declarators(&export_child, source, results);
                    }
                }
            }
        } else if child.kind() == "lexical_declaration" {
            // Top-level const without export wrapper -- these are only included
            // when `include_private` is true, which is handled by the caller
            // via is_exported check. We still collect them so the caller can filter.
            let text = node_text(&child, source);
            if text.starts_with("const ") {
                extract_const_declarators(&child, source, results);
            }
        }
    }
}

/// Extract variable_declarator names from a lexical_declaration node.
fn extract_const_declarators(
    decl_node: &Node,
    source: &str,
    results: &mut Vec<(String, usize, Option<String>)>,
) {
    let mut cursor = decl_node.walk();
    for child in decl_node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(&n, source));
            if let Some(name) = name {
                if !name.is_empty() {
                    let line_number = child.start_position().row + 1;
                    let type_ann = child.child_by_field_name("type").map(|n| {
                        node_text(&n, source)
                            .trim_start_matches(':')
                            .trim()
                            .to_string()
                    });
                    results.push((name, line_number, type_ann));
                }
            }
        }
    }
}

/// Extract all exported enums and their members from the tree.
///
/// `(member_name, member_line, optional_initializer)` for an enum member.
type EnumMember = (String, usize, Option<String>);
/// `(enum_name, enum_line, members)` for a full enum declaration.
type EnumEntry = (String, usize, Vec<EnumMember>);

/// Returns a list of `(enum_name, enum_line, members)` where `members` is a list
/// of `(member_name, member_line, value)` tuples.
///
/// - `enum_name`: the enum identifier (e.g. `"Status"`)
/// - `enum_line`: the 1-based source line of the enum declaration
/// - `member_name`: the member identifier (e.g. `"Active"`)
/// - `member_line`: the 1-based source line of the member
/// - `value`: the optional initializer value (e.g. `Some("\"active\"")`)
///
/// In `.d.ts` files all declarations are treated as exported. In regular `.ts` files
/// only enum declarations preceded by `export` keyword are included.
fn extract_exported_enums(
    tree: &Tree,
    source: &str,
    is_dts: bool,
    include_private: bool,
) -> Vec<EnumEntry> {
    let mut results = Vec::new();
    let root = tree.root_node();
    walk_for_enums(&root, source, is_dts, include_private, &mut results);
    results
}

/// Recursive walker that collects enum entries from `export_statement > enum_declaration`
/// (and bare `enum_declaration` for .d.ts / include_private mode).
fn walk_for_enums(
    node: &Node,
    source: &str,
    is_dts: bool,
    include_private: bool,
    results: &mut Vec<EnumEntry>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "export_statement" => {
                // Find enum_declaration inside the export_statement
                let mut inner_cursor = child.walk();
                for inner in child.children(&mut inner_cursor) {
                    if inner.kind() == "enum_declaration" {
                        if let Some(entry) = collect_enum_entry(&inner, source) {
                            results.push(entry);
                        }
                    }
                }
            }
            "enum_declaration" => {
                // Bare enum_declaration (not wrapped in export_statement).
                // Include if: it's a .d.ts file, or include_private is set,
                // or the line starts with `export`.
                let line = child.start_position().row + 1;
                if is_dts || include_private || is_exported(source, line, is_dts) {
                    if let Some(entry) = collect_enum_entry(&child, source) {
                        results.push(entry);
                    }
                }
            }
            _ => {
                // Recurse into other top-level nodes (e.g., module declarations)
                walk_for_enums(&child, source, is_dts, include_private, results);
            }
        }
    }
}

/// Given an `enum_declaration` node, extract the enum name, its line, and all member data.
///
/// Returns `None` if the enum name cannot be determined.
fn collect_enum_entry(enum_node: &Node, source: &str) -> Option<EnumEntry> {
    let enum_name = enum_node
        .child_by_field_name("name")
        .map(|n| node_text(&n, source))
        .unwrap_or_default();
    if enum_name.is_empty() {
        return None;
    }

    let enum_line = enum_node.start_position().row + 1;
    let mut members = Vec::new();

    // Find the enum_body child and collect members.
    //
    // In tree-sitter-typescript, enum_body children are:
    //   - `property_identifier`  for members without values: `Active,`
    //   - `enum_assignment`      for members with values: `Up = "UP",`
    //     - first child: `property_identifier` (the member name)
    //     - second child: `=` (punctuation)
    //     - third child: the value expression
    let mut cursor = enum_node.walk();
    for child in enum_node.children(&mut cursor) {
        if child.kind() == "enum_body" {
            let mut body_cursor = child.walk();
            for member in child.children(&mut body_cursor) {
                match member.kind() {
                    "property_identifier" => {
                        // Plain member with no value: `Active,`
                        let member_name = node_text(&member, source);
                        if !member_name.is_empty() {
                            let member_line = member.start_position().row + 1;
                            members.push((member_name, member_line, None));
                        }
                    }
                    "enum_assignment" => {
                        // Member with assigned value: `Up = "UP",`
                        // The first named child is the property_identifier (name).
                        let member_name = member
                            .named_child(0)
                            .map(|n| node_text(&n, source))
                            .unwrap_or_default();
                        if !member_name.is_empty() {
                            let member_line = member.start_position().row + 1;
                            // The value is the second named child.
                            let value = member.named_child(1).map(|n| node_text(&n, source));
                            members.push((member_name, member_line, value));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Some((enum_name, enum_line, members))
}

/// Get text content of a tree-sitter node.
fn node_text(node: &Node, source: &str) -> String {
    let start = node.start_byte();
    let end = node.end_byte();
    if start <= end && end <= source.len() {
        source[start..end].to_string()
    } else {
        String::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_ts_module_path_index() {
        let root = Path::new("/pkg");
        let file = Path::new("/pkg/index.ts");
        assert_eq!(compute_ts_module_path(file, root, "express"), "express");
    }

    #[test]
    fn test_compute_ts_module_path_submodule() {
        let root = Path::new("/pkg");
        let file = Path::new("/pkg/router.ts");
        assert_eq!(
            compute_ts_module_path(file, root, "express"),
            "express.router"
        );
    }

    #[test]
    fn test_compute_ts_module_path_dts() {
        let root = Path::new("/pkg");
        let file = Path::new("/pkg/index.d.ts");
        assert_eq!(compute_ts_module_path(file, root, "lodash"), "lodash");
    }

    #[test]
    fn test_compute_ts_module_path_src_dir() {
        let root = Path::new("/pkg");
        let file = Path::new("/pkg/src/utils.ts");
        assert_eq!(compute_ts_module_path(file, root, "mylib"), "mylib.utils");
    }

    #[test]
    fn test_is_exported_regular_ts() {
        let source = "function internal() {}\nexport function public() {}\n";
        assert!(!is_exported(source, 1, false));
        assert!(is_exported(source, 2, false));
    }

    #[test]
    fn test_is_exported_dts() {
        let source = "function declared(): void;\n";
        // In .d.ts files, everything is exported
        assert!(is_exported(source, 1, true));
    }

    #[test]
    fn test_determine_ts_class_kind_class() {
        let class = crate::types::ClassInfo {
            name: "MyClass".to_string(),
            line_number: 1,
            methods: vec![],
            fields: vec![],
            bases: vec![],
            decorators: vec![],
            docstring: None,
        };
        let source = "export class MyClass {\n}\n";
        assert_eq!(determine_ts_class_kind(&class, source), ApiKind::Class);
    }

    #[test]
    fn test_determine_ts_class_kind_interface() {
        let class = crate::types::ClassInfo {
            name: "MyInterface".to_string(),
            line_number: 1,
            methods: vec![],
            fields: vec![],
            bases: vec![],
            decorators: vec![],
            docstring: None,
        };
        let source = "export interface MyInterface {\n}\n";
        assert_eq!(determine_ts_class_kind(&class, source), ApiKind::Interface);
    }

    #[test]
    fn test_is_readonly_property() {
        let source = "interface Foo {\n    readonly bar: string;\n    baz: number;\n}\n";
        assert!(is_readonly_property(source, 2));
        assert!(!is_readonly_property(source, 3));
    }

    #[test]
    fn test_is_getter_property() {
        let source = "class Foo {\n    get bar(): string { return \"\"; }\n    baz() {}\n}\n";
        assert!(is_getter_property(source, 2));
        assert!(!is_getter_property(source, 3));
    }

    #[test]
    fn test_ts_example_for_type() {
        assert_eq!(ts_example_for_type(Some("string")), "\"example\"");
        assert_eq!(ts_example_for_type(Some("number")), "42");
        assert_eq!(ts_example_for_type(Some("boolean")), "true");
        assert_eq!(ts_example_for_type(Some("string[]")), "[]");
        assert_eq!(ts_example_for_type(Some("Array<number>")), "[]");
        assert_eq!(ts_example_for_type(None), "undefined");
    }

    #[test]
    fn test_truncate_docstring_short() {
        assert_eq!(truncate_docstring("Hello world"), "Hello world");
    }

    #[test]
    fn test_truncate_docstring_jsdoc() {
        let doc = "/** Parse the input data.\n * Returns a result. */";
        let result = truncate_docstring(doc);
        assert!(result.contains("Parse the input data"));
    }

    #[test]
    fn test_truncate_docstring_long() {
        let long = "a".repeat(300);
        let result = truncate_docstring(&long);
        assert!(result.len() <= 200 + 3); // 200 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_convert_ts_params() {
        let raw = vec![
            "name: string".to_string(),
            "age: number = 0".to_string(),
            "this: void".to_string(),
        ];
        let params = convert_ts_params(&raw);
        assert_eq!(params.len(), 2); // 'this' should be filtered out
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_annotation, Some("string".to_string()));
        assert_eq!(params[1].name, "age");
        assert_eq!(params[1].type_annotation, Some("number".to_string()));
        assert_eq!(params[1].default, Some("0".to_string()));
    }

    #[test]
    fn test_is_type_alias() {
        let source = "export type Foo = string | number;\nconst bar = 42;\n";
        assert!(is_type_alias(source, 1));
        assert!(!is_type_alias(source, 2));
    }

    #[test]
    fn test_find_typescript_files() {
        let tmp = std::env::temp_dir().join("tldr_test_ts_files");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("index.ts"), "export const x = 1;").unwrap();
        std::fs::write(tmp.join("utils.ts"), "export function f() {}").unwrap();
        std::fs::write(tmp.join("readme.md"), "# Docs").unwrap();

        let files = find_typescript_files(&tmp);
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|p| p.ends_with("index.ts")));
        assert!(files.iter().any(|p| p.ends_with("utils.ts")));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_typescript_files_excludes_noise_directories() {
        let tmp = std::env::temp_dir().join("tldr_test_ts_files_exclude_noise_dirs");
        let _ = std::fs::remove_dir_all(&tmp);

        for dir in [
            "src",
            "examples",
            "docs",
            "bench",
            "benchmarks",
            "fixtures",
            "tests",
            "specs",
        ] {
            std::fs::create_dir_all(tmp.join(dir)).unwrap();
        }

        std::fs::write(tmp.join("src").join("index.ts"), "export const live = 1;").unwrap();
        std::fs::write(
            tmp.join("examples").join("demo.ts"),
            "export const example = 1;",
        )
        .unwrap();
        std::fs::write(tmp.join("docs").join("config.ts"), "export const docs = 1;").unwrap();
        std::fs::write(tmp.join("bench").join("perf.ts"), "export const bench = 1;").unwrap();
        std::fs::write(
            tmp.join("benchmarks").join("perf.ts"),
            "export const bench = 1;",
        )
        .unwrap();
        std::fs::write(
            tmp.join("fixtures").join("sample.ts"),
            "export const fixture = 1;",
        )
        .unwrap();
        std::fs::write(tmp.join("tests").join("api.ts"), "export const test = 1;").unwrap();
        std::fs::write(tmp.join("specs").join("api.ts"), "export const spec = 1;").unwrap();

        let files = find_typescript_files(&tmp);
        assert_eq!(files, vec![tmp.join("src").join("index.ts")]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_find_typescript_files_excludes_test_and_bench_files() {
        let tmp = std::env::temp_dir().join("tldr_test_ts_files_exclude_noise_files");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        std::fs::write(tmp.join("index.ts"), "export const live = 1;").unwrap();
        std::fs::write(tmp.join("api.test.ts"), "export const test = 1;").unwrap();
        std::fs::write(tmp.join("api.spec.tsx"), "export const spec = 1;").unwrap();
        std::fs::write(tmp.join("api.bench.ts"), "export const bench = 1;").unwrap();
        std::fs::write(tmp.join("api.benchmark.tsx"), "export const benchmark = 1;").unwrap();
        std::fs::write(tmp.join("api.fixture.ts"), "export const fixture = 1;").unwrap();
        std::fs::write(tmp.join("types.d.ts"), "export interface PublicType {}").unwrap();

        let files = find_typescript_files(&tmp);
        assert_eq!(files, vec![tmp.join("index.ts"), tmp.join("types.d.ts")]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_typescript_api_surface_with_limit() {
        let tmp = std::env::temp_dir().join("tldr_test_ts_surface_limit");
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(
            tmp.join("index.ts"),
            "export function a(): void {}\nexport function b(): void {}\nexport function c(): void {}\n",
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "testpkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_typescript_api_surface(&resolved, false, Some(2));
        assert!(surface.is_ok());
        let s = surface.unwrap();
        assert!(s.apis.len() <= 2, "Limit should cap at 2 APIs");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_typescript_api_surface_ranks_entrypoint_before_neutral_paths() {
        let tmp = std::env::temp_dir().join(format!(
            "tldr_test_ts_surface_ranking_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("aaa_internal")).expect("create neutral dir");
        std::fs::create_dir_all(tmp.join("examples")).expect("create examples dir");
        std::fs::create_dir_all(tmp.join("src")).expect("create src dir");

        std::fs::write(
            tmp.join("aaa_internal").join("helper.ts"),
            "export function helperTool(): void {}\n",
        )
        .expect("write helper.ts");
        std::fs::write(
            tmp.join("examples").join("demo.ts"),
            "export function demoApi(): void {}\n",
        )
        .expect("write examples/demo.ts");
        std::fs::write(
            tmp.join("src").join("index.ts"),
            "export function publicApi(): void {}\n",
        )
        .expect("write src/index.ts");

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "rankedpkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_typescript_api_surface(&resolved, false, Some(1))
            .expect("extract typescript surface");

        assert_eq!(
            surface.apis.first().map(|api| api.qualified_name.as_str()),
            Some("rankedpkg.publicApi"),
            "entrypoint/public-root API should outrank neutral paths, got {:?}",
            surface
                .apis
                .iter()
                .map(|api| (&api.qualified_name, api.location.as_ref().map(|loc| &loc.file)))
                .collect::<Vec<_>>()
        );

        let full_surface = extract_typescript_api_surface(&resolved, false, None)
            .expect("extract full typescript surface");
        assert!(
            !full_surface
                .apis
                .iter()
                .any(|api| api.qualified_name.contains("demoApi")),
            "noise APIs from examples should be excluded, got {:?}",
            full_surface
                .apis
                .iter()
                .map(|api| (&api.qualified_name, api.location.as_ref().map(|loc| &loc.file)))
                .collect::<Vec<_>>()
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ========================================================================
    // Bug fix tests: TypeScript surface must extract ALL exported declaration types
    // ========================================================================

    /// Helper: create a temp dir with a single TypeScript file and extract surface.
    fn extract_surface_from_source(source: &str) -> ApiSurface {
        let tmp = std::env::temp_dir().join(format!(
            "tldr_ts_surface_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&tmp);
        std::fs::write(tmp.join("index.ts"), source).unwrap();

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "testpkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let result = extract_typescript_api_surface(&resolved, false, None)
            .expect("extraction should succeed");

        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    #[test]
    fn test_extract_exported_functions() {
        let source = r#"
export function greet(name: string): string {
    return `Hello, ${name}`;
}

export function add(a: number, b: number): number {
    return a + b;
}

function internal(): void {}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("greet")),
            "Should find exported function 'greet', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("add")),
            "Should find exported function 'add', got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("internal")),
            "Should NOT find non-exported function 'internal', got: {:?}",
            names
        );

        // Verify kind
        let greet_api = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("greet"))
            .unwrap();
        assert_eq!(greet_api.kind, ApiKind::Function);
    }

    #[test]
    fn test_extract_typescript_named_reexport_surfaces_package_alias() {
        // is_pure_source: false — simulates installed package with entry file filtering
        let tmp = std::env::temp_dir().join(format!(
            "tldr_ts_named_reexport_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("foo.ts"), "export class Foo {}\n").unwrap();
        std::fs::write(tmp.join("index.ts"), "export { Foo } from './foo';\n").unwrap();

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "pkg".to_string(),
            is_pure_source: false,
            public_names: None,
        };

        let surface = extract_typescript_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface.apis.iter().map(|api| api.qualified_name.as_str()).collect();

        assert!(
            names.contains(&"pkg.Foo"),
            "should surface package-facing alias pkg.Foo, got: {:?}",
            names
        );
        assert!(
            !names.contains(&"pkg.foo.Foo"),
            "package-facing surface should prune deep module symbol pkg.foo.Foo, got: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_typescript_export_star_surfaces_named_export_from_entrypoint() {
        let tmp = std::env::temp_dir().join(format!(
            "tldr_ts_star_reexport_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("foo.ts"),
            "export function greet(name: string): string { return name; }\n",
        )
        .unwrap();
        std::fs::write(tmp.join("index.ts"), "export * from './foo';\n").unwrap();

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "pkg".to_string(),
            is_pure_source: false,
            public_names: None,
        };

        let surface = extract_typescript_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface.apis.iter().map(|api| api.qualified_name.as_str()).collect();

        assert!(
            names.contains(&"pkg.greet"),
            "should surface package-facing alias pkg.greet, got: {:?}",
            names
        );
        assert!(
            !names.contains(&"pkg.foo.greet"),
            "package-facing surface should prune deep module symbol pkg.foo.greet, got: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_typescript_surface_prunes_non_entrypoint_internal_exports() {
        let tmp = std::env::temp_dir().join(format!(
            "tldr_ts_entrypoint_filter_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("internal")).unwrap();
        std::fs::write(
            tmp.join("index.ts"),
            "export { publicApi } from './internal/publicApi';\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("internal").join("publicApi.ts"),
            "export function publicApi(): string { return 'ok'; }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("internal").join("privateApi.ts"),
            "export function privateApi(): string { return 'no'; }\n",
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: tmp.clone(),
            package_name: "pkg".to_string(),
            is_pure_source: false,
            public_names: None,
        };

        let surface = extract_typescript_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface.apis.iter().map(|api| api.qualified_name.as_str()).collect();

        assert!(names.contains(&"pkg.publicApi"), "missing public alias: {:?}", names);
        assert!(
            !names.iter().any(|name| name.contains("privateApi")),
            "non-entrypoint internal export should be pruned: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_extract_exported_interfaces() {
        let source = r#"
export interface Account {
    name: string;
    balance: number;
}

export interface Logger {
    log(message: string): void;
    error(message: string): void;
}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        let kinds: Vec<(&str, &ApiKind)> = surface
            .apis
            .iter()
            .map(|a| (a.qualified_name.as_str(), &a.kind))
            .collect();

        assert!(
            names.iter().any(|n| n.contains("Account")),
            "Should find exported interface 'Account', got: {:?}",
            kinds
        );
        assert!(
            names.iter().any(|n| n.contains("Logger")),
            "Should find exported interface 'Logger', got: {:?}",
            kinds
        );

        // Verify they are Interface kind
        let account_api = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.ends_with(".Account"))
            .expect("Account should be found");
        assert_eq!(account_api.kind, ApiKind::Interface);
    }

    #[test]
    fn test_extract_exported_type_aliases() {
        let source = r#"
export type AccountId = string;
export type TransactionCallback = (result: boolean) => void;
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("AccountId")),
            "Should find exported type alias 'AccountId', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("TransactionCallback")),
            "Should find exported type alias 'TransactionCallback', got: {:?}",
            names
        );

        // Verify kind
        let id_api = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("AccountId"))
            .unwrap();
        assert_eq!(id_api.kind, ApiKind::TypeAlias);
    }

    #[test]
    fn test_extract_exported_constants() {
        let source = r#"
export const MAX_ACCOUNTS = 1000;
export const DEFAULT_NAME: string = "anonymous";
export const bankInstance = new Bank();
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("MAX_ACCOUNTS")),
            "Should find exported constant 'MAX_ACCOUNTS', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("DEFAULT_NAME")),
            "Should find exported constant 'DEFAULT_NAME', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("bankInstance")),
            "Should find exported constant 'bankInstance', got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_interface_methods() {
        let source = r#"
export interface Logger {
    log(message: string): void;
    error(message: string): void;
}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Interface itself should appear
        assert!(
            names.iter().any(|n| n.ends_with(".Logger")),
            "Should find interface 'Logger', got: {:?}",
            names
        );

        // Interface methods should appear
        assert!(
            names.iter().any(|n| n.contains("Logger.log")),
            "Should find method 'Logger.log', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Logger.error")),
            "Should find method 'Logger.error', got: {:?}",
            names
        );
    }

    #[test]
    fn test_comprehensive_surface_extraction() {
        // This is the bug reproduction case: a file with all declaration types
        let source = r#"
export function createAccount(name: string, balance: number): Account {
    return { name, balance };
}

export interface Account {
    name: string;
    balance: number;
}

export type AccountId = string;

export class Bank {
    private accounts: Account[] = [];

    addAccount(account: Account): void {
        this.accounts.push(account);
    }
}

export const DEFAULT_BALANCE: number = 0;

export interface TransactionResult {
    success: boolean;
    message: string;
}

export type TransactionCallback = (result: TransactionResult) => void;

export function processTransaction(from: string, to: string): TransactionResult {
    return { success: true, message: "ok" };
}

export const MAX_ACCOUNTS = 1000;
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Functions
        assert!(
            names.iter().any(|n| n.contains("createAccount")),
            "Missing exported function 'createAccount', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("processTransaction")),
            "Missing exported function 'processTransaction', got: {:?}",
            names
        );

        // Interfaces
        assert!(
            names.iter().any(|n| n.ends_with(".Account")),
            "Missing exported interface 'Account', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.ends_with(".TransactionResult")),
            "Missing exported interface 'TransactionResult', got: {:?}",
            names
        );

        // Type aliases
        assert!(
            names.iter().any(|n| n.contains("AccountId")),
            "Missing exported type alias 'AccountId', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("TransactionCallback")),
            "Missing exported type alias 'TransactionCallback', got: {:?}",
            names
        );

        // Class
        assert!(
            names.iter().any(|n| n.ends_with(".Bank")),
            "Missing exported class 'Bank', got: {:?}",
            names
        );

        // Constants
        assert!(
            names.iter().any(|n| n.contains("DEFAULT_BALANCE")),
            "Missing exported constant 'DEFAULT_BALANCE', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MAX_ACCOUNTS")),
            "Missing exported constant 'MAX_ACCOUNTS', got: {:?}",
            names
        );

        // At least 9 top-level APIs (2 functions + 2 interfaces + 2 type aliases + 1 class + 2 constants)
        // Plus class methods
        assert!(
            surface.total >= 9,
            "Should extract at least 9 top-level APIs, got {} total: {:?}",
            surface.total,
            names
        );
    }

    #[test]
    fn test_non_exported_not_included() {
        let source = r#"
export function publicFunc(): void {}

function privateFunc(): void {}

interface PrivateInterface {
    x: number;
}

type PrivateType = string;

const PRIVATE_CONST = 42;

class PrivateClass {}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("publicFunc")),
            "Should find exported 'publicFunc', got: {:?}",
            names
        );

        // None of the non-exported items should appear
        assert!(
            !names.iter().any(|n| n.contains("privateFunc")),
            "Should NOT find non-exported 'privateFunc', got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("PrivateInterface")),
            "Should NOT find non-exported 'PrivateInterface', got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("PrivateType")),
            "Should NOT find non-exported 'PrivateType', got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("PrivateClass")),
            "Should NOT find non-exported 'PrivateClass', got: {:?}",
            names
        );
    }

    // ========================================================================
    // Enum variant extraction tests
    // ========================================================================

    #[test]
    fn test_extract_enum_variants_basic() {
        let source = r#"
export enum Status {
    Active,
    Inactive,
    Pending,
}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        let kinds: Vec<(&str, &ApiKind)> = surface
            .apis
            .iter()
            .map(|a| (a.qualified_name.as_str(), &a.kind))
            .collect();

        // Enum itself should be extracted
        assert!(
            names.iter().any(|n| n.ends_with(".Status")),
            "Should find exported enum 'Status', got: {:?}",
            names
        );
        let enum_api = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.ends_with(".Status"))
            .unwrap();
        assert_eq!(enum_api.kind, ApiKind::Enum);

        // All three variants should be extracted as Constants
        assert!(
            names.iter().any(|n| n.contains("Status.Active")),
            "Should find variant 'Status.Active', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Status.Inactive")),
            "Should find variant 'Status.Inactive', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Status.Pending")),
            "Should find variant 'Status.Pending', got: {:?}",
            names
        );

        // Variants should have Constant kind
        for variant in &["Status.Active", "Status.Inactive", "Status.Pending"] {
            let entry = surface
                .apis
                .iter()
                .find(|a| a.qualified_name.contains(variant));
            assert!(
                entry.is_some(),
                "Variant {} not found, got: {:?}",
                variant,
                kinds
            );
            assert_eq!(
                entry.unwrap().kind,
                ApiKind::Constant,
                "Variant {} should be Constant kind",
                variant
            );
        }
    }

    #[test]
    fn test_extract_enum_variants_with_string_values() {
        let source = r#"
export enum Direction {
    Up = "UP",
    Down = "DOWN",
    Left = "LEFT",
    Right = "RIGHT",
}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("Direction.Up")),
            "Should find variant 'Direction.Up', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Direction.Down")),
            "Should find variant 'Direction.Down', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Direction.Left")),
            "Should find variant 'Direction.Left', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Direction.Right")),
            "Should find variant 'Direction.Right', got: {:?}",
            names
        );

        // Values should be captured in return_type
        let up = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("Direction.Up"))
            .unwrap();
        assert_eq!(
            up.return_type.as_deref(),
            Some("\"UP\""),
            "Up variant should capture value '\"UP\"'"
        );
    }

    #[test]
    fn test_non_exported_enum_not_included() {
        let source = r#"
export enum PublicStatus {
    Ok,
    Err,
}

enum PrivateStatus {
    Draft,
    Published,
}
"#;
        let surface = extract_surface_from_source(source);
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Exported enum and its variants should be present
        assert!(
            names.iter().any(|n| n.contains("PublicStatus.Ok")),
            "Should find exported variant 'PublicStatus.Ok', got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("PublicStatus.Err")),
            "Should find exported variant 'PublicStatus.Err', got: {:?}",
            names
        );

        // Non-exported enum variants should NOT be present
        assert!(
            !names.iter().any(|n| n.contains("PrivateStatus")),
            "Should NOT find non-exported enum 'PrivateStatus', got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("Draft")),
            "Should NOT find variant 'Draft' from non-exported enum, got: {:?}",
            names
        );
    }
}
