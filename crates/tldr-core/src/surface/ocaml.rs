//! OCaml-specific API surface extraction.
//!
//! OCaml's public boundary is the interface file (`.mli`). When a module has a
//! `.mli`, only items declared in the signature are visible from outside. When
//! a module has only a `.ml` (no `.mli`), every top-level binding is public by
//! default.
//!
//! # Heuristic (default when `include_private = false`)
//!
//! For each `.ml` file:
//!
//! 1. If a sibling `.mli` exists (same path, swapped extension), parse the
//!    `.mli` and surface only the names it declares (`val name : type`,
//!    `module Name : ...`, `type t = ...`).
//! 2. If no `.mli` exists, surface every top-level `let` binding from the
//!    `.ml` (these are public by language default).
//!
//! Names beginning with `_` are conventionally private and are filtered when
//! `include_private = false`.
//!
//! `.mli` files that have no corresponding `.ml` are also surfaced directly.
//!
//! # Heuristic (when `include_private = true`)
//!
//! Every top-level `let` binding from every `.ml`/`.mli` is surfaced
//! regardless of whether a more restrictive `.mli` would have hidden it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser, Tree};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::error::TldrError;
use crate::types::Language;
use crate::TldrResult;

use super::sort_apis_by_static_preference;
use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the public OCaml API surface for a resolved package.
pub fn extract_ocaml_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    let files = find_ocaml_files(&resolved.root_dir);

    // Index every .mli we find, so when we walk the .ml files we can decide
    // whether to defer to the interface.
    let mli_paths: HashSet<PathBuf> = files
        .iter()
        .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("mli"))
        .cloned()
        .collect();

    for file_path in &files {
        let ext = file_path.extension().and_then(|ext| ext.to_str());
        match ext {
            Some("ml") => {
                let sibling_mli = file_path.with_extension("mli");
                let has_sibling_mli = mli_paths.contains(&sibling_mli);
                apis.extend(extract_from_ml_file(
                    file_path,
                    &resolved.root_dir,
                    &resolved.package_name,
                    include_private,
                    has_sibling_mli,
                )?);
            }
            Some("mli") => {
                apis.extend(extract_from_mli_file(
                    file_path,
                    &resolved.root_dir,
                    &resolved.package_name,
                    include_private,
                )?);
            }
            _ => {}
        }
    }

    sort_apis_by_static_preference(&mut apis, "ocaml");

    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "ocaml".to_string(),
        total,
        apis,
    })
}

fn find_ocaml_files(dir: &Path) -> Vec<PathBuf> {
    if dir.is_file() {
        return dir
            .extension()
            .and_then(|ext| ext.to_str())
            .filter(|ext| matches!(*ext, "ml" | "mli"))
            .map(|_| vec![dir.to_path_buf()])
            .unwrap_or_default();
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !name.starts_with('.') && name != "_build" && name != "_opam" {
                        files.extend(find_ocaml_files(&path));
                    }
                }
            } else if matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("ml" | "mli")
            ) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn extract_from_ml_file(
    file_path: &Path,
    root_dir: &Path,
    package_name: &str,
    include_private: bool,
    has_sibling_mli: bool,
) -> TldrResult<Vec<ApiEntry>> {
    // If a sibling .mli exists and the caller is not asking for private items,
    // the .mli is the canonical public surface — let extract_from_mli_file
    // handle exposure.
    if has_sibling_mli && !include_private {
        return Ok(Vec::new());
    }

    let source = std::fs::read_to_string(file_path).map_err(|e| {
        crate::error::TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            format!("Cannot read: {}", e),
        )
    })?;

    let tree = parse(&source, Language::Ocaml)?;
    let module_info =
        extract_from_tree(&tree, &source, Language::Ocaml, file_path, Some(root_dir))?;
    let module_path = compute_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    // Top-level let bindings that take parameters are functions. The AST
    // extractor already filters value bindings (no parameters) -- those we
    // surface separately as constants below.
    for func in module_info.functions {
        let is_underscore = func.name.starts_with('_');
        if is_underscore && !include_private {
            continue;
        }

        let params: Vec<Param> = func
            .params
            .iter()
            .map(|name| Param {
                name: name.clone(),
                type_annotation: None,
                default: None,
                is_variadic: false,
                is_keyword: false,
            })
            .collect();

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
                "{}.{} {}",
                module_path,
                func.name,
                params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
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

    // Top-level value bindings without parameters (`let x = ...`) are
    // surfaced as constants. The AST extractor's `extract_module_constants`
    // already covers OCaml.
    for constant in module_info.constants {
        let is_underscore = constant.name.starts_with('_');
        if is_underscore && !include_private {
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
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: constant.line_number as usize,
                column: None,
            }),
        });
    }

    // Walk the tree directly for top-level `module M = ...` and `type t = ...`
    // declarations -- the AST extractor doesn't surface these as classes.
    apis.extend(extract_modules_and_types(
        &tree,
        &source,
        &module_path,
        &relative_path,
    ));

    Ok(apis)
}

fn extract_from_mli_file(
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

    let tree = parse_ocaml_interface(&source, file_path)?;
    let module_path = compute_module_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    // Walk the .mli signature for `val name : type` declarations.
    let signatures = extract_value_specifications(&tree, &source);
    for spec in signatures {
        let is_underscore = spec.name.starts_with('_');
        if is_underscore && !include_private {
            continue;
        }

        // A value specification's type tells us whether the value is a
        // function (contains an `->`) or a plain value.
        let is_function = spec.type_text.contains("->");
        let kind = if is_function {
            ApiKind::Function
        } else {
            ApiKind::Constant
        };

        let signature = if is_function {
            Some(Signature {
                params: derive_params_from_signature(&spec.type_text),
                return_type: derive_return_type_from_signature(&spec.type_text),
                is_async: false,
                is_generator: false,
            })
        } else {
            None
        };

        apis.push(ApiEntry {
            qualified_name: format!("{}.{}", module_path, spec.name),
            kind,
            module: module_path.clone(),
            signature,
            docstring: spec.docstring,
            example: Some(format!("{}.{}", module_path, spec.name)),
            triggers: extract_triggers(&spec.name, None),
            is_property: false,
            return_type: if is_function {
                derive_return_type_from_signature(&spec.type_text)
            } else {
                Some(spec.type_text.clone())
            },
            location: Some(Location {
                file: relative_path.clone(),
                line: spec.line,
                column: None,
            }),
        });
    }

    apis.extend(extract_modules_and_types(
        &tree,
        &source,
        &module_path,
        &relative_path,
    ));

    Ok(apis)
}

/// Parse a `.mli` interface file using tree-sitter-ocaml's dedicated
/// interface grammar (`LANGUAGE_OCAML_INTERFACE`).
///
/// The shared [`parse`] helper only knows about [`Language::Ocaml`], which
/// resolves to the implementation grammar. That grammar does not understand
/// `.mli`-only syntax such as `val name : type`. The interface grammar does,
/// and tree-sitter-ocaml ships it as a separate language.
fn parse_ocaml_interface(source: &str, file_path: &Path) -> TldrResult<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_ocaml::LANGUAGE_OCAML_INTERFACE.into())
        .map_err(|e| {
            TldrError::parse_error(
                file_path.to_path_buf(),
                None,
                format!("Failed to load OCaml interface grammar: {}", e),
            )
        })?;
    parser.parse(source, None).ok_or_else(|| {
        TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            "OCaml interface parser returned no tree".to_string(),
        )
    })
}

fn compute_module_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let mut parts: Vec<String> = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect();
    if let Some(last) = parts.pop() {
        let stem = Path::new(&last)
            .file_stem()
            .map(|s| capitalize_first(&s.to_string_lossy()))
            .unwrap_or(last);
        if !stem.is_empty() {
            parts.push(stem);
        }
    }
    if parts.is_empty() {
        package_name.to_string()
    } else {
        format!("{}.{}", package_name, parts.join("."))
    }
}

/// In OCaml, module names are derived from file names with the leading letter
/// uppercased (e.g., `main.ml` -> `Main`).
fn capitalize_first(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[derive(Debug)]
struct ValueSpec {
    name: String,
    type_text: String,
    docstring: Option<String>,
    line: usize,
}

/// Walk a parsed OCaml interface tree for `value_specification` nodes
/// (`val name : type`).
///
/// Tree-sitter-ocaml represents these uniformly across `.ml` and `.mli`. The
/// node has children `val` + `value_name` + `:` + a type expression. We pick
/// out the name, the textual representation of the type, and any
/// `(** ... *)` doc comment that immediately precedes the spec.
fn extract_value_specifications(tree: &Tree, source: &str) -> Vec<ValueSpec> {
    let mut out = Vec::new();
    walk_for_value_specs(tree.root_node(), source, &mut out);
    out
}

fn walk_for_value_specs(node: Node<'_>, source: &str, out: &mut Vec<ValueSpec>) {
    if node.kind() == "value_specification" {
        let mut name: Option<String> = None;
        let mut type_text: Option<String> = None;

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "value_name" => {
                    name = child
                        .utf8_text(source.as_bytes())
                        .ok()
                        .map(|s| s.trim().to_string());
                }
                kind if kind != "val" && kind != ":" => {
                    // The first non-`val`/`:`/name child is the type expression.
                    if name.is_some() && type_text.is_none() {
                        type_text = child
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.trim().to_string());
                    }
                }
                _ => {}
            }
        }

        if let (Some(name), Some(type_text)) = (name, type_text) {
            let line = node.start_position().row + 1;
            let docstring = extract_ocaml_doc_before(node, source);
            out.push(ValueSpec {
                name,
                type_text,
                docstring,
                line,
            });
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_value_specs(child, source, out);
    }
}

fn extract_ocaml_doc_before(node: Node<'_>, source: &str) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = sibling.utf8_text(source.as_bytes()).ok()?;
            let trimmed = text.trim();
            if trimmed.starts_with("(**") {
                let inner = trimmed
                    .strip_prefix("(**")
                    .and_then(|s| s.strip_suffix("*)"))
                    .unwrap_or(trimmed);
                return Some(inner.trim().to_string());
            }
            prev = sibling.prev_sibling();
        } else {
            break;
        }
    }
    None
}

fn derive_params_from_signature(type_text: &str) -> Vec<Param> {
    // Split by top-level `->`. Each leading segment is a parameter type.
    // The last segment is the return type.
    let parts = split_top_level_arrows(type_text);
    if parts.len() <= 1 {
        return Vec::new();
    }
    parts[..parts.len() - 1]
        .iter()
        .enumerate()
        .map(|(idx, ty)| Param {
            name: format!("arg{}", idx + 1),
            type_annotation: Some(ty.trim().to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        })
        .collect()
}

fn derive_return_type_from_signature(type_text: &str) -> Option<String> {
    let parts = split_top_level_arrows(type_text);
    parts.last().map(|s| s.trim().to_string())
}

fn split_top_level_arrows(s: &str) -> Vec<String> {
    let mut depth: i32 = 0;
    let mut parts = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        match ch {
            '(' | '[' | '{' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                current.push(ch);
            }
            '-' if depth == 0 && i + 1 < chars.len() && chars[i + 1] == '>' => {
                parts.push(current.trim().to_string());
                current.clear();
                i += 2;
                continue;
            }
            _ => current.push(ch),
        }
        i += 1;
    }
    parts.push(current.trim().to_string());
    parts
}

fn extract_modules_and_types(
    tree: &Tree,
    source: &str,
    module_path: &str,
    relative_path: &Path,
) -> Vec<ApiEntry> {
    let mut out = Vec::new();
    walk_for_modules_and_types(
        tree.root_node(),
        source,
        module_path,
        relative_path,
        &mut out,
    );
    out
}

fn walk_for_modules_and_types(
    node: Node<'_>,
    source: &str,
    module_path: &str,
    relative_path: &Path,
    out: &mut Vec<ApiEntry>,
) {
    let kind = node.kind();
    match kind {
        "module_definition" => {
            // module_definition wraps a module_binding, which exposes a
            // `module_name` child holding the bound name.
            if let Some(name) = first_module_name(node, source) {
                let line = node.start_position().row + 1;
                out.push(ApiEntry {
                    qualified_name: format!("{}.{}", module_path, name),
                    kind: ApiKind::Class,
                    module: module_path.to_string(),
                    signature: None,
                    docstring: extract_ocaml_doc_before(node, source),
                    example: Some(format!("{}.{}", module_path, name)),
                    triggers: extract_triggers(&name, None),
                    is_property: false,
                    return_type: None,
                    location: Some(Location {
                        file: relative_path.to_path_buf(),
                        line,
                        column: None,
                    }),
                });
            }
        }
        "type_definition" => {
            if let Some(name) = first_type_constructor_name(node, source) {
                let line = node.start_position().row + 1;
                out.push(ApiEntry {
                    qualified_name: format!("{}.{}", module_path, name),
                    kind: ApiKind::TypeAlias,
                    module: module_path.to_string(),
                    signature: None,
                    docstring: extract_ocaml_doc_before(node, source),
                    example: Some(format!("{}.{}", module_path, name)),
                    triggers: extract_triggers(&name, None),
                    is_property: false,
                    return_type: None,
                    location: Some(Location {
                        file: relative_path.to_path_buf(),
                        line,
                        column: None,
                    }),
                });
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_modules_and_types(child, source, module_path, relative_path, out);
    }
}

fn first_module_name(node: Node<'_>, source: &str) -> Option<String> {
    // Search the immediate `module_binding` child for a `module_name` node.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "module_binding" {
            let mut inner = child.walk();
            for grand in child.children(&mut inner) {
                if grand.kind() == "module_name" {
                    if let Ok(text) = grand.utf8_text(source.as_bytes()) {
                        return Some(text.trim().to_string());
                    }
                }
            }
        }
        if child.kind() == "module_name" {
            if let Ok(text) = child.utf8_text(source.as_bytes()) {
                return Some(text.trim().to_string());
            }
        }
    }
    None
}

fn first_type_constructor_name(node: Node<'_>, source: &str) -> Option<String> {
    // type_definition wraps one or more `type_binding` children. Each
    // binding has a leading `type_constructor` child (the name).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_binding" {
            let mut inner = child.walk();
            for grand in child.children(&mut inner) {
                if grand.kind() == "type_constructor" {
                    if let Ok(text) = grand.utf8_text(source.as_bytes()) {
                        return Some(text.trim().to_string());
                    }
                }
            }
        }
    }
    None
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
    fn test_extract_ocaml_let_bindings_from_ml() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.ml",
            "(** Greet someone *)\n\
             let greet name = print_string name\n\n\
             let add x y = x + y\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, false, None).unwrap();
        assert_eq!(surface.language, "ocaml");
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with(".greet")),
            "expected greet to surface; got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.ends_with(".add")),
            "expected add to surface; got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_ocaml_underscore_filtered_when_private_excluded() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.ml",
            "let _internal x = x\n\
             let public_one x = x + 1\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|n| n.ends_with(".public_one")));
        assert!(
            !names.iter().any(|n| n.ends_with("._internal")),
            "underscore-prefixed bindings should be private when include_private=false"
        );
    }

    #[test]
    fn test_extract_ocaml_underscore_included_when_private_requested() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "main.ml", "let _internal x = x\n");

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, true, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("._internal")));
    }

    #[test]
    fn test_extract_ocaml_mli_signature_file_is_public_surface() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.ml",
            "let helper x = x\n\
             let public_one x = x + 1\n\
             let internal_only () = ()\n",
        );
        // The .mli only exposes `public_one`. `helper` and `internal_only`
        // should be hidden when include_private=false.
        write_file(
            &dir,
            "main.mli",
            "(** Add one to the input *)\n\
             val public_one : int -> int\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with(".public_one")),
            "expected public_one in mli to surface; got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.ends_with(".helper")),
            "expected helper (not in mli) to be hidden; got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.ends_with(".internal_only")),
            "expected internal_only (not in mli) to be hidden; got: {:?}",
            names
        );

        // The .mli's `val public_one : int -> int` should produce a function
        // signature with the correct return type.
        let entry = surface
            .apis
            .iter()
            .find(|api| api.qualified_name.ends_with(".public_one"))
            .unwrap();
        assert!(matches!(entry.kind, ApiKind::Function));
        assert!(entry.signature.is_some());
        let sig = entry.signature.as_ref().unwrap();
        assert_eq!(sig.return_type.as_deref(), Some("int"));
        assert_eq!(sig.params.len(), 1);
    }

    #[test]
    fn test_extract_ocaml_mli_with_include_private_surfaces_ml_internals() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.ml",
            "let helper x = x\n\
             let public_one x = x + 1\n",
        );
        write_file(&dir, "main.mli", "val public_one : int -> int\n");

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, true, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with(".helper")),
            "include_private=true should also surface .ml internals; got: {:?}",
            names
        );
        assert!(names.iter().any(|n| n.ends_with(".public_one")));
    }

    #[test]
    fn test_extract_ocaml_module_path_uses_capitalized_filename() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "util.ml", "let b_util () = 2\n");

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "x".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|api| api.qualified_name.as_str())
            .collect();
        // OCaml module name is the file's stem with a capitalized first letter.
        assert!(
            names.iter().any(|n| n.contains("Util.b_util")),
            "expected qualified name to include capitalised module 'Util'; got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_ocaml_modules_and_types_from_ml() {
        let dir = TempDir::new().unwrap();
        write_file(
            &dir,
            "main.ml",
            "module Inner = struct\n  let x = 0\nend\n\n\
             type color = Red | Green | Blue\n",
        );

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "example".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_ocaml_api_surface(&resolved, false, None).unwrap();
        let entries: Vec<(&str, ApiKind)> = surface
            .apis
            .iter()
            .map(|api| (api.qualified_name.as_str(), api.kind))
            .collect();

        assert!(
            entries
                .iter()
                .any(|(name, kind)| name.ends_with(".Inner") && matches!(kind, ApiKind::Class)),
            "expected Inner module to surface as Class; got: {:?}",
            entries
        );
        assert!(
            entries
                .iter()
                .any(|(name, kind)| name.ends_with(".color") && matches!(kind, ApiKind::TypeAlias)),
            "expected color type to surface as TypeAlias; got: {:?}",
            entries
        );
    }

    #[test]
    fn test_extract_ocaml_split_top_level_arrows() {
        // helper unit test for the type-signature splitter
        let parts = split_top_level_arrows("int -> int -> int");
        assert_eq!(parts, vec!["int", "int", "int"]);
        let parts = split_top_level_arrows("(int -> int) -> bool");
        assert_eq!(parts, vec!["(int -> int)", "bool"]);
    }
}
