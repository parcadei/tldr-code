//! Python-specific API surface extraction.
//!
//! Extracts the complete public API surface from a Python package by:
//! 1. Walking all `.py` files in the package directory
//! 2. Using tree-sitter to parse each file and extract functions, classes, constants
//! 3. Filtering through `__all__` if present in `__init__.py`
//! 4. Detecting properties via `@property` decorator
//! 5. Generating example usage strings and trigger keywords
//! 6. Falling back to a Python inspect helper for C extensions

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, FunctionInfo, Language};
use crate::TldrResult;

use super::examples::generate_example;
use super::resolve::{find_python_files, has_c_extensions};
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the complete API surface from a Python package.
///
/// # Arguments
/// * `resolved` - The resolved package with root directory and metadata
/// * `include_private` - Whether to include private (underscore-prefixed) APIs
/// * `limit` - Optional maximum number of APIs to extract
///
/// # Returns
/// * `ApiSurface` with all extracted API entries
pub fn extract_python_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    // Check if this is a C built-in module (no parseable source)
    if super::resolve::is_builtin_marker_path(&resolved.root_dir) {
        // Use introspection-based extraction for C built-in modules
        let c_ext_apis = extract_c_extension_apis(&resolved.package_name)?;
        apis.extend(c_ext_apis);

        // Filter through __all__ if present
        if let Some(ref all_names) = resolved.public_names {
            apis.retain(|api| {
                let short_name = api
                    .qualified_name
                    .rsplit('.')
                    .next()
                    .unwrap_or(&api.qualified_name);
                all_names.iter().any(|n: &String| n.as_str() == short_name) || include_private
            });
        }

        // Apply limit if specified
        if let Some(max) = limit {
            apis.truncate(max);
        }

        let total = apis.len();
        return Ok(ApiSurface {
            package: resolved.package_name.clone(),
            language: "python".to_string(),
            total,
            apis,
        });
    }

    let package_layout = discover_python_package_layout(&resolved.root_dir, &resolved.package_name);

    // Find all Python source files
    let py_files = find_python_files(&package_layout.scan_root);

    // Extract from each file
    for file_path in &py_files {
        let file_apis = extract_from_python_file(
            file_path,
            &package_layout.scan_root,
            &resolved.package_name,
            include_private,
        )?;
        apis.extend(file_apis);
    }

    // Handle C extensions via Python inspect helper
    if has_c_extensions(&resolved.root_dir) {
        let c_ext_apis = extract_c_extension_apis(&resolved.package_name)?;
        // Only add C extension APIs for symbols not already found in source
        for api in c_ext_apis {
            if !apis
                .iter()
                .any(|a: &ApiEntry| a.qualified_name == api.qualified_name)
            {
                apis.push(api);
            }
        }
    }

    // Filter through __all__ if present
    if let Some(ref all_names) = resolved.public_names {
        apis.retain(|api| {
            // Keep if the short name is in __all__
            let short_name = api
                .qualified_name
                .rsplit('.')
                .next()
                .unwrap_or(&api.qualified_name);

            // For methods/properties, check the class name
            if matches!(
                api.kind,
                ApiKind::Method | ApiKind::ClassMethod | ApiKind::StaticMethod | ApiKind::Property
            ) {
                // Methods belong to classes that should be in __all__
                let parts: Vec<&str> = api.qualified_name.split('.').collect();
                if parts.len() >= 2 {
                    let class_name = parts[parts.len() - 2];
                    return all_names.iter().any(|n: &String| n == class_name);
                }
            }

            all_names.iter().any(|n: &String| n.as_str() == short_name) || include_private
        });
    }

    if package_layout.has_package_root && !include_private {
        apis = rewrite_and_filter_python_package_exports(
            apis,
            &resolved.package_name,
            package_layout.exports.as_ref(),
        );
    }

    // Sort: __all__ exports first, functions before classes, then alphabetical.
    // Within __all__ group: functions > classes > methods (headline APIs first).
    let all_names = resolved.public_names.clone();
    apis.sort_by(|a, b| {
        let a_in_all = is_in_all_exports(&a.qualified_name, a.kind, &all_names);
        let b_in_all = is_in_all_exports(&b.qualified_name, b.kind, &all_names);
        b_in_all
            .cmp(&a_in_all)
            .then_with(|| api_kind_rank(a.kind).cmp(&api_kind_rank(b.kind)))
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
    });

    // Apply limit if specified
    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "python".to_string(),
        total,
        apis,
    })
}

#[derive(Debug, Default)]
struct PythonPackageLayout {
    scan_root: PathBuf,
    exports: Option<PythonPackageExports>,
    has_package_root: bool,
}

#[derive(Debug, Default)]
struct PythonPackageExports {
    names: Vec<String>,
    export_order: HashMap<String, usize>,
    import_aliases: HashMap<String, String>,
}

fn discover_python_package_layout(root_dir: &Path, package_name: &str) -> PythonPackageLayout {
    let package_root = python_package_root(root_dir, package_name);
    let Some(package_root) = package_root else {
        return PythonPackageLayout {
            scan_root: root_dir.to_path_buf(),
            exports: None,
            has_package_root: false,
        };
    };

    let exports = parse_python_package_exports(&package_root.join("__init__.py"));
    PythonPackageLayout {
        scan_root: package_root,
        exports,
        has_package_root: true,
    }
}

fn python_package_root(root_dir: &Path, package_name: &str) -> Option<PathBuf> {
    if root_dir.join("__init__.py").is_file()
        && root_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == package_name)
    {
        return Some(root_dir.to_path_buf());
    }

    let direct = root_dir.join(package_name);
    if direct.join("__init__.py").is_file() {
        return Some(direct);
    }

    let src_layout = root_dir.join("src").join(package_name);
    if src_layout.join("__init__.py").is_file() {
        return Some(src_layout);
    }

    None
}

fn parse_python_package_exports(init_file: &Path) -> Option<PythonPackageExports> {
    let source = std::fs::read_to_string(init_file).ok()?;
    let mut names = Vec::new();
    let mut export_order = HashMap::new();
    let mut import_aliases = HashMap::new();

    for name in super::resolve::extract_all_names_from_source(&source).unwrap_or_default() {
        record_python_export_name(&mut names, &mut export_order, name);
    }

    for line in source.lines() {
        if let Some((module, imported)) = parse_python_reexport_line(line) {
            for (source_name, alias_name) in imported {
                record_python_export_name(&mut names, &mut export_order, alias_name.clone());
                import_aliases.insert(
                    alias_name,
                    module
                        .clone()
                        .map(|m| format!("{m}.{source_name}"))
                        .unwrap_or(source_name),
                );
            }
        }
    }

    if names.is_empty() && import_aliases.is_empty() {
        None
    } else {
        Some(PythonPackageExports {
            names,
            export_order,
            import_aliases,
        })
    }
}

fn record_python_export_name(
    names: &mut Vec<String>,
    export_order: &mut HashMap<String, usize>,
    name: String,
) {
    if export_order.contains_key(&name) {
        return;
    }

    export_order.insert(name.clone(), names.len());
    names.push(name);
}

type PythonReexport = (Option<String>, Vec<(String, String)>);

fn parse_python_reexport_line(line: &str) -> Option<PythonReexport> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') || !trimmed.starts_with("from .") {
        return None;
    }

    let remainder = trimmed.strip_prefix("from .")?;
    let (module_part, import_part) = remainder.split_once(" import ")?;
    let module = if module_part.is_empty() {
        None
    } else {
        Some(module_part.trim().to_string())
    };

    let mut imports = Vec::new();
    for item in import_part.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }

        let (source_name, alias_name) = item
            .split_once(" as ")
            .map_or((item, item), |(left, right)| (left.trim(), right.trim()));
        if !source_name.is_empty() && !alias_name.is_empty() {
            imports.push((source_name.to_string(), alias_name.to_string()));
        }
    }

    (!imports.is_empty()).then_some((module, imports))
}

fn rewrite_and_filter_python_package_exports(
    apis: Vec<ApiEntry>,
    package_name: &str,
    exports: Option<&PythonPackageExports>,
) -> Vec<ApiEntry> {
    let Some(exports) = exports else {
        return apis;
    };
    let export_names = &exports.names;
    if export_names.is_empty() {
        return apis;
    }

    let mut rewritten = Vec::new();
    for api in apis {
        let old_qualified_name = api.qualified_name.clone();
        let Some(suffix) = old_qualified_name.strip_prefix(&format!("{package_name}.")) else {
            continue;
        };
        let segments: Vec<String> = suffix.split('.').map(str::to_string).collect();
        if segments.is_empty() {
            continue;
        }

        let top_level = segments[0].as_str();
        let exported_top_level = exports
            .import_aliases
            .iter()
            .find_map(|(alias, target)| {
                target
                    .ends_with(&format!(".{top_level}"))
                    .then_some(alias.as_str())
            })
            .or_else(|| {
                export_names
                    .iter()
                    .find(|name| name.as_str() == top_level)
                    .map(String::as_str)
            });

        let class_export = if segments.len() >= 2 {
            exports
                .import_aliases
                .iter()
                .find_map(|(alias, target)| {
                    target
                        .ends_with(&format!(".{}", segments[1]))
                        .then_some(alias.as_str())
                })
                .or_else(|| {
                    export_names
                        .iter()
                        .find(|name| name.as_str() == segments[1])
                        .map(String::as_str)
                })
        } else {
            None
        };

        let Some(exported_name) = exported_top_level.or(class_export) else {
            continue;
        };

        let new_segments: Vec<String> = if top_level == exported_name {
            segments
        } else if segments.len() >= 2 && segments[1] == exported_name {
            let mut rewritten_segments = vec![exported_name.to_string()];
            rewritten_segments.extend_from_slice(&segments[2..]);
            rewritten_segments
        } else {
            let mut rewritten_segments = vec![exported_name.to_string()];
            rewritten_segments.extend_from_slice(&segments[1..]);
            rewritten_segments
        };

        let mut api = api;
        api.module = package_name.to_string();
        api.qualified_name = format!("{package_name}.{}", new_segments.join("."));
        if let Some(example) = api.example.as_mut() {
            *example = example.replace(&old_qualified_name, &api.qualified_name);
        }

        let export_name = new_segments
            .first()
            .cloned()
            .unwrap_or_else(|| exported_name.to_string());
        let export_rank = exports
            .export_order
            .get(&export_name)
            .copied()
            .unwrap_or(usize::MAX);
        let is_member = new_segments.len() > 1;

        rewritten.push((is_member, export_rank, api.qualified_name.clone(), api));
    }

    rewritten.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut apis: Vec<ApiEntry> = rewritten.into_iter().map(|(_, _, _, api)| api).collect();
    apis.dedup_by(|left, right| {
        left.qualified_name == right.qualified_name && left.kind == right.kind
    });
    apis
}

/// Extract API entries from a single Python file.
///
/// Gracefully skips files that cannot be read as valid UTF-8 (e.g., binary
/// test fixtures, compiled `.pyc` files with `.py` extension, or corrupt files).
/// This prevents crashes when scanning stdlib packages like `csv` that may
/// contain non-UTF-8 test data.
fn extract_from_python_file(
    file_path: &Path,
    root_dir: &Path,
    package_name: &str,
    include_private: bool,
) -> TldrResult<Vec<ApiEntry>> {
    let source = match std::fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(_) => {
            // Skip files that cannot be read as UTF-8 (binary data, corrupt files, etc.)
            return Ok(Vec::new());
        }
    };

    let tree = parse(&source, Language::Python)?;

    // Use extract_from_tree to get module info
    let module_info =
        extract_from_tree(&tree, &source, Language::Python, file_path, Some(root_dir))?;

    // Compute module path from file path relative to root
    let module_path = compute_module_path(file_path, root_dir, package_name);
    // For single-file modules root_dir == file_path, so strip_prefix yields "".
    // Fall back to the file name in that case so locations are meaningful.
    let stripped = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let relative_path = if stripped.as_os_str().is_empty() {
        file_path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| file_path.to_path_buf())
    } else {
        stripped.to_path_buf()
    };

    let mut apis = Vec::new();

    // Extract top-level functions
    for func in &module_info.functions {
        if !include_private && is_private_name(&func.name) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, func.name);
        let params = extract_rich_params_from_source(&tree, &source, &func.name, false);
        let kind = determine_function_kind(func);
        let return_type = func.return_type.clone();

        let signature = Some(Signature {
            params: params.clone(),
            return_type: return_type.clone(),
            is_async: func.is_async,
            is_generator: is_generator_function(&tree, &source, &func.name),
        });

        let example = generate_example(&module_path, &func.name, kind, &params, None);

        let triggers = extract_triggers(&func.name, func.docstring.as_deref());

        let docstring = func.docstring.as_ref().map(|d| truncate_docstring(d));

        apis.push(ApiEntry {
            qualified_name,
            kind,
            module: module_path.clone(),
            signature,
            docstring,
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

    // Extract classes and their methods
    for class in &module_info.classes {
        if !include_private && is_private_name(&class.name) {
            continue;
        }

        let class_qualified = format!("{}.{}", module_path, class.name);

        // Extract constructor params from __init__ if present
        let init_params = class
            .methods
            .iter()
            .find(|m| m.name == "__init__")
            .map(|_init| extract_rich_params_from_source(&tree, &source, "__init__", true))
            .unwrap_or_default();

        let class_docstring = class.docstring.as_ref().map(|d| truncate_docstring(d));

        let class_example = generate_example(
            &module_path,
            &class.name,
            ApiKind::Class,
            &init_params,
            None,
        );

        let class_triggers = extract_triggers(&class.name, class.docstring.as_deref());

        apis.push(ApiEntry {
            qualified_name: class_qualified.clone(),
            kind: ApiKind::Class,
            module: module_path.clone(),
            signature: if init_params.is_empty() {
                None
            } else {
                Some(Signature {
                    params: init_params,
                    return_type: None,
                    is_async: false,
                    is_generator: false,
                })
            },
            docstring: class_docstring,
            example: class_example,
            triggers: class_triggers,
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        // Extract methods
        let ctx = MethodExtractionCtx {
            tree: &tree,
            source: &source,
            class,
            class_qualified: &class_qualified,
            module_path: &module_path,
            relative_path: &relative_path,
            include_private,
        };
        extract_class_methods(&ctx, &mut apis);
    }

    // Extract module-level constants
    for constant in &module_info.constants {
        if !include_private && is_private_name(&constant.name) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, constant.name);

        apis.push(ApiEntry {
            qualified_name,
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

/// Context for method extraction, bundling parameters to reduce argument count.
struct MethodExtractionCtx<'a> {
    tree: &'a tree_sitter::Tree,
    source: &'a str,
    class: &'a ClassInfo,
    class_qualified: &'a str,
    module_path: &'a str,
    relative_path: &'a Path,
    include_private: bool,
}

/// Extract methods from a class and add them to the API list.
fn extract_class_methods(ctx: &MethodExtractionCtx, apis: &mut Vec<ApiEntry>) {
    for method in &ctx.class.methods {
        // Skip __init__ (already handled as class constructor)
        if method.name == "__init__" {
            continue;
        }
        // Skip dunder methods unless include_private is set
        if method.name.starts_with("__") && method.name.ends_with("__") && !ctx.include_private {
            continue;
        }
        if !ctx.include_private && is_private_name(&method.name) {
            continue;
        }

        let qualified_name = format!("{}.{}", ctx.class_qualified, method.name);
        let kind = determine_method_kind(method);
        let is_prop = kind == ApiKind::Property;
        let params = extract_rich_params_from_source(ctx.tree, ctx.source, &method.name, true);
        let return_type = method.return_type.clone();

        let signature = if is_prop {
            None
        } else {
            Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: method.is_async,
                is_generator: false,
            })
        };

        let example = generate_example(
            ctx.module_path,
            &method.name,
            kind,
            &params,
            Some(&ctx.class.name),
        );

        let triggers = extract_triggers(&method.name, method.docstring.as_deref());
        let docstring = method.docstring.as_ref().map(|d| truncate_docstring(d));

        apis.push(ApiEntry {
            qualified_name,
            kind,
            module: ctx.module_path.to_string(),
            signature,
            docstring,
            example,
            triggers,
            is_property: is_prop,
            return_type,
            location: Some(Location {
                file: ctx.relative_path.to_path_buf(),
                line: method.line_number as usize,
                column: None,
            }),
        });
    }
}

/// Compute the Python module path from a file path.
///
/// e.g., `root/flask/app.py` with package `flask` -> `flask.app`
/// e.g., `root/json/__init__.py` with package `json` -> `json`
fn compute_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);

    let stem = relative.with_extension("");
    let parts: Vec<&str> = stem
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    if parts.is_empty() {
        return package_name.to_string();
    }

    // If it's __init__.py, the module path is just the directory
    if parts.last() == Some(&"__init__") {
        let dir_parts: Vec<&str> = parts[..parts.len() - 1].to_vec();
        if dir_parts.is_empty() {
            return package_name.to_string();
        }
        return format!("{}.{}", package_name, dir_parts.join("."));
    }

    format!("{}.{}", package_name, parts.join("."))
}

/// Extract rich parameter information using tree-sitter.
///
/// This provides more detail than the basic `extract_python_params` which
/// only returns parameter names. This version extracts type annotations,
/// default values, and variadic/keyword markers.
fn extract_rich_params_from_source(
    tree: &tree_sitter::Tree,
    source: &str,
    func_name: &str,
    _is_method: bool,
) -> Vec<Param> {
    let root = tree.root_node();
    let mut params = Vec::new();

    // Extract params by traversing to the function and collecting directly
    collect_params_for_function(&root, source, func_name, &mut params);

    params
}

/// Traverse the tree to find a function by name and collect its parameters.
fn collect_params_for_function(
    node: &tree_sitter::Node,
    source: &str,
    func_name: &str,
    params: &mut Vec<Param>,
) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if node_text(&name_node, source) == func_name {
                        if let Some(params_node) = child.child_by_field_name("parameters") {
                            let mut pcursor = params_node.walk();
                            for pchild in params_node.children(&mut pcursor) {
                                if let Some(param) = extract_rich_param(&pchild, source) {
                                    params.push(param);
                                }
                            }
                        }
                        return true;
                    }
                }
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(name_node) = def.child_by_field_name("name") {
                            if node_text(&name_node, source) == func_name {
                                if let Some(params_node) = def.child_by_field_name("parameters") {
                                    let mut pcursor = params_node.walk();
                                    for pchild in params_node.children(&mut pcursor) {
                                        if let Some(param) = extract_rich_param(&pchild, source) {
                                            params.push(param);
                                        }
                                    }
                                }
                                return true;
                            }
                        }
                    }
                }
            }
            _ => {
                if collect_params_for_function(&child, source, func_name, params) {
                    return true;
                }
            }
        }
    }
    false
}

/// Extract a single rich parameter from a tree-sitter node.
fn extract_rich_param(node: &tree_sitter::Node, source: &str) -> Option<Param> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            Some(Param {
                name,
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: false,
            })
        }
        "typed_parameter" => {
            let name = node
                .child(0)
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let type_ann = node
                .child_by_field_name("type")
                .map(|n| node_text(&n, source));
            Some(Param {
                name,
                type_annotation: type_ann,
                default: None,
                is_variadic: false,
                is_keyword: false,
            })
        }
        "default_parameter" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let default = node
                .child_by_field_name("value")
                .map(|n| node_text(&n, source));
            Some(Param {
                name,
                type_annotation: None,
                default,
                is_variadic: false,
                is_keyword: false,
            })
        }
        "typed_default_parameter" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let type_ann = node
                .child_by_field_name("type")
                .map(|n| node_text(&n, source));
            let default = node
                .child_by_field_name("value")
                .map(|n| node_text(&n, source));
            Some(Param {
                name,
                type_annotation: type_ann,
                default,
                is_variadic: false,
                is_keyword: false,
            })
        }
        "list_splat_pattern" => {
            // *args -- the identifier child is after the "*" punctuation
            let name = find_child_identifier(node, source).unwrap_or_else(|| "args".to_string());
            Some(Param {
                name,
                type_annotation: None,
                default: None,
                is_variadic: true,
                is_keyword: false,
            })
        }
        "dictionary_splat_pattern" => {
            // **kwargs -- the identifier child is after the "**" punctuation
            let name = find_child_identifier(node, source).unwrap_or_else(|| "kwargs".to_string());
            Some(Param {
                name,
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: true,
            })
        }
        _ => None,
    }
}

/// Determine the API kind for a top-level function based on its decorators.
fn determine_function_kind(func: &FunctionInfo) -> ApiKind {
    for dec in &func.decorators {
        if dec == "staticmethod" {
            return ApiKind::StaticMethod;
        }
        if dec == "classmethod" {
            return ApiKind::ClassMethod;
        }
        if dec == "property" || dec.starts_with("property") {
            return ApiKind::Property;
        }
    }
    ApiKind::Function
}

/// Determine the API kind for a method based on its decorators.
fn determine_method_kind(method: &FunctionInfo) -> ApiKind {
    for dec in &method.decorators {
        let dec_lower = dec.to_lowercase();
        if dec_lower == "staticmethod" {
            return ApiKind::StaticMethod;
        }
        if dec_lower == "classmethod" {
            return ApiKind::ClassMethod;
        }
        if dec_lower == "property"
            || dec_lower.ends_with(".setter")
            || dec_lower.ends_with(".getter")
        {
            return ApiKind::Property;
        }
    }
    ApiKind::Method
}

/// Check if a function contains yield/yield from (is a generator).
fn is_generator_function(tree: &tree_sitter::Tree, source: &str, func_name: &str) -> bool {
    let root = tree.root_node();
    check_generator_recursive(&root, source, func_name)
}

/// Recursively search for a function and check if it contains yield.
fn check_generator_recursive(node: &tree_sitter::Node, source: &str, func_name: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if node_text(&name_node, source) == func_name {
                        if let Some(body) = child.child_by_field_name("body") {
                            return contains_yield(&body);
                        }
                        return false;
                    }
                }
            }
            "decorated_definition" => {
                if let Some(def) = child.child_by_field_name("definition") {
                    if def.kind() == "function_definition" {
                        if let Some(name_node) = def.child_by_field_name("name") {
                            if node_text(&name_node, source) == func_name {
                                if let Some(body) = def.child_by_field_name("body") {
                                    return contains_yield(&body);
                                }
                                return false;
                            }
                        }
                    }
                }
            }
            _ => {
                if check_generator_recursive(&child, source, func_name) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a node or its descendants contain a yield expression.
fn contains_yield(node: &tree_sitter::Node) -> bool {
    if node.kind() == "yield" || node.kind() == "yield_from" {
        return true;
    }
    // Don't recurse into nested function definitions
    if node.kind() == "function_definition" {
        return false;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if contains_yield(&child) {
            return true;
        }
    }
    false
}

/// Check if a name is private (starts with underscore but not dunder).
fn api_kind_rank(kind: ApiKind) -> u8 {
    match kind {
        ApiKind::Function => 0,
        ApiKind::Class | ApiKind::Struct | ApiKind::Trait | ApiKind::Interface | ApiKind::Enum => 1,
        ApiKind::Constant | ApiKind::TypeAlias => 2,
        ApiKind::Method | ApiKind::ClassMethod | ApiKind::StaticMethod => 3,
        ApiKind::Property => 4,
    }
}

fn is_in_all_exports(qualified_name: &str, kind: ApiKind, all_names: &Option<Vec<String>>) -> bool {
    let Some(ref names) = all_names else {
        return false;
    };
    let short_name = qualified_name.rsplit('.').next().unwrap_or(qualified_name);
    // For methods, check if the owning class is in __all__
    if matches!(
        kind,
        ApiKind::Method | ApiKind::ClassMethod | ApiKind::StaticMethod | ApiKind::Property
    ) {
        let parts: Vec<&str> = qualified_name.split('.').collect();
        if parts.len() >= 2 {
            let class_name = parts[parts.len() - 2];
            return names.iter().any(|n| n == class_name);
        }
    }
    names.iter().any(|n| n.as_str() == short_name)
}

fn is_private_name(name: &str) -> bool {
    name.starts_with('_') && !name.starts_with("__")
}

/// Truncate a docstring to ~200 characters, keeping the first paragraph.
fn truncate_docstring(doc: &str) -> String {
    // Strip surrounding quotes if present
    let cleaned = doc
        .trim()
        .trim_start_matches("\"\"\"")
        .trim_start_matches("'''")
        .trim_end_matches("\"\"\"")
        .trim_end_matches("'''")
        .trim();

    // Take first paragraph (up to blank line)
    let first_para = cleaned
        .split("\n\n")
        .next()
        .unwrap_or(cleaned)
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ");

    if first_para.len() <= 200 {
        first_para
    } else {
        format!("{}...", &first_para[..197])
    }
}

/// Find the first identifier child of a node.
fn find_child_identifier(node: &tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(node_text(&child, source));
        }
    }
    None
}

/// Get text content from a tree-sitter node.
fn node_text(node: &tree_sitter::Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// C extension helper script embedded in the binary.
///
/// This script is piped to `python3 -c` to extract API info from
/// compiled Python packages (e.g., numpy, duckdb) where tree-sitter
/// cannot parse the `.so`/`.pyd` files.
const C_EXTENSION_HELPER: &str = r#"
import inspect, json, sys, importlib

pkg_name = sys.argv[1] if len(sys.argv) > 1 else input()
mod = importlib.import_module(pkg_name)
apis = []

for name, obj in inspect.getmembers(mod):
    if name.startswith('_'):
        continue
    entry = {"name": name, "module": pkg_name, "qualified_name": f"{pkg_name}.{name}"}
    if inspect.isfunction(obj) or inspect.isbuiltin(obj):
        entry["kind"] = "Function"
        try:
            sig = inspect.signature(obj)
            entry["params"] = [
                {"name": p.name,
                 "type_annotation": str(p.annotation) if p.annotation != inspect.Parameter.empty else None,
                 "default": str(p.default) if p.default != inspect.Parameter.empty else None,
                 "is_variadic": p.kind == inspect.Parameter.VAR_POSITIONAL,
                 "is_keyword": p.kind == inspect.Parameter.VAR_KEYWORD}
                for p in sig.parameters.values()
            ]
            if sig.return_annotation != inspect.Parameter.empty:
                entry["return_type"] = str(sig.return_annotation)
        except (ValueError, TypeError):
            entry["params"] = []
        entry["docstring"] = (inspect.getdoc(obj) or "")[:200]
    elif inspect.isclass(obj):
        entry["kind"] = "Class"
        entry["docstring"] = (inspect.getdoc(obj) or "")[:200]
        try:
            sig = inspect.signature(obj)
            entry["params"] = [
                {"name": p.name,
                 "type_annotation": str(p.annotation) if p.annotation != inspect.Parameter.empty else None,
                 "default": str(p.default) if p.default != inspect.Parameter.empty else None,
                 "is_variadic": p.kind == inspect.Parameter.VAR_POSITIONAL,
                 "is_keyword": p.kind == inspect.Parameter.VAR_KEYWORD}
                for p in sig.parameters.values()
            ]
        except (ValueError, TypeError):
            entry["params"] = []
    else:
        entry["kind"] = "Constant"
        entry["docstring"] = None
        entry["params"] = []
    apis.append(entry)

print(json.dumps(apis))
"#;

/// Extract API entries from a C extension module using Python's inspect module.
fn extract_c_extension_apis(package_name: &str) -> TldrResult<Vec<ApiEntry>> {
    use std::process::Command;

    let output = Command::new("python3")
        .arg("-c")
        .arg(C_EXTENSION_HELPER)
        .arg(package_name)
        .output()
        .map_err(|e| {
            crate::error::TldrError::parse_error(
                std::path::PathBuf::new(),
                None,
                format!(
                    "Failed to run C extension helper for '{}': {}",
                    package_name, e
                ),
            )
        })?;

    if !output.status.success() {
        // C extension inspection is best-effort -- don't fail the whole extraction
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<serde_json::Value> = serde_json::from_str(stdout.trim()).unwrap_or_default();

    let mut apis = Vec::new();
    for entry in entries {
        let name = entry["name"].as_str().unwrap_or("").to_string();
        let kind_str = entry["kind"].as_str().unwrap_or("Function");
        let kind = match kind_str {
            "Class" => ApiKind::Class,
            "Constant" => ApiKind::Constant,
            _ => ApiKind::Function,
        };

        let params: Vec<Param> = entry["params"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| Param {
                        name: p["name"].as_str().unwrap_or("").to_string(),
                        type_annotation: p["type_annotation"].as_str().map(|s| s.to_string()),
                        default: p["default"].as_str().map(|s| s.to_string()),
                        is_variadic: p["is_variadic"].as_bool().unwrap_or(false),
                        is_keyword: p["is_keyword"].as_bool().unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let docstring = entry["docstring"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let qualified_name = entry["qualified_name"]
            .as_str()
            .unwrap_or(&name)
            .to_string();
        let module = entry["module"].as_str().unwrap_or(package_name).to_string();

        let return_type = entry["return_type"].as_str().map(|s| s.to_string());

        let signature = if params.is_empty() && kind == ApiKind::Constant {
            None
        } else {
            Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: false,
                is_generator: false,
            })
        };

        let example = generate_example(&module, &name, kind, &params, None);
        let triggers = extract_triggers(&name, docstring.as_deref());

        apis.push(ApiEntry {
            qualified_name,
            kind,
            module,
            signature,
            docstring,
            example,
            triggers,
            is_property: false,
            return_type,
            location: None,
        });
    }

    Ok(apis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_compute_module_path_init() {
        let root = PathBuf::from("/site-packages/json");
        let file = PathBuf::from("/site-packages/json/__init__.py");
        assert_eq!(compute_module_path(&file, &root, "json"), "json");
    }

    #[test]
    fn test_compute_module_path_submodule() {
        let root = PathBuf::from("/site-packages/flask");
        let file = PathBuf::from("/site-packages/flask/app.py");
        assert_eq!(compute_module_path(&file, &root, "flask"), "flask.app");
    }

    #[test]
    fn test_compute_module_path_nested() {
        let root = PathBuf::from("/site-packages/flask");
        let file = PathBuf::from("/site-packages/flask/helpers/utils.py");
        assert_eq!(
            compute_module_path(&file, &root, "flask"),
            "flask.helpers.utils"
        );
    }

    #[test]
    fn test_is_private_name() {
        assert!(is_private_name("_helper"));
        assert!(is_private_name("_internal_func"));
        assert!(!is_private_name("public_func"));
        assert!(!is_private_name("__init__")); // dunder is not private
        assert!(!is_private_name("__all__"));
    }

    #[test]
    fn test_truncate_docstring_short() {
        assert_eq!(truncate_docstring("Short doc."), "Short doc.");
    }

    #[test]
    fn test_truncate_docstring_multiline() {
        let doc = "First paragraph summary.\n\nSecond paragraph with details.\nMore details.";
        assert_eq!(truncate_docstring(doc), "First paragraph summary.");
    }

    #[test]
    fn test_truncate_docstring_with_quotes() {
        let doc = "\"\"\"Deserialize s to a Python object.\"\"\"";
        assert_eq!(truncate_docstring(doc), "Deserialize s to a Python object.");
    }

    #[test]
    fn test_truncate_docstring_long() {
        let long_doc = "A".repeat(300);
        let result = truncate_docstring(&long_doc);
        assert!(result.len() <= 200);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_determine_method_kind_property() {
        let method = FunctionInfo {
            name: "url_map".to_string(),
            params: vec!["self".to_string()],
            return_type: None,
            docstring: None,
            is_method: true,
            is_async: false,
            decorators: vec!["property".to_string()],
            line_number: 10,
        };
        assert_eq!(determine_method_kind(&method), ApiKind::Property);
    }

    #[test]
    fn test_determine_method_kind_static() {
        let method = FunctionInfo {
            name: "from_data".to_string(),
            params: vec!["data".to_string()],
            return_type: None,
            docstring: None,
            is_method: true,
            is_async: false,
            decorators: vec!["staticmethod".to_string()],
            line_number: 10,
        };
        assert_eq!(determine_method_kind(&method), ApiKind::StaticMethod);
    }

    #[test]
    fn test_determine_method_kind_classmethod() {
        let method = FunctionInfo {
            name: "create".to_string(),
            params: vec!["cls".to_string()],
            return_type: None,
            docstring: None,
            is_method: true,
            is_async: false,
            decorators: vec!["classmethod".to_string()],
            line_number: 10,
        };
        assert_eq!(determine_method_kind(&method), ApiKind::ClassMethod);
    }

    #[test]
    fn test_determine_method_kind_regular() {
        let method = FunctionInfo {
            name: "do_something".to_string(),
            params: vec!["self".to_string(), "x".to_string()],
            return_type: None,
            docstring: None,
            is_method: true,
            is_async: false,
            decorators: vec![],
            line_number: 10,
        };
        assert_eq!(determine_method_kind(&method), ApiKind::Method);
    }

    #[test]
    fn test_extract_rich_params_from_inline_source() {
        let source = r#"
def greet(name: str, greeting: str = "Hello", *args, **kwargs) -> str:
    """Greet someone."""
    return f"{greeting}, {name}!"
"#;
        let tree = parse(source, Language::Python).unwrap();
        let params = extract_rich_params_from_source(&tree, source, "greet", false);

        assert_eq!(params.len(), 4);
        assert_eq!(params[0].name, "name");
        assert_eq!(params[0].type_annotation, Some("str".to_string()));
        assert_eq!(params[0].default, None);

        assert_eq!(params[1].name, "greeting");
        assert_eq!(params[1].type_annotation, Some("str".to_string()));
        assert_eq!(params[1].default, Some("\"Hello\"".to_string()));

        assert_eq!(params[2].name, "args");
        assert!(params[2].is_variadic);

        assert_eq!(params[3].name, "kwargs");
        assert!(params[3].is_keyword);
    }

    #[test]
    fn test_extract_from_python_source_inline() {
        let source = r#"
"""Module docstring."""

VERSION = "1.0"

def public_func(x: int) -> str:
    """Convert int to string."""
    return str(x)

def _private_func():
    pass

class MyClass:
    """A sample class."""

    def __init__(self, name: str):
        self.name = name

    def greet(self) -> str:
        """Return greeting."""
        return f"Hello, {self.name}"

    @property
    def upper_name(self) -> str:
        return self.name.upper()

    @staticmethod
    def create(name: str) -> 'MyClass':
        return MyClass(name)
"#;

        // Create a temp file for extraction
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_extract");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let file_path = tmp_dir.join("sample.py");
        std::fs::write(&file_path, source).unwrap();

        let apis = extract_from_python_file(&file_path, &tmp_dir, "sample", false).unwrap();

        // Should have: public_func, MyClass, greet, upper_name, create, VERSION
        // Should NOT have: _private_func, __init__
        let names: Vec<&str> = apis.iter().map(|a| a.qualified_name.as_str()).collect();
        assert!(
            names.contains(&"sample.sample.public_func"),
            "missing public_func: {:?}",
            names
        );
        assert!(
            names.contains(&"sample.sample.MyClass"),
            "missing MyClass: {:?}",
            names
        );
        assert!(
            names.contains(&"sample.sample.MyClass.greet"),
            "missing greet: {:?}",
            names
        );
        assert!(
            names.contains(&"sample.sample.MyClass.upper_name"),
            "missing upper_name: {:?}",
            names
        );
        assert!(
            names.contains(&"sample.sample.MyClass.create"),
            "missing create: {:?}",
            names
        );

        assert!(!names.contains(&"sample.sample._private_func"));
        assert!(!names.contains(&"sample.sample.MyClass.__init__"));

        // Check kinds
        let public_func = apis
            .iter()
            .find(|a| a.qualified_name.ends_with("public_func"))
            .unwrap();
        assert_eq!(public_func.kind, ApiKind::Function);

        let my_class = apis
            .iter()
            .find(|a| {
                a.qualified_name.ends_with("MyClass") && !a.qualified_name.contains("MyClass.")
            })
            .unwrap();
        assert_eq!(my_class.kind, ApiKind::Class);

        let upper_name = apis
            .iter()
            .find(|a| a.qualified_name.ends_with("upper_name"))
            .unwrap();
        assert_eq!(upper_name.kind, ApiKind::Property);
        assert!(upper_name.is_property);

        let create = apis
            .iter()
            .find(|a| a.qualified_name.ends_with("create"))
            .unwrap();
        assert_eq!(create.kind, ApiKind::StaticMethod);

        // Clean up
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_generator_detection() {
        let source = r#"
def gen_numbers(n: int):
    """Generate numbers up to n."""
    for i in range(n):
        yield i

def not_a_generator(n: int) -> int:
    return n * 2
"#;
        let tree = parse(source, Language::Python).unwrap();
        assert!(is_generator_function(&tree, source, "gen_numbers"));
        assert!(!is_generator_function(&tree, source, "not_a_generator"));
    }

    #[test]
    fn test_extract_skips_non_utf8_files() {
        // Bug: extract_from_python_file crashes on non-UTF-8 .py files
        // (e.g., binary test fixtures in stdlib packages like csv).
        // The extraction should gracefully skip such files instead of crashing.
        let tmp_dir = std::env::temp_dir().join("tldr_test_non_utf8_py");
        let _ = std::fs::create_dir_all(&tmp_dir);

        // Create a valid Python file
        let valid_file = tmp_dir.join("valid_module.py");
        std::fs::write(&valid_file, "def hello():\n    return 'world'\n").unwrap();

        // Create a binary (non-UTF-8) file with .py extension
        let binary_file = tmp_dir.join("binary_data.py");
        std::fs::write(&binary_file, [0xFF, 0xFE, 0x00, 0x01, 0x80, 0x81, 0x82]).unwrap();

        // Extraction should NOT crash -- it should skip the binary file
        // and successfully extract from the valid file.
        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "test_pkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let result = extract_python_api_surface(&resolved, false, None);
        assert!(
            result.is_ok(),
            "extract_python_api_surface should not crash on non-UTF-8 files, got: {:?}",
            result.err()
        );

        let surface = result.unwrap();
        // Should have extracted at least the hello() function from the valid file
        assert!(
            surface
                .apis
                .iter()
                .any(|a| a.qualified_name.contains("hello")),
            "Should extract hello() from the valid file, got: {:?}",
            surface
                .apis
                .iter()
                .map(|a| &a.qualified_name)
                .collect::<Vec<_>>()
        );

        // Clean up
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_skips_test_directories() {
        // Files in test directories (tests/, test_*.py) should be skippable
        // to avoid processing test fixtures that may contain invalid content.
        let tmp_dir = std::env::temp_dir().join("tldr_test_skip_testdirs");
        let _ = std::fs::create_dir_all(&tmp_dir);

        // Create a valid Python file
        let valid_file = tmp_dir.join("core.py");
        std::fs::write(&valid_file, "def main():\n    pass\n").unwrap();

        // Create a tests directory with a binary .py file (simulates test fixtures)
        let tests_dir = tmp_dir.join("tests");
        let _ = std::fs::create_dir_all(&tests_dir);
        let test_fixture = tests_dir.join("test_data.py");
        std::fs::write(&test_fixture, [0xFF, 0xFE, 0x00, 0x01]).unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "test_pkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let result = extract_python_api_surface(&resolved, false, None);
        assert!(
            result.is_ok(),
            "Should not crash on binary files in test directories, got: {:?}",
            result.err()
        );

        // Clean up
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_skips_docs_and_examples() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_surface_docs_filter");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("samplepkg");
        let docs_dir = tmp_dir.join("docs");
        let docs_src_dir = tmp_dir.join("docs_src");
        let examples_dir = tmp_dir.join("examples");

        std::fs::create_dir_all(&package_dir).unwrap();
        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::create_dir_all(&docs_src_dir).unwrap();
        std::fs::create_dir_all(&examples_dir).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "from .api import public_api\n__all__ = ['public_api']\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("api.py"),
            "def public_api():\n    return 'real'\n",
        )
        .unwrap();
        std::fs::write(
            docs_dir.join("conf.py"),
            "def docs_api():\n    return 'docs'\n",
        )
        .unwrap();
        std::fs::write(
            docs_src_dir.join("tutorial.py"),
            "def tutorial_api():\n    return 'tutorial'\n",
        )
        .unwrap();
        std::fs::write(
            examples_dir.join("basic.py"),
            "def example_api():\n    return 'example'\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "samplepkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|name| name.ends_with("public_api")),
            "expected public package API to remain visible: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("docs_api")),
            "docs API should not leak into surface extraction: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("tutorial_api")),
            "docs_src API should not leak into surface extraction: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("example_api")),
            "example API should not leak into surface extraction: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_repo_root_src_layout_rewrites_package_exports() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_repo_root_src_layout");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("src").join("click");
        std::fs::create_dir_all(&package_dir).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "from .core import Command as Command\nfrom .decorators import command as command\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("core.py"),
            "class Command:\n    def invoke(self) -> None:\n        pass\n\nclass Context:\n    pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("decorators.py"),
            "def command() -> None:\n    pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("_compat.py"),
            "def get_best_encoding() -> str:\n    return 'utf-8'\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "click".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"click.Command"),
            "expected package-facing click.Command export, got: {:?}",
            names
        );
        assert!(
            names.contains(&"click.Command.invoke"),
            "expected methods on exported classes to be package-facing, got: {:?}",
            names
        );
        assert!(
            names.contains(&"click.command"),
            "expected package-facing click.command export, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("src.click")),
            "src layout should not leak into qualified names, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("_compat")),
            "internal helper modules should not surface when not exported, got: {:?}",
            names
        );
        assert!(
            !names.contains(&"click.Context"),
            "non-exported classes should not surface from repo-root extraction, got: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_repo_root_prefers_package_exports_over_internal_modules() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_repo_root_exports");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("fastapi");
        std::fs::create_dir_all(package_dir.join("_compat")).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "from .applications import FastAPI as FastAPI\nfrom .routing import APIRouter as APIRouter\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("applications.py"),
            "class FastAPI:\n    def get(self) -> None:\n        pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("routing.py"),
            "class APIRouter:\n    def include_router(self) -> None:\n        pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("_compat").join("shared.py"),
            "def lenient_issubclass() -> bool:\n    return True\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "fastapi".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, Some(10)).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"fastapi.FastAPI"),
            "expected package-facing FastAPI export, got: {:?}",
            names
        );
        assert!(
            names.contains(&"fastapi.FastAPI.get"),
            "expected methods on exported classes to survive filtering, got: {:?}",
            names
        );
        assert!(
            names.contains(&"fastapi.APIRouter"),
            "expected package-facing APIRouter export, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("_compat")),
            "internal compatibility helpers should not outrank package exports, got: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_repo_root_uses_package_all_for_dynamic_exports() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_repo_root_all_exports");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("pydantic");
        std::fs::create_dir_all(package_dir.join("_internal")).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "__all__ = ('BaseModel',)\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("main.py"),
            "class BaseModel:\n    def model_dump(self) -> dict:\n        return {}\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("_internal").join("_config.py"),
            "def prepare_config() -> None:\n    pass\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "pydantic".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"pydantic.BaseModel"),
            "expected __all__ export to rewrite class to package root, got: {:?}",
            names
        );
        assert!(
            names.contains(&"pydantic.BaseModel.model_dump"),
            "expected methods of __all__-exported classes to remain visible, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("_internal")),
            "internal pydantic helpers should not surface when not exported, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|name| name.contains("pydantic.pydantic")),
            "repo-root extraction should not duplicate package segments, got: {:?}",
            names
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_ranks_top_level_exports_before_members() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_export_ranking");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("fastapi");
        std::fs::create_dir_all(&package_dir).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "from .applications import FastAPI as FastAPI\nfrom .routing import APIRouter as APIRouter\nfrom .background import BackgroundTasks as BackgroundTasks\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("applications.py"),
            "class FastAPI:\n    def get(self) -> None:\n        pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("routing.py"),
            "class APIRouter:\n    def include_router(self) -> None:\n        pass\n",
        )
        .unwrap();
        std::fs::write(
            package_dir.join("background.py"),
            "class BackgroundTasks:\n    def add_task(self) -> None:\n        pass\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "fastapi".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, Some(3)).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();

        assert_eq!(
            names,
            vec![
                "fastapi.APIRouter",
                "fastapi.BackgroundTasks",
                "fastapi.FastAPI",
            ],
            "top-level package exports should outrank member methods under tight limits"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_extract_python_api_surface_rewrites_examples_to_package_exports() {
        let tmp_dir = std::env::temp_dir().join("tldr_test_python_export_examples");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let package_dir = tmp_dir.join("src").join("click");
        std::fs::create_dir_all(&package_dir).unwrap();

        std::fs::write(
            package_dir.join("__init__.py"),
            "from .core import Command as Command\nfrom .decorators import command as command\n",
        )
        .unwrap();
        std::fs::write(package_dir.join("core.py"), "class Command:\n    pass\n").unwrap();
        std::fs::write(
            package_dir.join("decorators.py"),
            "def command() -> Command:\n    return Command()\n",
        )
        .unwrap();

        let resolved = super::super::types::ResolvedPackage {
            root_dir: tmp_dir.clone(),
            package_name: "click".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_python_api_surface(&resolved, false, None).unwrap();
        let command_class = surface
            .apis
            .iter()
            .find(|api| api.qualified_name == "click.Command")
            .expect("missing click.Command");
        let command_fn = surface
            .apis
            .iter()
            .find(|api| api.qualified_name == "click.command")
            .expect("missing click.command");

        assert_eq!(
            command_class.example.as_deref(),
            Some("command = click.Command()"),
            "class examples should use package-facing names"
        );
        assert_eq!(
            command_fn.example.as_deref(),
            Some("result = click.command()"),
            "function examples should use package-facing names"
        );

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    // ========================================================================
    // Python C extension / built-in module extraction tests
    // ========================================================================

    #[test]
    fn test_extract_builtin_module_api_surface() {
        // For C built-in modules (no __file__), extraction should use introspection
        // and return meaningful API entries instead of crashing.
        let resolved = super::super::types::ResolvedPackage {
            root_dir: PathBuf::from("<builtin:itertools>"),
            package_name: "itertools".to_string(),
            is_pure_source: false,
            public_names: None,
        };

        let result = extract_python_api_surface(&resolved, false, None);
        assert!(
            result.is_ok(),
            "Should extract API surface from built-in module, got: {:?}",
            result.err()
        );

        let surface = result.unwrap();
        assert_eq!(surface.package, "itertools");
        assert_eq!(surface.language, "python");

        // Should have extracted at least some entries via introspection
        assert!(
            !surface.apis.is_empty(),
            "Built-in module should have at least some API entries"
        );

        // Check that well-known itertools items appear
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("chain")),
            "itertools surface should include 'chain', got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_builtin_module_sys() {
        // sys is a C built-in with functions and constants
        let resolved = super::super::types::ResolvedPackage {
            root_dir: PathBuf::from("<builtin:sys>"),
            package_name: "sys".to_string(),
            is_pure_source: false,
            public_names: None,
        };

        let result = extract_python_api_surface(&resolved, false, None);
        assert!(
            result.is_ok(),
            "Should extract API surface from sys built-in, got: {:?}",
            result.err()
        );

        let surface = result.unwrap();
        assert!(!surface.apis.is_empty(), "sys should have API entries");
    }

    #[test]
    fn test_builtin_module_marker_path() {
        // Verify that the special <builtin:X> path pattern is detected correctly
        use super::super::resolve::is_builtin_marker_path;

        let path = PathBuf::from("<builtin:itertools>");
        assert!(
            is_builtin_marker_path(&path),
            "Should detect <builtin:itertools> as a builtin marker path"
        );

        let normal_path = PathBuf::from("/usr/lib/python3/itertools.py");
        assert!(
            !is_builtin_marker_path(&normal_path),
            "Should NOT detect a normal path as builtin marker"
        );
    }
}
