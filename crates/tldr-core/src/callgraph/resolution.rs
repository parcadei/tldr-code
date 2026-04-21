//! Call Resolution Engine (Phase 6 modularization)
//!
//! This module contains the call resolution logic: strategies 0-9 for resolving
//! call sites to their target definitions, type-aware resolution, and FP guards.
//!
//! Extracted from builder_v2.rs as part of the Phase 14 modularization.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use super::cross_file_types::{
    CallSite, CallType, ClassDef, FileIR, FuncDef, VarType,
};
use super::import_resolver::{ReExportTracer, DEFAULT_MAX_DEPTH};
use super::type_resolver::{resolve_receiver_type};
use crate::types::Language;

// From new sibling modules:
use super::types::{FuncIndex, ClassIndex, ClassEntry, capitalize_first};
use super::module_path::path_to_module;
use super::imports::{ImportMap, ModuleImports};

// =============================================================================
// Phase 14e: Call Extraction and Resolution (Spec Section 14.6)
// =============================================================================

/// A resolved call target representing the location of a function/method definition.
///
/// This struct captures the final destination of a call site after import resolution,
/// re-export tracing, and type-aware method resolution.
///
/// # Example
/// ```rust,ignore
/// // For: from helper import process; process()
/// // Resolves to:
/// ResolvedTarget {
///     file: PathBuf::from("helper.py"),
///     name: "process".to_string(),
///     line: Some(5),
///     is_method: false,
///     class_name: None,
/// }
///
/// // For: user.save() where user: User
/// // Resolves to:
/// ResolvedTarget {
///     file: PathBuf::from("models.py"),
///     name: "save".to_string(),
///     line: Some(42),
///     is_method: true,
///     class_name: Some("User".to_string()),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    /// File path containing the definition (relative to project root).
    pub file: PathBuf,

    /// Name of the function/method.
    pub name: String,

    /// Line number of definition (1-indexed), if known.
    pub line: Option<u32>,

    /// True if this is a method of a class.
    pub is_method: bool,

    /// Containing class name if `is_method` is true.
    pub class_name: Option<String>,
}

impl ResolvedTarget {
    /// Creates a ResolvedTarget for a standalone function.
    pub fn function(file: PathBuf, name: impl Into<String>, line: Option<u32>) -> Self {
        Self {
            file,
            name: name.into(),
            line,
            is_method: false,
            class_name: None,
        }
    }

    /// Creates a ResolvedTarget for a method.
    pub fn method(
        file: PathBuf,
        name: impl Into<String>,
        class_name: impl Into<String>,
        line: Option<u32>,
    ) -> Self {
        Self {
            file,
            name: name.into(),
            line,
            is_method: true,
            class_name: Some(class_name.into()),
        }
    }

    /// Returns the qualified name (Class.method or just name).
    pub fn qualified_name(&self) -> String {
        if let Some(ref class) = self.class_name {
            format!("{}.{}", class, self.name)
        } else {
            self.name.clone()
        }
    }
}

/// Shared context required to resolve calls in a file.
pub struct ResolutionContext<'a, 'b> {
    /// Maps local names to `(module_path, original_name)`.
    pub import_map: &'a ImportMap,
    /// Maps module aliases to resolved module paths.
    pub module_imports: &'a ModuleImports,
    /// Global index of discovered functions.
    pub func_index: &'a FuncIndex,
    /// Global index of discovered classes.
    pub class_index: &'a ClassIndex,
    /// Re-export tracer used to follow package indirections.
    pub reexport_tracer: &'a mut ReExportTracer<'b>,
    /// Relative path to the file currently being resolved.
    pub current_file: &'a Path,
    /// Project root path.
    pub root: &'a Path,
    /// Language identifier used for language-specific resolution behavior.
    pub language: &'a str,
}

/// Returns candidate constructor method names for a language.
fn constructor_method_candidates(language: &str, class_name: &str) -> Vec<String> {
    match language.to_lowercase().as_str() {
        "python" => vec!["__init__".to_string()],
        "ruby" => vec!["initialize".to_string()],
        "php" => vec!["__construct".to_string()],
        "typescript" | "javascript" => vec!["constructor".to_string()],
        "swift" => vec!["init".to_string()],
        "kotlin" => vec!["init".to_string(), "constructor".to_string()],
        "java" | "csharp" | "cpp" => vec![class_name.to_string()],
        "scala" => vec![class_name.to_string()],
        _ => Vec::new(),
    }
}

/// Resolve a constructor call for a class if the constructor method is known.
pub(crate) fn resolve_constructor_target(
    class_name: &str,
    class_entry: &ClassEntry,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    for ctor_name in constructor_method_candidates(language, class_name) {
        if class_entry.methods.contains(&ctor_name) {
            let qualified = format!("{}.{}", class_name, ctor_name);
            let module = path_to_module(&class_entry.file_path, language);
            if let Some(entry) = func_index.get(&module, &qualified) {
                return Some(ResolvedTarget::method(
                    entry.file_path.clone(),
                    ctor_name,
                    class_name.to_string(),
                    Some(entry.line),
                ));
            }

            return Some(ResolvedTarget::method(
                class_entry.file_path.clone(),
                ctor_name,
                class_name.to_string(),
                Some(class_entry.line),
            ));
        }
    }

    None
}

/// Compute the import path used to resolve a call, if any.
pub(crate) fn compute_via_import(
    call_site: &CallSite,
    import_map: &ImportMap,
    module_imports: &ModuleImports,
) -> Option<String> {
    match call_site.call_type {
        CallType::Method | CallType::Attr => {
            if let Some(ref receiver) = call_site.receiver {
                if let Some(module_path) = module_imports.get(receiver) {
                    return Some(module_path.clone());
                }
                if let Some((module_path, original_name)) = import_map.get(receiver) {
                    return Some(format!("{}.{}", module_path, original_name));
                }
            }
            None
        }
        CallType::Direct | CallType::Ref | CallType::Static => {
            if let Some((module_path, _)) = import_map.get(&call_site.target) {
                return Some(module_path.clone());
            }
            None
        }
        CallType::Intra => None,
    }
}

pub(crate) fn enclosing_class_for_call(funcs: &[FuncDef], call_site: &CallSite) -> Option<String> {
    let line = call_site.line?;
    let mut best: Option<&FuncDef> = None;
    let mut best_span: u32 = u32::MAX;

    for func in funcs {
        if line < func.line || line > func.end_line {
            continue;
        }
        let span = func.end_line.saturating_sub(func.line);
        if span < best_span {
            best_span = span;
            best = Some(func);
        }
    }

    if let Some(func) = best {
        if let Some(class_name) = &func.class_name {
            return Some(class_name.clone());
        }
    }

    if let Some((class_name, _)) = call_site.caller.split_once('.') {
        return Some(class_name.to_string());
    }

    let mut unique: Option<String> = None;
    for func in funcs {
        if func.name == call_site.caller {
            if let Some(class_name) = &func.class_name {
                if let Some(ref existing) = unique {
                    if existing != class_name {
                        return None;
                    }
                } else {
                    unique = Some(class_name.clone());
                }
            }
        }
    }

    unique
}

pub(crate) fn first_base_for_class(classes: &[ClassDef], class_name: &str) -> Option<String> {
    classes
        .iter()
        .find(|class_def| class_def.name == class_name)
        .and_then(|class_def| class_def.bases.first())
        .cloned()
}

/// Apply type resolution to method/attribute calls in a FileIR.
pub fn apply_type_resolution(file_ir: &mut FileIR, source: &str, language: Language) {
    let supports_type_resolution = matches!(
        language,
        Language::Python
            | Language::TypeScript
            | Language::JavaScript
            | Language::Go
            | Language::Rust
            | Language::Java
            | Language::C
            | Language::Cpp
            | Language::Ruby
            | Language::Kotlin
            | Language::Swift
            | Language::CSharp
            | Language::Scala
            | Language::Php
            | Language::Lua
            | Language::Luau
            | Language::Elixir
            | Language::Ocaml
    );

    // FM-10 fix: borrow var_types immutably alongside mutable calls borrow
    let (funcs, classes, var_types, calls) = (
        &file_ir.funcs,
        &file_ir.classes,
        &file_ir.var_types,
        &mut file_ir.calls,
    );

    for (caller_name, call_sites) in calls.iter_mut() {
        for call_site in call_sites.iter_mut() {
            if !matches!(call_site.call_type, CallType::Method | CallType::Attr) {
                continue;
            }
            if call_site.receiver_type.is_some() {
                continue;
            }
            let receiver = match call_site.receiver.as_deref() {
                Some(r) => r,
                None => continue,
            };
            let line = match call_site.line {
                Some(l) => l,
                None => continue,
            };

            let receiver_key = receiver.trim();
            let receiver_simple = if receiver_key == "super"
                || receiver_key.starts_with("super(")
                || receiver_key.starts_with("super<")
            {
                "super"
            } else {
                receiver_key
            };

            let enclosing_class = enclosing_class_for_call(funcs, call_site);
            let base_class = enclosing_class
                .as_deref()
                .and_then(|class_name| first_base_for_class(classes, class_name));

            if supports_type_resolution {
                let (resolved, confidence) = resolve_receiver_type(
                    language,
                    source,
                    line,
                    receiver_key,
                    enclosing_class.as_deref(),
                );
                if resolved.is_some() && confidence != crate::types::Confidence::Low {
                    call_site.receiver_type = resolved;
                    continue;
                }
            }

            // VarType-driven injection: look up receiver in file_ir.var_types
            // This fills receiver_type from constructor assignments, type annotations,
            // and parameter annotations extracted by the language handler.
            // Implements "last assignment wins" with scoped priority over module-level.
            if call_site.receiver_type.is_none() && !var_types.is_empty() {
                // PHP receivers have `$` prefix (e.g. "$animal") but VarTypes store without it
                let vartype_key = receiver_key.strip_prefix('$').unwrap_or(receiver_key);
                if let Some(type_name) =
                    find_best_vartype(var_types, vartype_key, caller_name, line)
                {
                    call_site.receiver_type = Some(type_name);
                    continue;
                }
            }

            if call_site.receiver_type.is_some() {
                continue;
            }

            match receiver_simple {
                "self" | "cls" | "this" | "Self" => {
                    if let Some(class_name) = enclosing_class {
                        call_site.receiver_type = Some(class_name);
                    }
                }
                "super" | "base" => {
                    if let Some(base_name) = base_class {
                        call_site.receiver_type = Some(base_name);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Find the best matching VarType for a given receiver name and call context.
///
/// Implements:
/// - Scoped matches (same function) take priority over module-level (None scope)
/// - Among matches in the same priority tier, "last assignment wins" (highest line <= call_line)
///
/// Returns the `type_name` of the best match, or None.
fn find_best_vartype(
    var_types: &[VarType],
    receiver_name: &str,
    caller_name: &str,
    call_line: u32,
) -> Option<String> {
    let mut best_scoped: Option<&VarType> = None;
    let mut best_module: Option<&VarType> = None;

    for vt in var_types {
        if vt.var_name != receiver_name {
            continue;
        }
        if vt.line > call_line {
            continue;
        }

        match &vt.scope {
            Some(scope) if scope == caller_name => {
                // Scoped match: prefer latest line
                if best_scoped.is_none_or(|prev| vt.line > prev.line) {
                    best_scoped = Some(vt);
                }
            }
            None => {
                // Module-level match: prefer latest line
                if best_module.is_none_or(|prev| vt.line > prev.line) {
                    best_module = Some(vt);
                }
            }
            _ => {
                // Different scope, skip
            }
        }
    }

    // Scoped matches take priority over module-level
    best_scoped
        .or(best_module)
        .map(|vt| vt.type_name.clone())
}

/// Resolve the best caller name for a call site, qualifying methods with class names when possible.
pub(crate) fn resolve_caller_name(file_ir: &FileIR, call_site: &CallSite) -> String {
    let line = match call_site.line {
        Some(l) => l,
        None => return call_site.caller.clone(),
    };

    let mut best: Option<&FuncDef> = None;
    let mut best_span: u32 = u32::MAX;

    for func in &file_ir.funcs {
        if line < func.line || line > func.end_line {
            continue;
        }
        let span = func.end_line.saturating_sub(func.line);
        if span <= best_span {
            best_span = span;
            best = Some(func);
        }
    }

    if let Some(func) = best {
        if func.is_method {
            if let Some(ref class_name) = func.class_name {
                return format!("{}.{}", class_name, func.name);
            }
        }
        return func.name.clone();
    }

    call_site.caller.clone()
}

fn resolve_reexported_name(
    module_path: &str,
    name: &str,
    tracer: &mut ReExportTracer<'_>,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    if language != "python" {
        return None;
    }

    let traced = tracer.trace(module_path, name, DEFAULT_MAX_DEPTH)?;
    let traced_module = path_to_module(&traced.definition_file, language);

    if let Some(entry) = func_index.get(&traced_module, &traced.qualified_name) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: traced.qualified_name.clone(),
            line: Some(entry.line),
            is_method: entry.is_method,
            class_name: entry.class_name.clone(),
        });
    }

    if let Some(class_entry) = class_index.get(&traced.qualified_name) {
        if let Some(ctor_target) = resolve_constructor_target(
            &traced.qualified_name,
            class_entry,
            func_index,
            language,
        ) {
            return Some(ctor_target);
        }
        return Some(ResolvedTarget {
            file: class_entry.file_path.clone(),
            name: traced.qualified_name.clone(),
            line: Some(class_entry.line),
            is_method: false,
            class_name: None,
        });
    }

    None
}

fn resolve_reexported_receiver_target(
    module_path: &str,
    receiver_name: &str,
    method_name: &str,
    tracer: &mut ReExportTracer<'_>,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    if language != "python" {
        return None;
    }

    let traced = tracer.trace(module_path, receiver_name, DEFAULT_MAX_DEPTH)?;
    let traced_module = path_to_module(&traced.definition_file, language);

    if let Some(entry) = func_index.get(&traced_module, method_name) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: method_name.to_string(),
            line: Some(entry.line),
            is_method: entry.is_method,
            class_name: entry.class_name.clone(),
        });
    }

    if let Some(class_entry) = class_index.get(method_name) {
        return Some(ResolvedTarget {
            file: class_entry.file_path.clone(),
            name: method_name.to_string(),
            line: Some(class_entry.line),
            is_method: false,
            class_name: None,
        });
    }

    None
}

/// Resolve a call site to its target definition.
///
/// This function implements the resolution priority from spec section 14.6:
/// 1. Intra-file calls (local functions/classes)
/// 2. Direct calls via import map
/// 3. Attribute calls (module.func or obj.method)
/// 4. Method calls with receiver type
///
/// # Mitigations Implemented
/// - M2.5: TYPE_CHECKING imports tagged with is_type_only (not implemented in this call,
///   but import_map should filter them if config.runtime_only is set)
/// - M2.4: Dynamic imports (__import__, importlib) - returns None with warning
///
/// # Arguments
/// * `target` - The call target name (e.g., "foo", "bar" from "obj.bar")
/// * `call_type` - Classification of the call
/// * `context` - Shared resolution indexes and state
///
/// # Returns
/// * `Some(ResolvedTarget)` if the call can be resolved
/// * `None` if the call is external, stdlib, or cannot be resolved
///
/// # Example
/// ```rust,ignore
/// // Direct call: foo()
/// let mut context = ResolutionContext {
///     import_map: &import_map,
///     module_imports: &module_imports,
///     func_index: &func_index,
///     class_index: &class_index,
///     reexport_tracer: &mut reexport_tracer,
///     current_file: Path::new("main.py"),
///     root: Path::new("/project"),
///     language: "python",
/// };
/// let target = resolve_call(
///     "foo",
///     &CallType::Direct,
///     &mut context,
/// );
/// ```
pub fn resolve_call(
    target: &str,
    call_type: &CallType,
    context: &mut ResolutionContext<'_, '_>,
) -> Option<ResolvedTarget> {
    let import_map = context.import_map;
    let func_index = context.func_index;
    let class_index = context.class_index;
    let current_file = context.current_file;
    let language = context.language;

    // M2.4: Check for dynamic import patterns - these cannot be resolved
    if target.contains("__import__") || target.contains("importlib") {
        // Dynamic import detected - log warning and return None
        return None;
    }
    if matches!(language, "javascript" | "js" | "typescript" | "tsx") && target == "import" {
        return None;
    }

    // Convert current file to module path (language-aware)
    let current_module = path_to_module(current_file, language);

    match call_type {
        CallType::Intra => {
            // Intra-file call: target is in the same file
            // Look up in func_index using current module
            if let Some(entry) = func_index.get(&current_module, target) {
                return Some(ResolvedTarget {
                    file: entry.file_path.clone(),
                    name: target.to_string(),
                    line: Some(entry.line),
                    is_method: entry.is_method,
                    class_name: entry.class_name.clone(),
                });
            }

            // Also check if it's a class name (calling constructor)
            if let Some(class_entry) = class_index.get(target) {
                // Constructor call - resolve to the actual constructor method (__init__, initialize, etc.)
                if let Some(ctor) = resolve_constructor_target(target, class_entry, func_index, language) {
                    return Some(ctor);
                }
                // Fallback: resolve to the class itself
                return Some(ResolvedTarget {
                    file: class_entry.file_path.clone(),
                    name: target.to_string(),
                    line: Some(class_entry.line),
                    is_method: false,
                    class_name: None,
                });
            }

            None
        }

        CallType::Direct => {
            // Direct call to an imported or local name

            // First, check if it's a local function
            if let Some(entry) = func_index.get(&current_module, target) {
                return Some(ResolvedTarget {
                    file: entry.file_path.clone(),
                    name: target.to_string(),
                    line: Some(entry.line),
                    is_method: entry.is_method,
                    class_name: entry.class_name.clone(),
                });
            }

            // Check the import map for "from X import Y" style imports
            if let Some((module_path, original_name)) = import_map.get(target) {
                // BUG FIX 3: Try simple module name first, fallback to full path (CROSSFILE_SPEC.md Section 3.2.1)
                // When resolving `process()` with import_map["process"] = ("pkg.helper", "process"),
                // we need to check both ("helper", "process") and ("pkg.helper", "process").
                let simple_module = module_path.split('.').next_back().unwrap_or(module_path);

                // Normalize JS/TS module paths: strip .js/.ts extensions from import paths
                // Import strings often include .js extension (TS ESM convention)
                let stripped_ext = module_path
                    .strip_suffix(".js")
                    .or_else(|| module_path.strip_suffix(".jsx"))
                    .or_else(|| module_path.strip_suffix(".ts"))
                    .or_else(|| module_path.strip_suffix(".tsx"))
                    .or_else(|| module_path.strip_suffix(".mjs"))
                    .unwrap_or(module_path);

                // For TS/JS: func_index now uses ./prefix keys (matching ModuleIndex),
                // so try stripped_ext directly first (preserves ./ prefix).
                // For Python: try bare module (no ./ prefix) as fallback.
                let mut bare = stripped_ext;
                // Strip all leading ../ prefixes (handles ../../foo -> foo)
                while let Some(rest) = bare.strip_prefix("../") {
                    bare = rest;
                }
                // Also strip single ./ prefix
                let bare_module = bare.strip_prefix("./").unwrap_or(bare);

                // Try extension-stripped path first (preserves ./ for TS/JS)
                if stripped_ext != bare_module {
                    if let Some(entry) = func_index.get(stripped_ext, original_name) {
                        return Some(ResolvedTarget {
                            file: entry.file_path.clone(),
                            name: original_name.clone(),
                            line: Some(entry.line),
                            is_method: entry.is_method,
                            class_name: entry.class_name.clone(),
                        });
                    }
                }

                // Try bare module name (without ./ prefix) -- matches Python-style keys
                if let Some(entry) = func_index.get(bare_module, original_name) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: original_name.clone(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: entry.class_name.clone(),
                    });
                }

                // Try simple module name (last dot component)
                if let Some(entry) = func_index.get(simple_module, original_name) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: original_name.clone(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: entry.class_name.clone(),
                    });
                }
                // Fallback to full module path
                if let Some(entry) = func_index.get(module_path, original_name) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: original_name.clone(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: entry.class_name.clone(),
                    });
                }

                // It might be a class (constructor call via import)
                if let Some(class_entry) = class_index.get(original_name) {
                    // Try resolving to actual constructor method (__init__, initialize, etc.)
                    if let Some(ctor) = resolve_constructor_target(original_name, class_entry, func_index, language) {
                        return Some(ctor);
                    }
                    return Some(ResolvedTarget {
                        file: class_entry.file_path.clone(),
                        name: original_name.clone(),
                        line: Some(class_entry.line),
                        is_method: false,
                        class_name: None,
                    });
                }

                if let Some(resolved) = resolve_reexported_name(
                    module_path,
                    original_name,
                    context.reexport_tracer,
                    func_index,
                    class_index,
                    language,
                ) {
                    return Some(resolved);
                }
            }

            // Check if target is a class name for constructor call
            if let Some(class_entry) = class_index.get(target) {
                if let Some(ctor_target) =
                    resolve_constructor_target(target, class_entry, func_index, language)
                {
                    return Some(ctor_target);
                }

                return Some(ResolvedTarget {
                    file: class_entry.file_path.clone(),
                    name: target.to_string(),
                    line: Some(class_entry.line),
                    is_method: false,
                    class_name: None,
                });
            }

            // Not found - likely external/stdlib
            None
        }

        CallType::Attr => {
            // Attribute call like module.func() or obj.method()
            // The "receiver" in CallSite tells us what's before the dot
            // The "target" is the attribute name

            // This is handled in resolve_call_with_receiver since we need receiver info
            // If we get here without receiver context, we can't resolve
            None
        }

        CallType::Method => {
            // Method call with receiver like user.save()
            // Similar to Attr - needs receiver info
            // This is handled in resolve_call_with_receiver
            None
        }

        CallType::Ref => {
            // Function reference without call (higher-order)
            // Resolve like Direct
            if let Some((module_path, original_name)) = import_map.get(target) {
                if let Some(entry) = func_index.get(module_path, original_name) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: original_name.clone(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: entry.class_name.clone(),
                    });
                }
                if let Some(resolved) = resolve_reexported_name(
                    module_path,
                    original_name,
                    context.reexport_tracer,
                    func_index,
                    class_index,
                    language,
                ) {
                    return Some(resolved);
                }
            }

            // Check local
            if let Some(entry) = func_index.get(&current_module, target) {
                return Some(ResolvedTarget {
                    file: entry.file_path.clone(),
                    name: target.to_string(),
                    line: Some(entry.line),
                    is_method: entry.is_method,
                    class_name: entry.class_name.clone(),
                });
            }

            None
        }

        CallType::Static => {
            // Static method call: ClassName::staticMethod() (PHP-style)
            // The target contains "ClassName::methodName"
            if let Some(sep_pos) = target.find("::") {
                let class_name = &target[..sep_pos];
                let method_name = &target[sep_pos + 2..];

                if let Some(resolved) = resolve_call_with_receiver(
                    target,
                    class_name,
                    None,
                    call_type,
                    context,
                ) {
                    return Some(resolved);
                }

                if let Some(entry) = func_index.get(&current_module, target) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: target.to_string(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: entry.class_name.clone(),
                    });
                }

                let qualified_dot = format!("{}.{}", class_name, method_name);
                if let Some(entry) = func_index.get(&current_module, &qualified_dot) {
                    return Some(ResolvedTarget {
                        file: entry.file_path.clone(),
                        name: method_name.to_string(),
                        line: Some(entry.line),
                        is_method: entry.is_method,
                        class_name: Some(class_name.to_string()),
                    });
                }

                if let Some(resolved) = resolve_method_in_class(
                    class_name,
                    method_name,
                    class_index,
                    func_index,
                    language,
                )
                .or_else(|| {
                    resolve_method_in_bases(class_name, method_name, class_index, func_index, language)
                }) {
                    return Some(resolved);
                }
            }
            None
        }
    }
}

/// Resolve a method lookup in a specific class via class_index and func_index.
pub(crate) fn resolve_method_in_class(
    class_name: &str,
    method_name: &str,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    let class_entry = class_index.get(class_name)?;
    let module = path_to_module(&class_entry.file_path, language);
    let qualified = format!("{}.{}", class_name, method_name);

    if let Some(entry) = func_index.get(&module, &qualified) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: method_name.to_string(),
            line: Some(entry.line),
            is_method: true,
            class_name: Some(class_name.to_string()),
        });
    }

    if class_entry.methods.contains(&method_name.to_string()) {
        return Some(ResolvedTarget {
            file: class_entry.file_path.clone(),
            name: method_name.to_string(),
            line: Some(class_entry.line),
            is_method: true,
            class_name: Some(class_name.to_string()),
        });
    }

    None
}

/// Resolve a method by traversing base classes via BFS.
pub(crate) fn resolve_method_in_bases(
    class_name: &str,
    method_name: &str,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Some(entry) = class_index.get(class_name) {
        for base in &entry.bases {
            queue.push_back(base.clone());
        }
    }

    while let Some(base) = queue.pop_front() {
        if !seen.insert(base.clone()) {
            continue;
        }
        if let Some(resolved) =
            resolve_method_in_class(&base, method_name, class_index, func_index, language)
        {
            return Some(resolved);
        }
        if let Some(entry) = class_index.get(&base) {
            for parent in &entry.bases {
                if !seen.contains(parent) {
                    queue.push_back(parent.clone());
                }
            }
        }
    }

    None
}

/// Check if a type name is a known Python/Ruby/etc stdlib or builtin type.
///
/// These types' methods should never resolve to project-internal classes via
/// the fuzzy fallback strategies (7, 8). For example, `OrderedDict.items()`
/// should not resolve to `RequestsCookieJar.items()`.
fn is_stdlib_type(name: &str) -> bool {
    matches!(
        name,
        // Python builtins
        "dict" | "list" | "set" | "tuple" | "frozenset" | "str" | "bytes"
        | "bytearray" | "int" | "float" | "bool" | "complex" | "object"
        | "type" | "range" | "memoryview" | "slice" | "None" | "NoneType"
        // Python collections
        | "OrderedDict" | "defaultdict" | "deque" | "Counter" | "ChainMap"
        | "namedtuple" | "UserDict" | "UserList" | "UserString"
        // Python io
        | "StringIO" | "BytesIO" | "TextIOWrapper" | "BufferedReader"
        // Python pathlib
        | "Path" | "PurePath" | "PosixPath" | "WindowsPath"
        // Python typing module aliases
        | "Dict" | "List" | "Set" | "Tuple" | "FrozenSet" | "Optional"
        | "Union" | "Any" | "Callable" | "Type" | "Sequence" | "Mapping"
        | "MutableMapping" | "MutableSequence" | "MutableSet" | "Iterator"
        | "Iterable" | "Generator" | "Coroutine" | "AsyncGenerator"
        // Ruby builtins
        | "Array" | "Hash" | "String" | "Integer" | "Float" | "Symbol"
        | "Regexp" | "Proc" | "Lambda" | "IO" | "File" | "Dir"
    )
}

/// Check if a method name is commonly defined on builtin types (dict, list, str, etc.).
/// When the receiver has no inferred type, these names are too ambiguous to resolve
/// via class scanning -- they'd match project classes that happen to define the same method.
fn is_builtin_method_name(name: &str) -> bool {
    matches!(
        name,
        // dict methods
        "items" | "values" | "keys" | "get" | "pop" | "update" | "setdefault"
        | "clear" | "copy" | "popitem"
        // list methods
        | "append" | "extend" | "insert" | "remove" | "sort" | "reverse" | "count" | "index"
        // set methods
        | "add" | "discard" | "union" | "intersection" | "difference"
        // str methods
        | "strip" | "split" | "join" | "replace" | "format" | "encode" | "decode"
        | "startswith" | "endswith" | "lower" | "upper" | "find"
        // io methods
        | "close" | "read" | "write" | "flush" | "seek" | "tell" | "readline"
        // Go serialization methods (safe to block -- rarely project method names)
        | "MarshalJSON" | "UnmarshalJSON" | "MarshalText" | "UnmarshalText"
        // very common names that collide across unrelated types
        | "invoke" | "call" | "run" | "execute" | "send" | "receive"
        | "start" | "stop" | "reset" | "setup" | "teardown"
    )
}

/// Check if `candidate_class` is in the inheritance chain of `receiver_class`.
///
/// Returns true if:
/// - candidate_class == receiver_class (same class)
/// - candidate_class is a base (parent) of receiver_class (direct or transitive)
/// - receiver_class is a base of candidate_class (child calling parent's method via self)
///
/// Uses BFS to traverse the inheritance tree up to a bounded depth.
fn is_in_inheritance_chain(
    receiver_class: &str,
    candidate_class: &str,
    class_index: &ClassIndex,
) -> bool {
    if receiver_class == candidate_class {
        return true;
    }

    // Check if candidate_class is an ancestor of receiver_class (self.method() calling parent method)
    {
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut seen: HashSet<String> = HashSet::new();

        if let Some(entry) = class_index.get(receiver_class) {
            for base in &entry.bases {
                queue.push_back(base.clone());
            }
        }

        while let Some(base) = queue.pop_front() {
            if !seen.insert(base.clone()) {
                continue;
            }
            if base == candidate_class {
                return true;
            }
            if let Some(entry) = class_index.get(&base) {
                for parent in &entry.bases {
                    if !seen.contains(parent) {
                        queue.push_back(parent.clone());
                    }
                }
            }
        }
    }

    // Check if receiver_class is an ancestor of candidate_class (less common, but possible)
    {
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut seen: HashSet<String> = HashSet::new();

        if let Some(entry) = class_index.get(candidate_class) {
            for base in &entry.bases {
                queue.push_back(base.clone());
            }
        }

        while let Some(base) = queue.pop_front() {
            if !seen.insert(base.clone()) {
                continue;
            }
            if base == receiver_class {
                return true;
            }
            if let Some(entry) = class_index.get(&base) {
                for parent in &entry.bases {
                    if !seen.contains(parent) {
                        queue.push_back(parent.clone());
                    }
                }
            }
        }
    }

    false
}

/// Resolve a call that has receiver information (Method or Attr calls).
///
/// This function handles calls like `receiver.target()` where we need to determine
/// whether `receiver` is a module (import) or an object instance.
///
/// # Arguments
/// * `target` - The method/attribute being called
/// * `receiver` - The receiver (what's before the dot)
/// * `receiver_type` - Inferred type of receiver, if known
/// * `call_type` - Either Method or Attr
/// * `context` - Shared resolution indexes and state
///
/// # Resolution Strategy
/// 1. If receiver is a known module import -> resolve as module.func
/// 2. If receiver_type is known -> resolve as Type.method
/// 3. If receiver is a class name -> resolve as static call
/// 4. Search class index for method name matches
pub fn resolve_call_with_receiver(
    target: &str,
    receiver: &str,
    receiver_type: Option<&str>,
    _call_type: &CallType,
    context: &mut ResolutionContext<'_, '_>,
) -> Option<ResolvedTarget> {
    let import_map = context.import_map;
    let module_imports = context.module_imports;
    let func_index = context.func_index;
    let class_index = context.class_index;
    let current_file = context.current_file;
    let language = context.language;

    let current_module = path_to_module(current_file, language);
    let bare_target = normalize_receiver_target(target, receiver);

    if let Some(resolved) = resolve_with_receiver_type(
        receiver_type,
        bare_target,
        class_index,
        func_index,
        language,
    ) {
        return Some(resolved);
    }

    if let Some(resolved) = resolve_self_receiver_in_current_file(
        receiver,
        bare_target,
        &current_module,
        func_index,
        class_index,
    ) {
        return Some(resolved);
    }

    let mut receiver_context = ReceiverLookupContext {
        func_index,
        class_index,
        reexport_tracer: context.reexport_tracer,
        language,
    };

    if let Some(resolved) = resolve_module_import_receiver(
        target,
        receiver,
        bare_target,
        module_imports,
        &mut receiver_context,
    ) {
        return Some(resolved);
    }

    if let Some(resolved) = resolve_import_map_receiver(
        target,
        receiver,
        bare_target,
        import_map,
        &mut receiver_context,
    ) {
        return Some(resolved);
    }

    if let Some(resolved) =
        resolve_method_in_class_or_bases(receiver, bare_target, class_index, func_index, language)
    {
        return Some(resolved);
    }

    if let Some(resolved) =
        resolve_local_qualified_receiver(receiver, bare_target, &current_module, func_index)
    {
        return Some(resolved);
    }

    if let Some(resolved) =
        resolve_capitalized_receiver(receiver, bare_target, class_index, func_index, language)
    {
        return Some(resolved);
    }

    let type_filter = receiver_type_filter(receiver_type, receiver, class_index);
    if let Some(resolved) = resolve_local_fuzzy_match(
        bare_target,
        type_filter,
        func_index,
        class_index,
        current_file,
    ) {
        return Some(resolved);
    }
    if let Some(resolved) = resolve_global_fuzzy_match(bare_target, type_filter, func_index, class_index) {
        return Some(resolved);
    }

    resolve_type_aware_fallback(receiver_type, bare_target, func_index, class_index)
}

fn normalize_receiver_target<'a>(target: &'a str, receiver: &str) -> &'a str {
    target
        .strip_prefix(&format!("{}.", receiver))
        .or_else(|| target.strip_prefix(&format!("{}::", receiver)))
        .or_else(|| target.strip_prefix(&format!("{}->", receiver)))
        .unwrap_or(target)
}

fn resolve_with_receiver_type(
    receiver_type: Option<&str>,
    bare_target: &str,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    let type_name = receiver_type?;
    resolve_method_in_class_or_bases(type_name, bare_target, class_index, func_index, language)
}

fn resolve_self_receiver_in_current_file(
    receiver: &str,
    bare_target: &str,
    current_module: &str,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
) -> Option<ResolvedTarget> {
    if !matches!(receiver, "self" | "cls" | "this" | "Self") {
        return None;
    }
    if let Some(entry) = func_index.get(current_module, bare_target) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: bare_target.to_string(),
            line: Some(entry.line),
            is_method: true,
            class_name: entry.class_name.clone(),
        });
    }
    let class_entry = class_index.get(bare_target)?;
    Some(ResolvedTarget {
        file: class_entry.file_path.clone(),
        name: bare_target.to_string(),
        line: Some(class_entry.line),
        is_method: false,
        class_name: Some(bare_target.to_string()),
    })
}

struct ReceiverLookupContext<'a, 'b> {
    func_index: &'a FuncIndex,
    class_index: &'a ClassIndex,
    reexport_tracer: &'a mut ReExportTracer<'b>,
    language: &'a str,
}

fn resolve_module_import_receiver(
    target: &str,
    receiver: &str,
    bare_target: &str,
    module_imports: &ModuleImports,
    context: &mut ReceiverLookupContext<'_, '_>,
) -> Option<ResolvedTarget> {
    let module_path = module_imports.get(receiver)?;
    let simple_module = module_path.split('.').next_back().unwrap_or(module_path);

    if let Some(entry) = context.func_index.get(module_path, bare_target) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: bare_target.to_string(),
            line: Some(entry.line),
            is_method: entry.is_method,
            class_name: entry.class_name.clone(),
        });
    }
    if simple_module != module_path.as_str() {
        if let Some(entry) = context.func_index.get(simple_module, bare_target) {
            return Some(ResolvedTarget {
                file: entry.file_path.clone(),
                name: bare_target.to_string(),
                line: Some(entry.line),
                is_method: entry.is_method,
                class_name: entry.class_name.clone(),
            });
        }
    }
    if bare_target != target {
        if let Some(entry) = context.func_index.get(module_path, target) {
            return Some(ResolvedTarget {
                file: entry.file_path.clone(),
                name: target.to_string(),
                line: Some(entry.line),
                is_method: entry.is_method,
                class_name: entry.class_name.clone(),
            });
        }
    }

    resolve_reexported_name(
        module_path,
        bare_target,
        context.reexport_tracer,
        context.func_index,
        context.class_index,
        context.language,
    )
}

fn resolve_import_map_receiver(
    target: &str,
    receiver: &str,
    bare_target: &str,
    import_map: &ImportMap,
    context: &mut ReceiverLookupContext<'_, '_>,
) -> Option<ResolvedTarget> {
    let (module_path, original_name) = import_map.get(receiver)?;
    if let Some(resolved) = resolve_method_in_class_or_bases(
        original_name,
        bare_target,
        context.class_index,
        context.func_index,
        context.language,
    ) {
        return Some(resolved);
    }

    if let Some(entry) = context.func_index.get(module_path, bare_target) {
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: bare_target.to_string(),
            line: Some(entry.line),
            is_method: entry.is_method,
            class_name: entry.class_name.clone(),
        });
    }
    if bare_target != target {
        if let Some(entry) = context.func_index.get(module_path, target) {
            return Some(ResolvedTarget {
                file: entry.file_path.clone(),
                name: target.to_string(),
                line: Some(entry.line),
                is_method: entry.is_method,
                class_name: entry.class_name.clone(),
            });
        }
    }

    resolve_reexported_receiver_target(
        module_path,
        original_name,
        bare_target,
        context.reexport_tracer,
        context.func_index,
        context.class_index,
        context.language,
    )
}

fn resolve_method_in_class_or_bases(
    class_name: &str,
    method_name: &str,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    resolve_method_in_class(class_name, method_name, class_index, func_index, language).or_else(|| {
        resolve_method_in_bases(class_name, method_name, class_index, func_index, language)
    })
}

fn resolve_local_qualified_receiver(
    receiver: &str,
    bare_target: &str,
    current_module: &str,
    func_index: &FuncIndex,
) -> Option<ResolvedTarget> {
    let qualified = format!("{}.{}", receiver, bare_target);
    let entry = func_index.get(current_module, &qualified)?;
    Some(ResolvedTarget {
        file: entry.file_path.clone(),
        name: bare_target.to_string(),
        line: Some(entry.line),
        is_method: entry.is_method,
        class_name: entry.class_name.clone(),
    })
}

fn resolve_capitalized_receiver(
    receiver: &str,
    bare_target: &str,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    let capitalized = capitalize_first(receiver);
    if capitalized == receiver {
        return None;
    }
    resolve_method_in_class_or_bases(
        &capitalized,
        bare_target,
        class_index,
        func_index,
        language,
    )
}

fn receiver_type_filter<'a>(
    receiver_type: Option<&'a str>,
    receiver: &str,
    class_index: &ClassIndex,
) -> Option<&'a str> {
    receiver_type.filter(|type_name| {
        if class_index.get(type_name).is_some() {
            return true;
        }
        if matches!(receiver, "self" | "cls" | "this" | "Self") {
            return true;
        }
        is_stdlib_type(type_name)
    })
}

fn resolve_local_fuzzy_match(
    bare_target: &str,
    type_filter: Option<&str>,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
    current_file: &Path,
) -> Option<ResolvedTarget> {
    if type_filter.is_none() && is_builtin_method_name(bare_target) {
        return None;
    }

    let local_matches: Vec<_> = func_index
        .iter()
        .filter(|((_module, func_name), entry)| {
            if *func_name != bare_target || entry.file_path != current_file {
                return false;
            }
            if let Some(type_name) = type_filter {
                if let Some(ref candidate_class) = entry.class_name {
                    return is_in_inheritance_chain(type_name, candidate_class, class_index);
                }
            }
            true
        })
        .collect();

    if local_matches.len() == 1 || (type_filter.is_some() && !local_matches.is_empty()) {
        let (_, entry) = local_matches[0];
        return Some(ResolvedTarget {
            file: entry.file_path.clone(),
            name: bare_target.to_string(),
            line: Some(entry.line),
            is_method: entry.is_method,
            class_name: entry.class_name.clone(),
        });
    }
    None
}

fn resolve_global_fuzzy_match(
    bare_target: &str,
    type_filter: Option<&str>,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
) -> Option<ResolvedTarget> {
    if type_filter.is_none() && is_builtin_method_name(bare_target) {
        return None;
    }

    let mut candidates: Vec<_> = func_index
        .find_by_name(bare_target)
        .filter(|e| e.is_method)
        .collect();
    if let Some(type_name) = type_filter {
        candidates.retain(|e| match &e.class_name {
            Some(c) => is_in_inheritance_chain(type_name, c, class_index),
            None => false,
        });
    }
    if candidates.len() != 1 {
        return None;
    }
    let entry = candidates[0];
    Some(ResolvedTarget {
        file: entry.file_path.clone(),
        name: bare_target.to_string(),
        line: Some(entry.line),
        is_method: true,
        class_name: entry.class_name.clone(),
    })
}

fn resolve_type_aware_fallback(
    receiver_type: Option<&str>,
    bare_target: &str,
    func_index: &FuncIndex,
    class_index: &ClassIndex,
) -> Option<ResolvedTarget> {
    let type_name = receiver_type?;
    if let Some(class_entry) = class_index.get(type_name) {
        if class_entry.methods.contains(&bare_target.to_string()) {
            return Some(ResolvedTarget {
                file: class_entry.file_path.clone(),
                name: bare_target.to_string(),
                line: Some(class_entry.line),
                is_method: true,
                class_name: Some(type_name.to_string()),
            });
        }
        for base in &class_entry.bases {
            if let Some(base_entry) = class_index.get(base.as_str()) {
                if base_entry.methods.contains(&bare_target.to_string()) {
                    return Some(ResolvedTarget {
                        file: base_entry.file_path.clone(),
                        name: bare_target.to_string(),
                        line: Some(base_entry.line),
                        is_method: true,
                        class_name: Some(base.to_string()),
                    });
                }
            }
        }
    }

    for ((_module, func_name), entry) in func_index.iter() {
        if func_name == bare_target && entry.class_name.as_deref() == Some(type_name) {
            return Some(ResolvedTarget {
                file: entry.file_path.clone(),
                name: bare_target.to_string(),
                line: Some(entry.line),
                is_method: true,
                class_name: Some(type_name.to_string()),
            });
        }
    }
    None
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    // From new sibling modules:
    use super::super::types::{FuncIndex, ClassIndex, FuncEntry, ClassEntry};
    use super::super::imports::{ImportMap, ModuleImports, augment_go_module_imports};
    use super::super::module_path::path_to_module;
    // From existing sibling modules:
    use crate::callgraph::cross_file_types::{
        CallType, ImportDef,
    };
    use crate::callgraph::import_resolver::ReExportTracer;
    use crate::callgraph::module_index::ModuleIndex;
    
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    macro_rules! resolve_call {
        (
            $target:expr,
            $call_type:expr,
            $import_map:expr,
            $module_imports:expr,
            $func_index:expr,
            $class_index:expr,
            $reexport_tracer:expr,
            $current_file:expr,
            $root:expr,
            $language:expr $(,)?
        ) => {{
            let mut context = ResolutionContext {
                import_map: $import_map,
                module_imports: $module_imports,
                func_index: $func_index,
                class_index: $class_index,
                reexport_tracer: $reexport_tracer,
                current_file: $current_file,
                root: $root,
                language: $language,
            };
            super::resolve_call($target, $call_type, &mut context)
        }};
    }

    macro_rules! resolve_call_with_receiver {
        (
            $target:expr,
            $receiver:expr,
            $receiver_type:expr,
            $call_type:expr,
            $import_map:expr,
            $module_imports:expr,
            $func_index:expr,
            $class_index:expr,
            $reexport_tracer:expr,
            $current_file:expr,
            $root:expr,
            $language:expr $(,)?
        ) => {{
            let mut context = ResolutionContext {
                import_map: $import_map,
                module_imports: $module_imports,
                func_index: $func_index,
                class_index: $class_index,
                reexport_tracer: $reexport_tracer,
                current_file: $current_file,
                root: $root,
                language: $language,
            };
            super::resolve_call_with_receiver(
                $target,
                $receiver,
                $receiver_type,
                $call_type,
                &mut context,
            )
        }};
    }

    /// Test: ResolvedTarget::function creates a function target
    #[test]
    fn test_resolved_target_function() {
        let target = ResolvedTarget::function(
            PathBuf::from("helper.py"),
            "process",
            Some(10),
        );

        assert_eq!(target.file, PathBuf::from("helper.py"));
        assert_eq!(target.name, "process");
        assert_eq!(target.line, Some(10));
        assert!(!target.is_method);
        assert!(target.class_name.is_none());
        assert_eq!(target.qualified_name(), "process");
    }

    /// Test: ResolvedTarget::method creates a method target
    #[test]
    fn test_resolved_target_method() {
        let target = ResolvedTarget::method(
            PathBuf::from("models.py"),
            "save",
            "User",
            Some(42),
        );

        assert_eq!(target.file, PathBuf::from("models.py"));
        assert_eq!(target.name, "save");
        assert_eq!(target.line, Some(42));
        assert!(target.is_method);
        assert_eq!(target.class_name, Some("User".to_string()));
        assert_eq!(target.qualified_name(), "User.save");
    }

    /// Test: resolve_call for intra-file calls
    #[test]
    fn test_resolve_call_intra() {
        // Setup: Create a func_index with a local function
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "main",
            "helper",
            FuncEntry::function(PathBuf::from("main.py"), 10, 15),
        );

        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call resolve_call for an intra-file call
        let resolved = resolve_call!(
            "helper",
            &CallType::Intra,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve intra-file call");
        let target = resolved.unwrap();
        assert_eq!(target.file, PathBuf::from("main.py"));
        assert_eq!(target.name, "helper");
        assert!(!target.is_method);
    }

    /// Test: resolve_call for direct calls via import map
    #[test]
    fn test_resolve_call_direct_import() {
        // Setup: Function is in helper module, imported as 'process'
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "helper",
            "process",
            FuncEntry::function(PathBuf::from("helper.py"), 5, 10),
        );

        let mut import_map = ImportMap::new();
        import_map.insert("process".to_string(), ("helper".to_string(), "process".to_string()));

        let module_imports = ModuleImports::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call resolve_call for a direct call to imported name
        let resolved = resolve_call!(
            "process",
            &CallType::Direct,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve direct call via import map");
        let target = resolved.unwrap();
        assert_eq!(target.file, PathBuf::from("helper.py"));
        assert_eq!(target.name, "process");
    }

    /// Test: resolve_call returns None for external/stdlib
    #[test]
    fn test_resolve_call_external() {
        let func_index = FuncIndex::new();
        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call to something not in project
        let resolved = resolve_call!(
            "json_loads",
            &CallType::Direct,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_none(), "External/stdlib calls should return None");
    }

    /// Test: resolve_call detects dynamic imports (M2.4)
    #[test]
    fn test_resolve_call_dynamic_import() {
        let func_index = FuncIndex::new();
        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Dynamic import pattern
        let resolved = resolve_call!(
            "__import__",
            &CallType::Direct,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_none(), "Dynamic imports should return None");
    }

    /// Test: resolve_call_with_receiver for module.func pattern
    #[test]
    fn test_resolve_call_module_func() {
        // Setup: json module with loads function
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "json",
            "loads",
            FuncEntry::function(PathBuf::from("json.py"), 100, 120),
        );

        let import_map = ImportMap::new();
        let mut module_imports = ModuleImports::new();
        module_imports.insert("json".to_string(), "json".to_string());

        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: json.loads()
        let resolved = resolve_call_with_receiver!(
            "loads",
            "json",
            None,
            &CallType::Attr,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve module.func pattern");
        let target = resolved.unwrap();
        assert_eq!(target.name, "loads");
    }

    /// Test: resolve_call_with_receiver for method with known receiver type
    #[test]
    fn test_resolve_call_method_with_type() {
        // Setup: User class with save method
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "models",
            "User.save",
            FuncEntry::method(PathBuf::from("models.py"), 50, 60, "User".to_string()),
        );

        let mut class_index = ClassIndex::new();
        class_index.insert(
            "User",
            ClassEntry::new(
                PathBuf::from("models.py"),
                10,
                100,
                vec!["save".to_string(), "delete".to_string()],
                vec![],
            ),
        );

        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: user.save() where user: User
        let resolved = resolve_call_with_receiver!(
            "save",
            "user",
            Some("User"),
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve method with known type");
        let target = resolved.unwrap();
        assert_eq!(target.name, "save");
        assert!(target.is_method);
        assert_eq!(target.class_name, Some("User".to_string()));
    }

    /// Test: Ref call type resolution
    #[test]
    fn test_resolve_call_ref() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "utils",
            "transform",
            FuncEntry::function(PathBuf::from("utils.py"), 5, 15),
        );

        let mut import_map = ImportMap::new();
        import_map.insert("transform".to_string(), ("utils".to_string(), "transform".to_string()));

        let module_imports = ModuleImports::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Reference to transform function (passed as callback)
        let resolved = resolve_call!(
            "transform",
            &CallType::Ref,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve Ref call type");
        let target = resolved.unwrap();
        assert_eq!(target.name, "transform");
    }

    /// Test: Static call type resolution (PHP-style)
    #[test]
    fn test_resolve_call_static() {
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "models",
            "User.create",
            FuncEntry::method(PathBuf::from("models.py"), 25, 35, "User".to_string()),
        );

        let mut class_index = ClassIndex::new();
        class_index.insert(
            "User",
            ClassEntry::new(
                PathBuf::from("models.py"),
                5,
                50,
                vec!["create".to_string()],
                vec![],
            ),
        );

        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Static call: User::create()
        let resolved = resolve_call!(
            "User::create",
            &CallType::Static,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.php"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve static call");
        let target = resolved.unwrap();
        assert_eq!(target.name, "create");
        assert!(target.is_method);
        assert_eq!(target.class_name, Some("User".to_string()));
    }

    /// Test: Class constructor resolution (Direct call to class name)
    #[test]
    fn test_resolve_call_constructor() {
        let mut class_index = ClassIndex::new();
        class_index.insert(
            "MyClass",
            ClassEntry::new(
                PathBuf::from("classes.py"),
                10,
                50,
                vec!["__init__".to_string()],
                vec![],
            ),
        );

        let func_index = FuncIndex::new();
        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Direct call to class (constructor): MyClass()
        let resolved = resolve_call!(
            "MyClass",
            &CallType::Direct,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve constructor call");
        let target = resolved.unwrap();
        assert_eq!(target.file, PathBuf::from("classes.py"));
        assert_eq!(target.name, "__init__");
    }

    /// Test: Imported class used for method call resolution
    #[test]
    fn test_resolve_imported_class_method() {
        // Setup: User class imported and used as User.create()
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "models",
            "User.create",
            FuncEntry::method(PathBuf::from("models.py"), 30, 40, "User".to_string()),
        );

        let mut class_index = ClassIndex::new();
        class_index.insert(
            "User",
            ClassEntry::new(
                PathBuf::from("models.py"),
                10,
                50,
                vec!["create".to_string()],
                vec![],
            ),
        );

        let mut import_map = ImportMap::new();
        import_map.insert("User".to_string(), ("models".to_string(), "User".to_string()));

        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: User.create() (calling on the class itself)
        let resolved = resolve_call_with_receiver!(
            "create",
            "User",
            None,
            &CallType::Attr,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("/project"),
            "python",
        );

        assert!(resolved.is_some(), "Should resolve imported class method");
        let target = resolved.unwrap();
        assert_eq!(target.name, "create");
        assert!(target.is_method);
    }

    #[test]
    fn test_resolve_call_typescript_module_keys_match() {
        // End-to-end test: func_index keys (from path_to_module) must match
        // import_map keys (from ModuleIndex) for TypeScript
        let mut func_index = FuncIndex::new();
        let class_index = ClassIndex::new();

        // Simulate what build_indices_parallel does with the fixed path_to_module:
        // For a TS file "errors.ts", the module should be "./errors"
        let module = path_to_module(Path::new("errors.ts"), "typescript");
        func_index.insert(&module, "ZodError", FuncEntry::function(
            PathBuf::from("errors.ts"), 10, 20,
        ));

        // Simulate what import_map contains (from ModuleIndex resolution):
        // import { ZodError } from "./errors"
        let mut import_map: ImportMap = HashMap::new();
        import_map.insert("ZodError".to_string(), ("./errors".to_string(), "ZodError".to_string()));
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "typescript");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // resolve_call should find ZodError in func_index
        let result = resolve_call!(
            "ZodError",
            &CallType::Direct,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("core.ts"),
            Path::new("."),
            "typescript",
        );

        assert!(
            result.is_some(),
            "resolve_call should find ZodError when func_index key './errors' matches import_map key './errors'"
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.name, "ZodError");
        assert_eq!(resolved.file, PathBuf::from("errors.ts"));
    }

    #[test]
    fn test_resolve_call_with_receiver_typescript_module_import() {
        // Test that module imports resolve correctly for TypeScript
        let mut func_index = FuncIndex::new();
        let class_index = ClassIndex::new();

        // errors module has a createZodError function
        let module = path_to_module(Path::new("errors.ts"), "typescript");
        func_index.insert(&module, "createZodError", FuncEntry::function(
            PathBuf::from("errors.ts"), 5, 15,
        ));

        let import_map: ImportMap = HashMap::new();
        let mut module_imports: ModuleImports = HashMap::new();
        // import * as errors from "./errors"
        module_imports.insert("errors".to_string(), "./errors".to_string());
        let module_index = ModuleIndex::new(PathBuf::from("."), "typescript");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // errors.createZodError() should resolve
        let result = resolve_call_with_receiver!(
            "createZodError",
            "errors",
            None,
            &CallType::Attr,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("core.ts"),
            Path::new("."),
            "typescript",
        );

        assert!(
            result.is_some(),
            "resolve_call_with_receiver should find createZodError via module import './errors'"
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.name, "createZodError");
    }

    // =========================================================================
    // Tests for Strategy 7/8 self-receiver false positive filtering
    // =========================================================================

    /// Test: Strategy 8 should NOT match a method from an unrelated class
    /// when receiver is "self" and receiver_type is set.
    ///
    /// Scenario: CaseInsensitiveDict calls self.items() internally.
    /// RequestsCookieJar also defines items(). Strategy 8 (global scan) should
    /// NOT match RequestsCookieJar.items() because self refers to
    /// CaseInsensitiveDict, not RequestsCookieJar.
    ///
    /// Setup: CaseInsensitiveDict is NOT in the class_index (simulating it being
    /// missed or external), so Strategy 0's resolve_method_in_class fails.
    /// The only func_index entry for "items" belongs to RequestsCookieJar.
    /// Strategy 8 would match it as a "unique" method -- FALSE POSITIVE.
    #[test]
    fn test_strategy8_self_receiver_filters_unrelated_class() {
        let mut func_index = FuncIndex::new();
        let mut class_index = ClassIndex::new();

        // RequestsCookieJar defines items() in cookies.py -- indexed with BARE name
        func_index.insert(
            "cookies",
            "items",
            FuncEntry::method(PathBuf::from("cookies.py"), 80, 90, "RequestsCookieJar".to_string()),
        );
        class_index.insert(
            "RequestsCookieJar",
            ClassEntry::new(
                PathBuf::from("cookies.py"), 5, 200,
                vec!["items".to_string(), "values".to_string()],
                vec!["cookielib.CookieJar".to_string()],
            ),
        );

        // CaseInsensitiveDict is NOT in class_index (Strategy 0 will fail to find it)
        // but receiver_type IS set (from apply_type_resolution which uses enclosing class)

        let import_map: ImportMap = HashMap::new();
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: self.items() inside CaseInsensitiveDict (file=structures.py)
        // receiver="self", receiver_type=Some("CaseInsensitiveDict")
        // Strategy 0: resolve_method_in_class("CaseInsensitiveDict", "items") fails (not in class_index)
        // Strategy 1: func_index.get("structures", "items") fails (no such entry)
        // ...
        // Strategy 8: finds "items" as unique method -- SHOULD be filtered out
        let result = resolve_call_with_receiver!(
            "items",
            "self",
            Some("CaseInsensitiveDict"),
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("structures.py"),
            Path::new("."),
            "python",
        );

        // Without the fix: Strategy 8 returns RequestsCookieJar.items (false positive)
        // With the fix: Strategy 8 filters it out because RequestsCookieJar is not
        // in CaseInsensitiveDict's inheritance chain
        if let Some(ref resolved) = result {
            assert_ne!(
                resolved.class_name.as_deref(),
                Some("RequestsCookieJar"),
                "self.items() in CaseInsensitiveDict must NOT resolve to RequestsCookieJar.items (false positive)"
            );
        }
    }

    /// Test: Strategy 7 (local file scan) should NOT match a method from
    /// an unrelated class when receiver is "self" and receiver_type is set.
    ///
    /// Scenario: Two classes in the same file, both define process() with bare
    /// func names. self.process() inside ClassA should NOT match ClassB.process().
    /// Uses bare func names to force past Strategies 0-6 into Strategy 7.
    #[test]
    fn test_strategy7_self_receiver_filters_unrelated_class_same_file() {
        let mut func_index = FuncIndex::new();
        let mut class_index = ClassIndex::new();

        // Use bare method name "process" (not "ClassA.process") to bypass Strategy 0/1.
        // Both are in the same file (module.py) to trigger Strategy 7.
        // Strategy 7 iterates func_index looking for bare_target matching in current_file.
        // With bare names, it will match the FIRST one it finds -- which could be ClassB.

        // Insert ClassB.process first (to make it the "wrong" match for Strategy 7)
        func_index.insert(
            "module_b",
            "process",
            FuncEntry::method(PathBuf::from("module.py"), 30, 40, "ClassB".to_string()),
        );
        // Insert ClassA.process second
        func_index.insert(
            "module_a",
            "process",
            FuncEntry::method(PathBuf::from("module.py"), 10, 20, "ClassA".to_string()),
        );

        class_index.insert(
            "ClassA",
            ClassEntry::new(
                PathBuf::from("module.py"), 5, 25,
                vec!["process".to_string()],
                vec![],
            ),
        );
        class_index.insert(
            "ClassB",
            ClassEntry::new(
                PathBuf::from("module.py"), 26, 45,
                vec!["process".to_string()],
                vec![],
            ),
        );

        let import_map: ImportMap = HashMap::new();
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: self.process() inside ClassA (file=module.py)
        // receiver="self", receiver_type=Some("ClassA")
        let result = resolve_call_with_receiver!(
            "process",
            "self",
            Some("ClassA"),
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("module.py"),
            Path::new("."),
            "python",
        );

        // With the fix: should resolve to ClassA.process, not ClassB.process
        if let Some(ref resolved) = result {
            assert_ne!(
                resolved.class_name.as_deref(),
                Some("ClassB"),
                "self.process() in ClassA must NOT resolve to ClassB.process (false positive)"
            );
        }
    }

    /// Test: Strategy 8 should still work for non-self receivers
    /// (no false-positive filtering when receiver is a variable name).
    /// Uses bare func name so find_by_name matches.
    #[test]
    fn test_strategy8_non_self_receiver_still_resolves_unique() {
        let mut func_index = FuncIndex::new();
        let class_index = ClassIndex::new();

        // Only one class defines unique_method() -- use bare func name
        func_index.insert(
            "helpers",
            "unique_method",
            FuncEntry::method(PathBuf::from("helpers.py"), 10, 20, "Helper".to_string()),
        );

        let import_map: ImportMap = HashMap::new();
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: obj.unique_method() -- obj is NOT self, and unique_method is globally unique
        let result = resolve_call_with_receiver!(
            "unique_method",
            "obj",
            None,
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.py"),
            Path::new("."),
            "python",
        );

        assert!(
            result.is_some(),
            "obj.unique_method() should still resolve via Strategy 8 when unique"
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.name, "unique_method");
    }

    /// Test: Strategy 8 with self receiver should resolve to base class method
    /// when the method is defined in a parent class.
    /// Uses bare func names to force into Strategy 8.
    #[test]
    fn test_strategy8_self_receiver_allows_base_class_method() {
        let mut func_index = FuncIndex::new();
        let mut class_index = ClassIndex::new();

        // BaseClass defines save() in base.py -- use bare func name
        func_index.insert(
            "base",
            "save",
            FuncEntry::method(PathBuf::from("base.py"), 10, 20, "BaseClass".to_string()),
        );

        // ChildClass inherits from BaseClass (defined in child.py)
        class_index.insert(
            "ChildClass",
            ClassEntry::new(
                PathBuf::from("child.py"), 5, 50,
                vec!["run".to_string()],
                vec!["BaseClass".to_string()],
            ),
        );
        class_index.insert(
            "BaseClass",
            ClassEntry::new(
                PathBuf::from("base.py"), 1, 30,
                vec!["save".to_string()],
                vec![],
            ),
        );

        // UnrelatedClass also defines save() in other.py -- bare func name
        func_index.insert(
            "other",
            "save",
            FuncEntry::method(PathBuf::from("other.py"), 10, 20, "UnrelatedClass".to_string()),
        );
        class_index.insert(
            "UnrelatedClass",
            ClassEntry::new(
                PathBuf::from("other.py"), 1, 30,
                vec!["save".to_string()],
                vec![],
            ),
        );

        let import_map: ImportMap = HashMap::new();
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: self.save() inside ChildClass (file=child.py)
        // receiver="self", receiver_type=Some("ChildClass")
        // save() is not in ChildClass but IS in BaseClass (parent)
        // Strategy 8 finds two "save" entries -- must filter to inheritance chain
        let result = resolve_call_with_receiver!(
            "save",
            "self",
            Some("ChildClass"),
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("child.py"),
            Path::new("."),
            "python",
        );

        assert!(
            result.is_some(),
            "self.save() in ChildClass should resolve to base class BaseClass.save"
        );
        let resolved = result.unwrap();
        assert_eq!(
            resolved.class_name.as_deref(),
            Some("BaseClass"),
            "self.save() should resolve to BaseClass.save (inherited), not {:?}",
            resolved.class_name
        );
        assert_eq!(resolved.file, PathBuf::from("base.py"));
    }

    /// Test: Strategy 8 with stdlib receiver_type should filter out project methods.
    ///
    /// Scenario: self._store.items() where _store is an OrderedDict (stdlib).
    /// The only "items" method in func_index belongs to RequestsCookieJar.
    /// Since OrderedDict is a stdlib type, items() should NOT resolve to
    /// RequestsCookieJar.items().
    #[test]
    fn test_strategy8_stdlib_receiver_type_filters_project_methods() {
        let mut func_index = FuncIndex::new();
        let mut class_index = ClassIndex::new();

        // RequestsCookieJar defines items() -- indexed with bare name
        func_index.insert(
            "cookies",
            "items",
            FuncEntry::method(PathBuf::from("cookies.py"), 80, 90, "RequestsCookieJar".to_string()),
        );
        class_index.insert(
            "RequestsCookieJar",
            ClassEntry::new(
                PathBuf::from("cookies.py"), 5, 200,
                vec!["items".to_string()],
                vec![],
            ),
        );

        let import_map: ImportMap = HashMap::new();
        let module_imports: ModuleImports = HashMap::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Call: self._store.items() inside CaseInsensitiveDict
        // receiver="_store", receiver_type=Some("OrderedDict")
        // OrderedDict is a stdlib type -- not in class_index
        let result = resolve_call_with_receiver!(
            "items",
            "_store",
            Some("OrderedDict"),
            &CallType::Method,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("structures.py"),
            Path::new("."),
            "python",
        );

        // Should NOT resolve to RequestsCookieJar.items because
        // OrderedDict.items() is a stdlib method call
        if let Some(ref resolved) = result {
            assert_ne!(
                resolved.class_name.as_deref(),
                Some("RequestsCookieJar"),
                "OrderedDict.items() must NOT resolve to RequestsCookieJar.items (false positive)"
            );
        }
    }

    /// Test: augment_go_module_imports with resolve_call_with_receiver (Strategy 2)
    ///
    /// End-to-end test: Go import creates module_imports entry,
    /// then resolve_call_with_receiver uses Strategy 2 to resolve
    /// models.NewUser() to the correct function.
    #[test]
    fn test_go_cross_package_resolve_end_to_end() {
        // Setup func_index with Go functions
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "pkg/models",
            "NewUser",
            FuncEntry::function(PathBuf::from("pkg/models/user.go"), 12, 14),
        );
        func_index.insert(
            "pkg/models",
            "NewAdmin",
            FuncEntry::function(PathBuf::from("pkg/models/user.go"), 33, 38),
        );
        func_index.insert(
            "pkg/service",
            "NewUserService",
            FuncEntry::function(PathBuf::from("pkg/service/service.go"), 10, 13),
        );

        // Build module_imports via augment_go_module_imports
        let imports = vec![
            ImportDef::simple_import("go-callgraph-test/pkg/models"),
            ImportDef::simple_import("go-callgraph-test/pkg/service"),
        ];
        let mut module_imports = ModuleImports::new();
        augment_go_module_imports(&imports, &mut module_imports, &func_index);

        let import_map = ImportMap::new();
        let class_index = ClassIndex::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "go");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Test: models.NewUser() should resolve
        let resolved = resolve_call_with_receiver!(
            "models.NewUser",
            "models",
            None,
            &CallType::Attr,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.go"),
            Path::new("/project"),
            "go",
        );

        assert!(
            resolved.is_some(),
            "models.NewUser() should resolve via Strategy 2"
        );
        let target = resolved.unwrap();
        assert_eq!(target.name, "NewUser");
        assert_eq!(target.file, PathBuf::from("pkg/models/user.go"));

        // Test: service.NewUserService() should resolve
        let resolved2 = resolve_call_with_receiver!(
            "service.NewUserService",
            "service",
            None,
            &CallType::Attr,
            &import_map,
            &module_imports,
            &func_index,
            &class_index,
            &mut reexport_tracer,
            Path::new("main.go"),
            Path::new("/project"),
            "go",
        );

        assert!(
            resolved2.is_some(),
            "service.NewUserService() should resolve via Strategy 2"
        );
        let target2 = resolved2.unwrap();
        assert_eq!(target2.name, "NewUserService");
        assert_eq!(target2.file, PathBuf::from("pkg/service/service.go"));
    }
}
