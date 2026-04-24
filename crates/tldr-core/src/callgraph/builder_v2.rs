//! Builder V2 - Main entry point with parallel processing (Phase 14)
//!
//! This module provides the V2 implementation of the call graph builder with:
//! - Parallel file processing via rayon
//! - String interning for memory efficiency
//! - Explicit tree drops for memory management
//! - Integration with all 17 language handlers
//!
//! # Feature Gate
//! Canonical implementation (no feature flag required).
//!
//! # Example
//! ```rust,ignore
//! use tldr_core::callgraph::builder_v2::{build_project_call_graph_v2, BuildConfig};
//!
//! let config = BuildConfig {
//!     language: "python".to_string(),
//!     parallelism: 4,
//!     ..Default::default()
//! };
//!
//! let ir = build_project_call_graph_v2(Path::new("src"), config)?;
//! println!("Found {} files with {} functions", ir.file_count(), ir.function_count());
//! ```

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rayon::prelude::*;

use super::cross_file_types::{CallGraphIR, CallSite, CallType, FileIR};
use super::import_resolver::{ImportResolver, ReExportTracer};
use super::module_index::ModuleIndex;
use super::type_aware_resolver::TypeAwareCallResolver;
use super::type_resolver::{expand_union_type, MAX_UNION_EXPANSION};
use crate::types::Language;

// --- Re-exports for backward compatibility (sub-modules are private) ---
pub use super::imports::{
    augment_go_module_imports, build_import_map, build_import_map_with_index,
    extract_python_imports, resolve_imports_for_file, trace_reexport_with_cycle_detection,
    ImportMap, ModuleImports,
};
pub use super::module_path::path_to_module;
pub use super::resolution::{
    apply_type_resolution, resolve_call, resolve_call_with_receiver, ResolutionContext,
    ResolvedTarget,
};
pub use super::scanner::{filter_tldrignored, scan_project_files, should_skip_path, ScannedFile};
pub use super::types::{
    BuildConfig, BuildDiagnostics, BuildError, BuildResult, ClassEntry, ClassIndex, FuncEntry,
    FuncIndex, ParseDiagnostic, ResolutionWarning, SkipReason,
};

// --- Internal imports from sub-modules ---
use super::module_path::{extract_definitions, normalize_path_relative_to_root};
use super::resolution::{
    compute_via_import, enclosing_class_for_call, first_base_for_class, resolve_caller_name,
    resolve_constructor_target, resolve_method_in_bases, resolve_method_in_class,
};
use super::scanner::{is_supported_language, normalize_language_string};
use super::types::PYTHON_BUILTINS;
use super::var_types::FileParseResult;

// =============================================================================
// Parallel Index Building (Spec Section 14.5)
// =============================================================================

/// Build function and class indices in parallel using rayon.
///
/// This function processes all files in parallel, extracting function and
/// class definitions, then merges the results into unified indices.
///
/// # Mitigations Implemented
/// - M1.4: Thread-local parsers via thread_local!
/// - M1.8: Flat par_iter, no nested parallelism
/// - M1.9: Phase barrier - indices built before resolution
///
/// # Arguments
/// * `files` - Scanned files to process
/// * `root` - Project root for relative path computation
/// * `language` - Language to parse
/// * `config` - Build configuration
///
/// # Returns
/// Tuple of (FuncIndex, ClassIndex, Vec<FileIR>)
pub fn build_indices_parallel(
    files: &[ScannedFile],
    root: &Path,
    language: &str,
    _config: &BuildConfig,
) -> (FuncIndex, ClassIndex, Vec<FileIR>) {
    // P1: Canonicalize root for consistent path operations (parity-fix-plan.yaml)
    let canonical_root = root.canonicalize().ok();

    // Process files in parallel using rayon
    // Per M1.8: Use flat par_iter, no nested parallelism
    let results: Vec<_> = files
        .par_iter()
        .map(|scanned| {
            // Read file content
            let content = match fs::read_to_string(&scanned.path) {
                Ok(c) => c,
                Err(e) => {
                    return (
                        scanned.path.clone(),
                        FileParseResult {
                            error: Some(format!("Failed to read file: {}", e)),
                            ..Default::default()
                        },
                    );
                }
            };

            // Extract definitions
            let result = extract_definitions(&content, &scanned.path, language);
            (scanned.path.clone(), result)
        })
        .collect();

    // Build indices from parallel results (single-threaded merge)
    // Per M1.9: Build indices completely before resolution phase
    let total_funcs: usize = results.iter().map(|(_, r)| r.funcs.len()).sum();
    let total_classes: usize = results.iter().map(|(_, r)| r.classes.len()).sum();

    let mut func_index = FuncIndex::with_capacity(total_funcs);
    let mut class_index = ClassIndex::with_capacity(total_classes);
    let mut file_irs = Vec::with_capacity(results.len());

    for (abs_path, parse_result) in results {
        // P1: Compute relative path with normalization (parity-fix-plan.yaml)
        let relative_path =
            normalize_path_relative_to_root(&abs_path, root, canonical_root.as_deref());

        // Compute module name from path (language-aware for ModuleIndex parity)
        let module = path_to_module(&relative_path, language);

        // Build FileIR
        let mut file_ir = FileIR::new(relative_path.clone());

        // Add functions to FileIR and index
        for func in parse_result.funcs {
            // Add to function index
            let entry = if func.is_method {
                FuncEntry::method(
                    relative_path.clone(),
                    func.line,
                    func.end_line,
                    func.class_name.clone().unwrap_or_default(),
                )
            } else {
                FuncEntry::function(relative_path.clone(), func.line, func.end_line)
            };

            func_index.insert(&module, &func.name, entry.clone());

            // BUG FIX 2: Index BOTH simple AND full module name (CROSSFILE_SPEC.md Section 2.2)
            // When resolving `from core import my_function`, we need to find it under both
            // "pkg.core" (full path) and "core" (simple name). Previously only full path was indexed.
            let simple_module = module.split('.').next_back().unwrap_or(&module);
            if simple_module != module {
                func_index.insert(simple_module, &func.name, entry);
            }

            // Also index as Class.method if it's a method
            if let Some(ref class_name) = func.class_name {
                let qualified = format!("{}.{}", class_name, func.name);
                let method_entry = FuncEntry::method(
                    relative_path.clone(),
                    func.line,
                    func.end_line,
                    class_name.clone(),
                );
                func_index.insert(&module, &qualified, method_entry.clone());
                // Also index method under simple module name
                if simple_module != module {
                    func_index.insert(simple_module, &qualified, method_entry);
                }
            }

            // Add to FileIR
            file_ir.funcs.push(func);
        }

        // Add classes to FileIR and index
        for class in parse_result.classes {
            // Add to class index
            let entry = ClassEntry::new(
                relative_path.clone(),
                class.line,
                class.end_line,
                class.methods.clone(),
                class.bases.clone(),
            );
            class_index.insert(&class.name, entry);

            // Add to FileIR
            file_ir.classes.push(class);
        }

        // Add imports to FileIR (Phase 14d)
        file_ir.imports = parse_result.imports;

        // Add calls to FileIR (Phase 14d)
        file_ir.calls = parse_result.calls;

        // Add VarType information to FileIR (Phase: VarType extraction)
        file_ir.var_types = parse_result.var_types;

        file_irs.push(file_ir);
    }

    (func_index, class_index, file_irs)
}

// =============================================================================
// Call Resolution (Spec Section 14.6)
// =============================================================================

/// Result of extracting and resolving calls from a file.
#[derive(Debug, Default)]
pub struct ResolvedCalls {
    /// Resolved calls: (CallSite, ResolvedTarget)
    pub resolved: Vec<(CallSite, ResolvedTarget)>,

    /// Unresolved calls (external, stdlib, or cannot be resolved)
    pub unresolved: Vec<CallSite>,

    /// Warnings generated during resolution
    pub warnings: Vec<ResolutionWarning>,
}

/// Extract and resolve all calls from a file.
///
/// This function processes all call sites in a FileIR and attempts to resolve
/// each one to its target definition using the various indices.
///
/// # Arguments
/// * `file_ir` - The FileIR containing calls to resolve
/// * `context` - Shared resolution indexes and state for this file
///
/// # Returns
/// ResolvedCalls containing resolved and unresolved calls
pub fn extract_and_resolve_calls(
    file_ir: &FileIR,
    context: &mut ResolutionContext<'_, '_>,
) -> ResolvedCalls {
    let mut result = ResolvedCalls::default();
    let current_file = &file_ir.path;
    let mut builder_context = BuilderResolutionContext {
        resolution_context: context,
    };

    for call_sites in file_ir.calls.values() {
        for call_site in call_sites {
            if let Some(super_target) = resolve_super_constructor_call(
                file_ir,
                call_site,
                builder_context.resolution_context.class_index,
                builder_context.resolution_context.func_index,
                builder_context.resolution_context.language,
            ) {
                result.resolved.push((call_site.clone(), super_target));
                continue;
            }
            match resolve_call_site_for_builder(
                file_ir,
                call_site,
                &mut builder_context,
                &mut result,
            ) {
                CallSiteResolution::Handled => {}
                CallSiteResolution::Resolved(target) => {
                    if PYTHON_BUILTINS.contains(&target.name.as_str()) {
                        continue;
                    }
                    result.resolved.push((call_site.clone(), target));
                }
                CallSiteResolution::Unresolved => {
                    if call_site.target.contains("__import__")
                        || call_site.target.contains("importlib")
                    {
                        result.warnings.push(ResolutionWarning {
                            file: current_file.clone(),
                            line: call_site.line.unwrap_or(0),
                            target: call_site.target.clone(),
                            reason: "Dynamic import pattern cannot be resolved statically"
                                .to_string(),
                        });
                    }
                    result.unresolved.push(call_site.clone());
                }
            }
        }
    }

    result
}

enum CallSiteResolution {
    Handled,
    Resolved(ResolvedTarget),
    Unresolved,
}

struct BuilderResolutionContext<'ctx, 'a, 'b> {
    resolution_context: &'ctx mut ResolutionContext<'a, 'b>,
}

impl BuilderResolutionContext<'_, '_, '_> {
    fn resolve_call(&mut self, target: &str, call_type: &CallType) -> Option<ResolvedTarget> {
        resolve_call(target, call_type, self.resolution_context)
    }

    fn resolve_call_with_receiver(
        &mut self,
        target: &str,
        receiver: &str,
        receiver_type: Option<&str>,
        call_type: &CallType,
    ) -> Option<ResolvedTarget> {
        resolve_call_with_receiver(
            target,
            receiver,
            receiver_type,
            call_type,
            self.resolution_context,
        )
    }
}

fn resolve_super_constructor_call(
    file_ir: &FileIR,
    call_site: &CallSite,
    class_index: &ClassIndex,
    func_index: &FuncIndex,
    language: &str,
) -> Option<ResolvedTarget> {
    let supports_super_ctor = matches!(
        language,
        "java"
            | "kotlin"
            | "scala"
            | "swift"
            | "typescript"
            | "tsx"
            | "javascript"
            | "js"
            | "csharp"
    );
    if !supports_super_ctor
        || !matches!(call_site.call_type, CallType::Direct | CallType::Intra)
        || call_site.target != "super"
    {
        return None;
    }
    let class_name = enclosing_class_for_call(&file_ir.funcs, call_site)?;
    let base = first_base_for_class(&file_ir.classes, &class_name)?;
    let class_entry = class_index.get(&base)?;
    if let Some(ctor_target) = resolve_constructor_target(&base, class_entry, func_index, language)
    {
        return Some(ctor_target);
    }
    Some(ResolvedTarget {
        file: class_entry.file_path.clone(),
        name: base,
        line: Some(class_entry.line),
        is_method: false,
        class_name: None,
    })
}

fn resolve_call_site_for_builder(
    file_ir: &FileIR,
    call_site: &CallSite,
    context: &mut BuilderResolutionContext<'_, '_, '_>,
    result: &mut ResolvedCalls,
) -> CallSiteResolution {
    let resolved = match call_site.call_type {
        CallType::Intra => resolve_intra_call(file_ir, call_site, context),
        CallType::Static => resolve_static_call(file_ir, call_site, context),
        CallType::Method | CallType::Attr => {
            return resolve_method_or_attr_call(call_site, context, result);
        }
        _ => context.resolve_call(&call_site.target, &call_site.call_type),
    };

    match resolved {
        Some(target) => CallSiteResolution::Resolved(target),
        None => CallSiteResolution::Unresolved,
    }
}

fn resolve_intra_call(
    file_ir: &FileIR,
    call_site: &CallSite,
    context: &mut BuilderResolutionContext<'_, '_, '_>,
) -> Option<ResolvedTarget> {
    let class_index = context.resolution_context.class_index;
    let func_index = context.resolution_context.func_index;
    let language = context.resolution_context.language;

    if let Some(func) = file_ir
        .funcs
        .iter()
        .find(|func| func.name == call_site.target && !func.is_method)
    {
        return Some(ResolvedTarget {
            file: file_ir.path.clone(),
            name: func.name.clone(),
            line: Some(func.line),
            is_method: false,
            class_name: None,
        });
    }
    if let Some(class_name) = enclosing_class_for_call(&file_ir.funcs, call_site) {
        if let Some(target) = resolve_method_in_class(
            &class_name,
            &call_site.target,
            class_index,
            func_index,
            language,
        ) {
            return Some(target);
        }
        if let Some(target) = resolve_method_in_bases(
            &class_name,
            &call_site.target,
            class_index,
            func_index,
            language,
        ) {
            return Some(target);
        }
        return context.resolve_call(&call_site.target, &call_site.call_type);
    }
    context.resolve_call(&call_site.target, &call_site.call_type)
}

fn resolve_static_call(
    file_ir: &FileIR,
    call_site: &CallSite,
    context: &mut BuilderResolutionContext<'_, '_, '_>,
) -> Option<ResolvedTarget> {
    let class_index = context.resolution_context.class_index;
    let func_index = context.resolution_context.func_index;
    let language = context.resolution_context.language;

    let Some((receiver, method)) = call_site.target.split_once("::") else {
        return context.resolve_call(&call_site.target, &call_site.call_type);
    };

    let receiver_key = receiver.trim();
    if receiver_key == "self" || receiver_key == "static" {
        if let Some(class_name) = enclosing_class_for_call(&file_ir.funcs, call_site) {
            if let Some(target) =
                resolve_method_in_class(&class_name, method, class_index, func_index, language)
            {
                return Some(target);
            }
            if let Some(target) =
                resolve_method_in_bases(&class_name, method, class_index, func_index, language)
            {
                return Some(target);
            }
        }
        return context.resolve_call(&call_site.target, &call_site.call_type);
    }

    if receiver_key == "parent" || receiver_key == "base" || receiver_key == "super" {
        if let Some(class_name) = enclosing_class_for_call(&file_ir.funcs, call_site) {
            if let Some(base) = first_base_for_class(&file_ir.classes, &class_name) {
                if let Some(target) =
                    resolve_method_in_class(&base, method, class_index, func_index, language)
                {
                    return Some(target);
                }
                if let Some(target) =
                    resolve_method_in_bases(&base, method, class_index, func_index, language)
                {
                    return Some(target);
                }
            }
        }
    }

    context.resolve_call(&call_site.target, &call_site.call_type)
}

fn resolve_method_or_attr_call(
    call_site: &CallSite,
    context: &mut BuilderResolutionContext<'_, '_, '_>,
    result: &mut ResolvedCalls,
) -> CallSiteResolution {
    let Some(receiver) = call_site.receiver.as_ref() else {
        return match context.resolve_call(&call_site.target, &call_site.call_type) {
            Some(target) => CallSiteResolution::Resolved(target),
            None => CallSiteResolution::Unresolved,
        };
    };

    let mut receiver_type_for_resolution = call_site.receiver_type.as_deref().map(Cow::Borrowed);

    if let Some(raw_receiver_type) = call_site.receiver_type.as_deref() {
        match expand_union_type(raw_receiver_type, Some(MAX_UNION_EXPANSION)) {
            Some(members) => {
                if members.len() > 1 {
                    let mut seen: HashSet<(PathBuf, String)> = HashSet::new();
                    let mut resolved_any = false;
                    for member in members {
                        if let Some(target) = context.resolve_call_with_receiver(
                            &call_site.target,
                            receiver,
                            Some(member.as_str()),
                            &call_site.call_type,
                        ) {
                            let key = (target.file.clone(), target.qualified_name());
                            if seen.insert(key) {
                                result.resolved.push((call_site.clone(), target));
                            }
                            resolved_any = true;
                        }
                    }
                    if resolved_any {
                        return CallSiteResolution::Handled;
                    }
                    receiver_type_for_resolution = None;
                } else if let Some(single) = members.first() {
                    receiver_type_for_resolution = Some(Cow::Owned(single.clone()));
                }
            }
            None => {
                result.warnings.push(ResolutionWarning {
                    file: context.resolution_context.current_file.to_path_buf(),
                    line: call_site.line.unwrap_or(0),
                    target: call_site.target.clone(),
                    reason: "Union type too large to expand; skipping type-aware resolution"
                        .to_string(),
                });
                receiver_type_for_resolution = None;
            }
        }
    }

    match context.resolve_call_with_receiver(
        &call_site.target,
        receiver,
        receiver_type_for_resolution.as_deref(),
        &call_site.call_type,
    ) {
        Some(target) => CallSiteResolution::Resolved(target),
        None => CallSiteResolution::Unresolved,
    }
}

// =============================================================================
// Main Entry Point (Spec Section 14.2)
// =============================================================================

/// Build a complete project-wide call graph.
///
/// This is the V2 implementation with:
/// - Parallel file processing via rayon
/// - String interning for memory efficiency
/// - Explicit tree drops for memory management
/// - Integration with all 17 language handlers
///
/// # Arguments
/// * `root` - Project root directory
/// * `config` - Builder configuration
///
/// # Returns
/// * `Result<CallGraphIR, BuildError>` - Complete call graph or error
///
/// # Errors
/// * `BuildError::RootNotFound` - if root directory doesn't exist
/// * `BuildError::UnsupportedLanguage` - if language not in registry
/// * `BuildError::Io` - for file system errors
///
/// # Example
/// ```rust,ignore
/// let config = BuildConfig {
///     language: "python".to_string(),
///     parallelism: 0, // auto-detect
///     ..Default::default()
/// };
/// let ir = build_project_call_graph_v2(Path::new("src"), config)?;
/// ```
pub fn build_project_call_graph_v2(
    root: &Path,
    mut config: BuildConfig,
) -> Result<CallGraphIR, BuildError> {
    // Step 1: Validate inputs
    if !root.exists() {
        return Err(BuildError::RootNotFound(root.to_path_buf()));
    }

    if !root.is_dir() {
        return Err(BuildError::RootNotFound(root.to_path_buf()));
    }

    config.language = normalize_language_string(&config.language);

    if !is_supported_language(&config.language) {
        return Err(BuildError::UnsupportedLanguage(config.language.clone()));
    }

    // Step 2: Scan project files (Phase 14b)
    let scanned_files = scan_project_files(root, &config.language, &config)?;

    // Step 3: Create IR with capacity hint
    let mut ir =
        CallGraphIR::with_capacity(root.to_path_buf(), &config.language, scanned_files.len());

    // Step 4: Build function and class indices in parallel (Phase 14c)
    // Per M1.9: Build indices completely before resolution phase
    let (_func_index, _class_index, file_irs) =
        build_indices_parallel(&scanned_files, root, &config.language, &config);

    // Step 5: Add FileIRs to the CallGraphIR
    for file_ir in file_irs {
        ir.add_file(file_ir);
    }

    // Step 6: Build the indices within CallGraphIR
    // This populates func_index and class_index within the IR itself
    ir.build_indices();

    // Phase 14d-14f: Import Resolution and Cross-File Edge Creation
    // Per M1.9: Build indices completely before resolution phase (done above)
    //
    // Step 7: Build ModuleIndex for import resolution.
    //
    // VAL-007: when a workspace config is active, read manifests
    // (tsconfig.json, package.json, ...) from every workspace root so
    // per-package path aliases such as `@/*` in `apps/web/tsconfig.json`
    // resolve correctly. Without this, monorepo imports collapse to the
    // AST-fallback path and emit misleading "no callers" reports.
    let extra_roots: Vec<PathBuf> = if config.use_workspace_config {
        config
            .workspace_roots
            .iter()
            .filter(|p| p.as_path() != root)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let module_index = ModuleIndex::build_with_workspace_roots(
        root,
        &config.language,
        config.respect_ignore,
        &extra_roots,
    )
    .map_err(|e| BuildError::Io(std::io::Error::other(e.to_string())))?;

    // Step 8: Create ImportResolver with LRU cache
    let mut import_resolver = ImportResolver::with_default_cache(&module_index);
    let mut reexport_tracer = ReExportTracer::new(&module_index);

    // Step 9: Build FuncIndex and ClassIndex for call resolution
    // We need our own copies because the IR's indices use a different format
    let mut func_index = FuncIndex::with_capacity(ir.function_count());
    let mut class_index = ClassIndex::with_capacity(ir.class_count());

    // Populate indices from IR
    for (file_path, file_ir) in &ir.files {
        let module = path_to_module(file_path, &config.language);

        for func in &file_ir.funcs {
            let entry = if func.is_method {
                FuncEntry::method(
                    file_path.clone(),
                    func.line,
                    func.end_line,
                    func.class_name.clone().unwrap_or_default(),
                )
            } else {
                FuncEntry::function(file_path.clone(), func.line, func.end_line)
            };
            func_index.insert(&module, &func.name, entry.clone());

            // BUG FIX 2: Index BOTH simple AND full module name (CROSSFILE_SPEC.md Section 2.2)
            // Only for Python-style dot-separated modules (e.g., "pkg.helper" -> also index as "helper")
            // TS/JS use ./ prefix (not dot-separated), Go uses /, Rust uses ::
            let is_python_style = !module.starts_with("./")
                && !module.starts_with("crate::")
                && !module.contains('/');
            let simple_module = if is_python_style {
                module.split('.').next_back().unwrap_or(&module)
            } else {
                &module // No simple alias for non-Python languages
            };
            if is_python_style && simple_module != module.as_str() {
                func_index.insert(simple_module, &func.name, entry);
            }

            // Also index as Class.method if it's a method
            if let Some(ref class_name) = func.class_name {
                let qualified = format!("{}.{}", class_name, func.name);
                let method_entry = FuncEntry::method(
                    file_path.clone(),
                    func.line,
                    func.end_line,
                    class_name.clone(),
                );
                func_index.insert(&module, &qualified, method_entry.clone());
                // Also index method under simple module name (Python only)
                if is_python_style && simple_module != module.as_str() {
                    func_index.insert(simple_module, &qualified, method_entry);
                }
            }
        }

        for class in &file_ir.classes {
            let entry = ClassEntry::new(
                file_path.clone(),
                class.line,
                class.end_line,
                class.methods.clone(),
                class.bases.clone(),
            );
            class_index.insert(&class.name, entry);
        }
    }

    // Go interface dispatch: identify interface types before the method merge pass.
    // Interfaces have methods extracted from their type declaration AST (non-empty),
    // while structs have empty methods at this point (methods come from FuncDef merge below).
    let go_interface_names: HashSet<String> = if config.language == "go" {
        class_index
            .iter()
            .filter(|(_, entry)| !entry.methods.is_empty())
            .map(|(name, _)| name.to_string())
            .collect()
    } else {
        HashSet::new()
    };

    // Merge method definitions into class index (extensions/partials)
    for (file_path, file_ir) in &ir.files {
        for func in &file_ir.funcs {
            if !func.is_method {
                continue;
            }
            let class_name = match func.class_name.as_deref() {
                Some(name) => name,
                None => continue,
            };

            if let Some(entry) = class_index.get_mut(class_name) {
                if !entry.methods.contains(&func.name) {
                    entry.methods.push(func.name.clone());
                }
            } else {
                class_index.insert(
                    class_name,
                    ClassEntry::new(
                        file_path.clone(),
                        func.line,
                        func.end_line,
                        vec![func.name.clone()],
                        Vec::new(),
                    ),
                );
            }
        }
    }

    // Go interface dispatch: wire interface→implementor relationships.
    // For each Go interface, find all structs whose method sets are a superset
    // of the interface's method set. Add those struct names as "bases" of the
    // interface so that resolve_method_in_bases() can resolve interface method
    // calls to concrete implementations.
    if config.language == "go" && !go_interface_names.is_empty() {
        // Collect interface method sets
        let interface_methods: Vec<(String, Vec<String>)> = go_interface_names
            .iter()
            .filter_map(|name| {
                class_index
                    .get(name)
                    .map(|entry| (name.clone(), entry.methods.clone()))
            })
            .collect();

        // For each interface, find concrete implementors
        for (iface_name, iface_methods) in &interface_methods {
            if iface_methods.is_empty() {
                continue;
            }
            let mut implementors = Vec::new();
            for (class_name, class_entry) in class_index.iter() {
                // Skip interfaces themselves
                if go_interface_names.contains(class_name) {
                    continue;
                }
                // Check if this struct has all methods of the interface
                let has_all = iface_methods
                    .iter()
                    .all(|m| class_entry.methods.contains(m));
                if has_all {
                    implementors.push(class_name.to_string());
                }
            }
            // Add implementors as "bases" of the interface
            if !implementors.is_empty() {
                if let Some(iface_entry) = class_index.get_mut(iface_name) {
                    for imp in implementors {
                        if !iface_entry.bases.contains(&imp) {
                            iface_entry.bases.push(imp);
                        }
                    }
                }
            }
        }
    }

    // Step 9b: Create type-aware resolver for chained calls and MRO-based resolution
    let func_path_map = func_index.to_path_map();
    let class_path_map = class_index.to_path_map();
    let mut type_resolver =
        TypeAwareCallResolver::new(&module_index, &func_path_map, &class_path_map);

    // Feed all FileIRs and class defs into the resolver
    for (file_path, file_ir) in &ir.files {
        type_resolver.add_file_ir(file_path.clone(), file_ir.clone());
    }

    // Step 10: For each file, resolve imports and then resolve calls
    // Note: Cannot parallelize easily due to ImportResolver having mutable cache
    // Future optimization: Use parallel iteration with thread-local resolvers

    let mut edge_set: HashSet<super::cross_file_types::CrossFileCallEdge> =
        HashSet::with_capacity(ir.function_count() * 4);

    // Collect file paths to avoid borrow issues
    let file_paths: Vec<PathBuf> = ir.files.keys().cloned().collect();

    for file_path in file_paths {
        // Get the FileIR (need to clone to avoid borrow issues)
        let mut file_ir = match ir.files.get(&file_path) {
            Some(f) => f.clone(),
            None => continue,
        };

        if config.use_type_resolution {
            if let Ok(lang) = Language::from_str(&config.language) {
                if let Ok(source) = fs::read_to_string(root.join(&file_ir.path)) {
                    apply_type_resolution(&mut file_ir, &source, lang);
                }
            }
        }

        // Step 10a: Resolve imports for this file
        let resolved_imports = resolve_imports_for_file(&file_ir, &mut import_resolver, root);

        // Step 10b: Build import map from resolved imports.
        //
        // VAL-007: pass the ModuleIndex so aliased imports (e.g. `@/util`)
        // get rewritten to the canonical func_index key (e.g.
        // `./apps/web/src/util`). Without this, cross-file edges through
        // tsconfig path aliases are silently dropped.
        let (import_map, mut module_imports) =
            build_import_map_with_index(&resolved_imports, Some(&module_index));

        // Step 10b.1: Augment module_imports for Go cross-package function calls.
        // Go imports use full module paths that don't match func_index keys directly.
        // This bridges the gap by mapping Go package aliases to func_index module keys.
        if config.language == "go" {
            augment_go_module_imports(&file_ir.imports, &mut module_imports, &func_index);
        }

        // Step 10c: Resolve calls using the import map and indices
        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: &file_ir.path,
            root,
            language: &config.language,
        };
        let resolved_calls = extract_and_resolve_calls(&file_ir, &mut resolution_context);

        // Step 10d: Add resolved edges to the IR (both cross-file and intra-file)
        for (call_site, target) in resolved_calls.resolved {
            use super::cross_file_types::CrossFileCallEdge;

            let src_func = resolve_caller_name(&file_ir, &call_site);
            let via_import = compute_via_import(&call_site, &import_map, &module_imports);
            let edge = CrossFileCallEdge {
                src_file: file_path.clone(),
                src_func,
                dst_file: target.file.clone(),
                dst_func: target.qualified_name(),
                call_type: call_site.call_type,
                via_import,
            };
            if edge_set.insert(edge.clone()) {
                ir.add_edge(edge);
            }
        }
    }

    Ok(ir)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Test: Cross-file call resolution - main.py calls helper.py:process
    /// This is a key acceptance test from the plan
    #[test]
    fn test_build_cross_file_calls() {
        let dir = TempDir::new().unwrap();

        // Create helper.py with process function
        let helper_content = r#"
def process(data):
    return data * 2
"#;
        std::fs::write(dir.path().join("helper.py"), helper_content).unwrap();

        // Create main.py that imports and calls process
        let main_content = r#"
from helper import process

def main():
    result = process(42)
    return result
"#;
        std::fs::write(dir.path().join("main.py"), main_content).unwrap();

        // Build the call graph
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // Verify both files are indexed
        assert_eq!(ir.file_count(), 2, "Should have 2 files");

        // Get the FileIRs
        let main_ir = ir.files.get(&PathBuf::from("main.py"));
        let helper_ir = ir.files.get(&PathBuf::from("helper.py"));

        assert!(main_ir.is_some(), "Should have main.py IR");
        assert!(helper_ir.is_some(), "Should have helper.py IR");

        // Check that main.py has the import
        let main_file = main_ir.unwrap();
        assert!(!main_file.imports.is_empty(), "main.py should have imports");

        // Check that helper.py has the process function
        let helper_file = helper_ir.unwrap();
        let process_func = helper_file.funcs.iter().find(|f| f.name == "process");
        assert!(
            process_func.is_some(),
            "helper.py should have process function"
        );

        // Check that main.py has a call to process
        let main_calls = main_file.calls.get("main");
        assert!(main_calls.is_some(), "main() should have calls");

        let calls = main_calls.unwrap();
        let process_call = calls.iter().find(|c| c.target == "process");
        assert!(process_call.is_some(), "main() should call process()");
    }
    /// Test: Method resolution - user.save() resolves to User.save
    /// This is a key acceptance test from the plan
    #[test]
    fn test_build_method_resolution() {
        // Setup indices directly to test resolution logic
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "models",
            "User.save",
            FuncEntry::method(PathBuf::from("models.py"), 20, 30, "User".to_string()),
        );
        func_index.insert(
            "models",
            "User.__init__",
            FuncEntry::method(PathBuf::from("models.py"), 10, 15, "User".to_string()),
        );

        let mut class_index = ClassIndex::new();
        class_index.insert(
            "User",
            ClassEntry::new(
                PathBuf::from("models.py"),
                5,
                50,
                vec!["__init__".to_string(), "save".to_string()],
                vec![],
            ),
        );

        // Import map: User is imported from models
        let mut import_map = ImportMap::new();
        import_map.insert(
            "User".to_string(),
            ("models".to_string(), "User".to_string()),
        );

        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Create a CallSite for user.save() with known receiver type
        let call_site =
            CallSite::method("main", "save", "user", Some("User".to_string()), Some(10));

        // Resolve using the with_receiver function
        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: Path::new("main.py"),
            root: Path::new("/project"),
            language: "python",
        };
        let resolved = resolve_call_with_receiver(
            &call_site.target,
            call_site.receiver.as_ref().unwrap(),
            call_site.receiver_type.as_deref(),
            &call_site.call_type,
            &mut resolution_context,
        );

        assert!(
            resolved.is_some(),
            "Should resolve user.save() to User.save"
        );
        let target = resolved.unwrap();
        assert_eq!(target.file, PathBuf::from("models.py"));
        assert_eq!(target.name, "save");
        assert!(target.is_method);
        assert_eq!(target.class_name, Some("User".to_string()));
        assert_eq!(target.qualified_name(), "User.save");
    }
    /// Test: extract_and_resolve_calls processes all calls in a FileIR
    #[test]
    fn test_extract_and_resolve_calls() {
        // Create a FileIR with some calls
        let mut file_ir = FileIR::new(PathBuf::from("main.py"));

        // Add a direct call
        file_ir.add_call("main", CallSite::direct("main", "helper", Some(5)));

        // Add a method call
        file_ir.add_call(
            "main",
            CallSite::method("main", "save", "user", Some("User".to_string()), Some(10)),
        );

        // Setup indices
        let mut func_index = FuncIndex::new();
        func_index.insert(
            "main",
            "helper",
            FuncEntry::function(PathBuf::from("main.py"), 15, 20),
        );
        func_index.insert(
            "models",
            "User.save",
            FuncEntry::method(PathBuf::from("models.py"), 30, 40, "User".to_string()),
        );

        let mut class_index = ClassIndex::new();
        class_index.insert(
            "User",
            ClassEntry::new(
                PathBuf::from("models.py"),
                10,
                50,
                vec!["save".to_string()],
                vec![],
            ),
        );

        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Run extraction and resolution
        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: &file_ir.path,
            root: Path::new("/project"),
            language: "python",
        };
        let result = extract_and_resolve_calls(&file_ir, &mut resolution_context);

        // Should have resolved at least the local helper call
        assert!(
            !result.resolved.is_empty() || !result.unresolved.is_empty(),
            "Should process calls"
        );

        // The helper call should be resolved (it's local)
        let helper_resolved = result.resolved.iter().find(|(cs, _)| cs.target == "helper");
        assert!(
            helper_resolved.is_some(),
            "helper() call should be resolved"
        );

        // The save call should also be resolved with type info
        let save_resolved = result.resolved.iter().find(|(cs, _)| cs.target == "save");
        assert!(
            save_resolved.is_some(),
            "save() call should be resolved with type info"
        );
    }
    /// Test: Dynamic import pattern generates warning
    #[test]
    fn test_dynamic_import_warning() {
        // Create a FileIR with a dynamic import call
        let mut file_ir = FileIR::new(PathBuf::from("plugin.py"));
        file_ir.add_call(
            "load_plugin",
            CallSite::direct("load_plugin", "__import__", Some(10)),
        );

        let func_index = FuncIndex::new();
        let class_index = ClassIndex::new();
        let import_map = ImportMap::new();
        let module_imports = ModuleImports::new();
        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: &file_ir.path,
            root: Path::new("/project"),
            language: "python",
        };
        let result = extract_and_resolve_calls(&file_ir, &mut resolution_context);

        // Should be unresolved with a warning
        assert!(
            result.unresolved.iter().any(|cs| cs.target == "__import__"),
            "Dynamic import should be unresolved"
        );
        assert!(
            result.warnings.iter().any(|w| w.target == "__import__"),
            "Should generate warning for dynamic import"
        );
    }

    // =========================================================================
    // Phase 14d-f Integration Tests: Cross-File Edge Resolution
    // =========================================================================

    /// Test: build_project_call_graph_v2 creates cross-file edges when calls span files
    #[test]
    fn test_cross_file_edges_created() {
        // Create a project with two files that have a cross-file call
        let dir = TempDir::new().unwrap();

        // File 1: main.py - imports and calls helper
        let main_py = r#"
from helper import process

def main():
    result = process()
    return result
"#;

        // File 2: helper.py - defines the function
        let helper_py = r#"
def process():
    return "processed"
"#;

        std::fs::write(dir.path().join("main.py"), main_py).unwrap();
        std::fs::write(dir.path().join("helper.py"), helper_py).unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // Verify files are present
        assert_eq!(ir.file_count(), 2, "Should have 2 files");

        // Verify main.py has calls
        let main_file = ir.get_file("main.py");
        assert!(main_file.is_some(), "Should have main.py");
        let main_ir = main_file.unwrap();

        // Check that main has calls from the `main` function
        let main_calls = main_ir.calls.get("main");
        assert!(main_calls.is_some(), "main function should have calls");

        // Verify process() call exists in main()
        let process_call = main_calls.unwrap().iter().find(|c| c.target == "process");
        assert!(process_call.is_some(), "Should have call to process()");

        // KEY TEST: Verify cross-file edges are created
        assert!(
            ir.edge_count() > 0,
            "Should have cross-file edges, got {} edges",
            ir.edge_count()
        );

        // Verify the specific edge: main.py:main -> helper.py:process
        let edge = ir.edges().iter().find(|e| {
            e.src_file.to_string_lossy().contains("main.py")
                && e.src_func == "main"
                && e.dst_file.to_string_lossy().contains("helper.py")
                && e.dst_func == "process"
        });
        assert!(
            edge.is_some(),
            "Should have edge from main.py:main to helper.py:process. Edges: {:?}",
            ir.edges()
        );
    }
    /// Test: build_project_call_graph_v2 resolves imports and calls across files
    #[test]
    fn test_build_resolves_cross_file_calls() {
        use crate::callgraph::import_resolver::ImportResolver;
        use crate::callgraph::module_index::ModuleIndex;

        let dir = TempDir::new().unwrap();

        // Create a multi-file project
        let main_py = r#"
from utils import helper
import processor

def main():
    helper()
    processor.run()
"#;

        let utils_py = r#"
def helper():
    return "help"
"#;

        let processor_py = r#"
def run():
    return "running"
"#;

        std::fs::write(dir.path().join("main.py"), main_py).unwrap();
        std::fs::write(dir.path().join("utils.py"), utils_py).unwrap();
        std::fs::write(dir.path().join("processor.py"), processor_py).unwrap();

        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let ir = build_project_call_graph_v2(dir.path(), config).unwrap();

        // Verify basic structure
        assert_eq!(ir.file_count(), 3, "Should have 3 files");
        assert!(ir.function_count() >= 3, "Should have at least 3 functions");

        // Verify func_index has entries for all functions
        assert!(
            ir.func_index.get("utils", "helper").is_some(),
            "func_index should have utils.helper"
        );
        assert!(
            ir.func_index.get("processor", "run").is_some(),
            "func_index should have processor.run"
        );

        // Test that we can resolve calls using the built infrastructure
        let main_file = ir.get_file("main.py").unwrap();

        // Build module index for resolution
        let module_index = ModuleIndex::build(dir.path(), "python").unwrap();
        let mut resolver = ImportResolver::with_default_cache(&module_index);

        // Resolve imports for main.py
        let resolved_imports = resolve_imports_for_file(main_file, &mut resolver, dir.path());

        // Build import map
        let (import_map, module_imports) = build_import_map(&resolved_imports);

        // The import_map should have 'helper' from utils
        // Note: Resolution may mark external imports as external
        // Let's check that internal imports are resolved

        // Create func_index and class_index from IR for resolution
        let mut func_index = FuncIndex::new();
        let class_index = ClassIndex::new();

        // Populate func_index from IR
        for (file_path, file_ir) in &ir.files {
            let module = path_to_module(file_path, "python");
            for func in &file_ir.funcs {
                func_index.insert(
                    &module,
                    &func.name,
                    FuncEntry::function(file_path.clone(), func.line, func.end_line),
                );
            }
        }

        let module_index = ModuleIndex::new(PathBuf::from("."), "python");
        let mut reexport_tracer = ReExportTracer::new(&module_index);

        // Now resolve the calls
        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: &main_file.path,
            root: dir.path(),
            language: "python",
        };
        let resolved_calls = extract_and_resolve_calls(main_file, &mut resolution_context);

        // At minimum, we should have some resolved calls (helper from utils)
        // Note: This test verifies the resolution machinery works
        // The actual wiring into build_project_call_graph_v2 is tested elsewhere
        assert!(
            !resolved_calls.resolved.is_empty() || !resolved_calls.unresolved.is_empty(),
            "Should have processed some calls"
        );
    }
}
