use std::env;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result};

use tldr_core::callgraph::builder_v2::{
    apply_type_resolution, build_import_map, extract_and_resolve_calls, path_to_module,
    resolve_imports_for_file, ClassEntry, ClassIndex, FuncEntry, FuncIndex, ResolutionContext,
};
use tldr_core::callgraph::{
    build_project_call_graph_v2, BuildConfig, ImportResolver, ModuleIndex, ReExportTracer,
};
use tldr_core::types::Language;

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .context("usage: callgraph_resolution_stats <path> <language>")?;
    let language = args
        .next()
        .context("usage: callgraph_resolution_stats <path> <language>")?;

    let root = PathBuf::from(path);
    let use_type_resolution = true;
    let config = BuildConfig {
        language: language.clone(),
        use_type_resolution,
        respect_ignore: true,
        ..Default::default()
    };

    let ir = build_project_call_graph_v2(&root, config)?;

    let total_calls: usize = ir
        .files
        .values()
        .map(|file_ir| {
            file_ir
                .calls
                .values()
                .map(|calls| calls.len())
                .sum::<usize>()
        })
        .sum();

    let resolved_edges = ir.edges.len();

    let module_index = ModuleIndex::build(&root, &language)?;
    let mut import_resolver = ImportResolver::with_default_cache(&module_index);
    let mut reexport_tracer = ReExportTracer::new(&module_index);

    let mut func_index = FuncIndex::with_capacity(ir.function_count());
    let mut class_index = ClassIndex::with_capacity(ir.class_count());

    for (file_path, file_ir) in &ir.files {
        let module = path_to_module(file_path, &language);

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

            let is_python_style = !module.starts_with("./")
                && !module.starts_with("crate::")
                && !module.contains('/');
            let simple_module = if is_python_style {
                module.split('.').next_back().unwrap_or(&module)
            } else {
                &module
            };
            if is_python_style && simple_module != module.as_str() {
                func_index.insert(simple_module, &func.name, entry);
            }

            if let Some(ref class_name) = func.class_name {
                let qualified = format!("{}.{}", class_name, func.name);
                let method_entry = FuncEntry::method(
                    file_path.clone(),
                    func.line,
                    func.end_line,
                    class_name.clone(),
                );
                func_index.insert(&module, &qualified, method_entry.clone());
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

    let mut resolved_call_sites = 0usize;
    let mut unresolved_call_sites = 0usize;

    for file_ir in ir.files.values() {
        let mut file_ir = file_ir.clone();

        if use_type_resolution {
            if let Ok(lang) = Language::from_str(&language) {
                if let Ok(source) = fs::read_to_string(root.join(&file_ir.path)) {
                    apply_type_resolution(&mut file_ir, &source, lang);
                }
            }
        }

        let resolved_imports = resolve_imports_for_file(&file_ir, &mut import_resolver, &root);
        let (import_map, module_imports) = build_import_map(&resolved_imports);
        let mut resolution_context = ResolutionContext {
            import_map: &import_map,
            module_imports: &module_imports,
            func_index: &func_index,
            class_index: &class_index,
            reexport_tracer: &mut reexport_tracer,
            current_file: &file_ir.path,
            root: &root,
            language: &language,
        };
        let resolved_calls = extract_and_resolve_calls(&file_ir, &mut resolution_context);

        let file_call_sites: usize = file_ir.calls.values().map(|calls| calls.len()).sum();
        let unresolved_len = resolved_calls.unresolved.len();
        let resolved_len = file_call_sites.saturating_sub(unresolved_len);

        resolved_call_sites += resolved_len;
        unresolved_call_sites += unresolved_len;
    }

    let skipped_call_sites =
        total_calls.saturating_sub(resolved_call_sites + unresolved_call_sites);
    let callsite_pct = if total_calls > 0 {
        (resolved_call_sites as f64 / total_calls as f64) * 100.0
    } else {
        0.0
    };
    let unique_edge_pct = if total_calls > 0 {
        (resolved_edges as f64 / total_calls as f64) * 100.0
    } else {
        0.0
    };

    println!(
        "{},{},{},{},{},{},{:.2},{:.2}",
        language,
        resolved_edges,
        total_calls,
        resolved_call_sites,
        unresolved_call_sites,
        skipped_call_sites,
        callsite_pct,
        unique_edge_pct
    );

    Ok(())
}
