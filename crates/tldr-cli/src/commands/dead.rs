//! Dead command - Find dead code
//!
//! Identifies functions that are never called (unreachable code).
//! Auto-routes through daemon when available for ~35x speedup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use serde::Serialize;
use tldr_core::walker::ProjectWalker;

/// Maximum number of files to scan in WalkDir traversals.
///
/// Prevents runaway scans in massive monorepos or symlink-heavy layouts.
/// Projects with fewer files are unaffected.
const MAX_FILES: usize = 10_000;

use tldr_core::analysis::dead::dead_code_analysis_refcount;
use tldr_core::analysis::refcount::count_identifiers_in_tree;
use tldr_core::ast::parser::parse_file;
use tldr_core::ast::{extract_file, extract_from_tree};
use tldr_core::types::{DeadCodeReport, ModuleInfo};
use tldr_core::{
    build_project_call_graph, collect_all_functions, dead_code_analysis, FunctionRef, Language,
};

use crate::commands::daemon_router::{params_for_dead, try_daemon_route};
use crate::output::{OutputFormat, OutputWriter};

/// Find dead (unreachable) code
#[derive(Debug, Args)]
pub struct DeadArgs {
    /// Project root directory (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Custom entry point patterns (comma-separated)
    #[arg(long, short = 'e', value_delimiter = ',')]
    pub entry_points: Vec<String>,

    /// Maximum number of dead functions to display
    #[arg(long, default_value = "100")]
    pub max_items: usize,

    /// Use call-graph-based analysis instead of the default reference counting
    #[arg(long)]
    pub call_graph: bool,

    /// Walk vendored/build dirs (node_modules, target, dist, etc.) that would normally be skipped.
    #[arg(long)]
    pub no_default_ignore: bool,
}

impl DeadArgs {
    /// Run the dead command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        // Try daemon first for cached result
        let entry_points: Option<Vec<String>> = if self.entry_points.is_empty() {
            None
        } else {
            Some(self.entry_points.clone())
        };

        if let Some(report) = try_daemon_route::<DeadCodeReport>(
            &self.path,
            "dead",
            params_for_dead(Some(&self.path), entry_points.as_deref()),
        ) {
            // Apply truncation if needed
            let (truncated_report, truncated, total_count, shown_count) =
                apply_truncation(report, self.max_items);

            // Output based on format
            if writer.is_text() {
                let text = format_dead_code_text_truncated(
                    &truncated_report,
                    truncated,
                    total_count,
                    shown_count,
                );
                writer.write_text(&text)?;
                return Ok(());
            } else {
                let output = DeadCodeOutput {
                    report: truncated_report,
                    truncated,
                    total_count,
                    shown_count,
                };
                writer.write(&output)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        let entry_points_for_analysis: Option<Vec<String>> = if self.entry_points.is_empty() {
            None
        } else {
            Some(self.entry_points.clone())
        };

        let report = if self.call_graph {
            // Old path: build call graph, then analyze
            writer.progress(&format!(
                "Building call graph for {} ({:?})...",
                self.path.display(),
                language
            ));

            let graph = build_project_call_graph(&self.path, language, None, true)?;

            writer.progress("Extracting all functions...");
            let module_infos = collect_module_infos(&self.path, language, self.no_default_ignore);
            let all_functions: Vec<FunctionRef> = collect_all_functions(&module_infos);

            writer.progress("Analyzing dead code (call graph)...");
            dead_code_analysis(&graph, &all_functions, entry_points_for_analysis.as_deref())?
        } else {
            // New default path: reference counting (single-pass)
            writer.progress(&format!(
                "Scanning {} ({:?}) with reference counting...",
                self.path.display(),
                language
            ));

            let (module_infos, merged_ref_counts) =
                collect_module_infos_with_refcounts(&self.path, language, self.no_default_ignore);
            let all_functions: Vec<FunctionRef> = collect_all_functions(&module_infos);

            writer.progress("Analyzing dead code (refcount)...");
            dead_code_analysis_refcount(
                &all_functions,
                &merged_ref_counts,
                entry_points_for_analysis.as_deref(),
            )?
        };

        // Apply truncation if needed
        let (truncated_report, truncated, total_count, shown_count) =
            apply_truncation(report, self.max_items);

        // Output based on format
        if writer.is_text() {
            let text = format_dead_code_text_truncated(
                &truncated_report,
                truncated,
                total_count,
                shown_count,
            );
            writer.write_text(&text)?;
        } else {
            let output = DeadCodeOutput {
                report: truncated_report,
                truncated,
                total_count,
                shown_count,
            };
            writer.write(&output)?;
        }

        Ok(())
    }
}

/// Check if JS/TS source has a file-level 'use server' or 'use client' directive.
/// This is checked on the source string directly (no file I/O) to avoid path resolution issues.
fn source_has_framework_directive(source: &str, ext: &str) -> bool {
    if !matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mjs") {
        return false;
    }
    for line in source.lines().take(5) {
        let trimmed = line.trim();
        if trimmed == r#""use server""#
            || trimmed == r#"'use server'"#
            || trimmed == r#""use server";"#
            || trimmed == r#"'use server';"#
            || trimmed == r#""use client""#
            || trimmed == r#"'use client'"#
            || trimmed == r#""use client";"#
            || trimmed == r#"'use client';"#
        {
            return true;
        }
        // Skip empty lines and comments
        if !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("/*")
            && !trimmed.starts_with('*')
            && !trimmed.starts_with('"')
            && !trimmed.starts_with('\'')
        {
            break;
        }
    }
    false
}

/// Tag all functions and class methods in a ModuleInfo with a synthetic decorator
/// if the source contains a framework directive ('use server'/'use client').
fn tag_directive_functions(info: &mut ModuleInfo, source: &str, path: &Path) {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if source_has_framework_directive(source, ext) {
        for func in &mut info.functions {
            if !func
                .decorators
                .contains(&"use_server_directive".to_string())
            {
                func.decorators.push("use_server_directive".to_string());
            }
        }
        for class in &mut info.classes {
            for method in &mut class.methods {
                if !method
                    .decorators
                    .contains(&"use_server_directive".to_string())
                {
                    method.decorators.push("use_server_directive".to_string());
                }
            }
        }
    }
}

/// Collect ModuleInfo from all files in a directory using detailed AST extraction.
///
/// This provides the enriched function metadata (decorators, visibility, etc.)
/// needed for accurate dead code analysis with low false-positive rates.
fn collect_module_infos(
    path: &Path,
    language: Language,
    no_default_ignore: bool,
) -> Vec<(PathBuf, ModuleInfo)> {
    let mut module_infos = Vec::new();

    if path.is_file() {
        if let Ok(mut info) = extract_file(path, path.parent()) {
            if let Ok(source) = std::fs::read_to_string(path) {
                tag_directive_functions(&mut info, &source, path);
            }
            // Use filename only for single files (matches call graph convention)
            let rel_path = path
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| path.to_path_buf());
            module_infos.push((rel_path, info));
        }
    } else {
        let extensions: &[&str] = language.extensions();
        let mut file_count: usize = 0;
        let mut walker = ProjectWalker::new(path);
        if no_default_ignore {
            walker = walker.no_default_ignore();
        }
        for entry in walker.iter() {
            let file_path = entry.path();
            if file_path.is_file() {
                if let Some(ext_str) = file_path.extension().and_then(|e| e.to_str()) {
                    let dotted = format!(".{}", ext_str);
                    if extensions.contains(&dotted.as_str()) {
                        file_count += 1;
                        if file_count > MAX_FILES {
                            eprintln!(
                                "Warning: dead code scan truncated at {} files in {}",
                                MAX_FILES,
                                path.display()
                            );
                            break;
                        }
                        if let Ok(mut info) = extract_file(file_path, Some(path)) {
                            // Tag functions with framework directive from source
                            if let Ok(source) = std::fs::read_to_string(file_path) {
                                tag_directive_functions(&mut info, &source, file_path);
                            }
                            // Use relative path to match call graph edge convention
                            let rel_path = file_path
                                .strip_prefix(path)
                                .unwrap_or(file_path)
                                .to_path_buf();
                            module_infos.push((rel_path, info));
                        }
                    }
                }
            }
        }
    }

    module_infos
}

/// Collect ModuleInfo AND identifier reference counts in a single pass.
///
/// For each file, we parse once with tree-sitter and then run both:
/// - `extract_from_tree()` to get ModuleInfo (functions, classes, imports)
/// - `count_identifiers_in_tree()` to get identifier occurrence counts
///
/// The identifier counts are merged into a single project-wide HashMap.
pub(crate) fn collect_module_infos_with_refcounts(
    path: &Path,
    language: Language,
    no_default_ignore: bool,
) -> (Vec<(PathBuf, ModuleInfo)>, HashMap<String, usize>) {
    let mut module_infos = Vec::new();
    let mut merged_counts: HashMap<String, usize> = HashMap::new();

    if path.is_file() {
        if let Ok((tree, source, lang)) = parse_file(path) {
            // Extract ModuleInfo from the parsed tree
            if let Ok(mut info) = extract_from_tree(&tree, &source, lang, path, path.parent()) {
                tag_directive_functions(&mut info, &source, path);
                let rel_path = path
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| path.to_path_buf());
                module_infos.push((rel_path, info));
            }
            // Count identifiers from the same parsed tree
            let file_counts = count_identifiers_in_tree(&tree, source.as_bytes(), lang);
            for (name, count) in file_counts {
                *merged_counts.entry(name).or_insert(0) += count;
            }
        }
    } else {
        let extensions: &[&str] = language.extensions();
        let mut file_count: usize = 0;
        let mut walker = ProjectWalker::new(path);
        if no_default_ignore {
            walker = walker.no_default_ignore();
        }
        for entry in walker.iter() {
            let file_path = entry.path();
            if file_path.is_file() {
                if let Some(ext_str) = file_path.extension().and_then(|e| e.to_str()) {
                    let dotted = format!(".{}", ext_str);
                    if extensions.contains(&dotted.as_str()) {
                        file_count += 1;
                        if file_count > MAX_FILES {
                            eprintln!(
                                "Warning: born-dead scan truncated at {} files in {}",
                                MAX_FILES,
                                path.display()
                            );
                            break;
                        }
                        if let Ok((tree, source, lang)) = parse_file(file_path) {
                            // Extract ModuleInfo from the parsed tree
                            if let Ok(mut info) =
                                extract_from_tree(&tree, &source, lang, file_path, Some(path))
                            {
                                // Tag functions with framework directive while we have the source
                                tag_directive_functions(&mut info, &source, file_path);
                                let rel_path = file_path
                                    .strip_prefix(path)
                                    .unwrap_or(file_path)
                                    .to_path_buf();
                                module_infos.push((rel_path, info));
                            }
                            // Count identifiers from the same parsed tree
                            let file_counts =
                                count_identifiers_in_tree(&tree, source.as_bytes(), lang);
                            for (name, count) in file_counts {
                                *merged_counts.entry(name).or_insert(0) += count;
                            }
                        }
                    }
                }
            }
        }
    }

    (module_infos, merged_counts)
}

/// Wrapper struct for JSON output with truncation metadata.
#[derive(Serialize)]
struct DeadCodeOutput {
    #[serde(flatten)]
    report: DeadCodeReport,
    #[serde(skip_serializing_if = "is_false", default)]
    truncated: bool,
    total_count: usize,
    shown_count: usize,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Apply truncation to the report based on max_items.
fn apply_truncation(
    mut report: DeadCodeReport,
    max_items: usize,
) -> (DeadCodeReport, bool, usize, usize) {
    let total_count = report.dead_functions.len();

    if total_count > max_items {
        report.dead_functions.truncate(max_items);
        // Also truncate by_file to match
        let mut count = 0;
        let mut new_by_file = std::collections::HashMap::new();
        for (path, funcs) in report.by_file {
            let remaining = max_items - count;
            if remaining == 0 {
                break;
            }
            let to_take = funcs.len().min(remaining);
            let truncated_funcs: Vec<String> = funcs.into_iter().take(to_take).collect();
            count += truncated_funcs.len();
            new_by_file.insert(path, truncated_funcs);
        }
        report.by_file = new_by_file;
        (report, true, total_count, max_items)
    } else {
        (report, false, total_count, total_count)
    }
}

/// Format dead code report with optional truncation notice.
fn format_dead_code_text_truncated(
    report: &DeadCodeReport,
    truncated: bool,
    total_count: usize,
    shown_count: usize,
) -> String {
    use colored::Colorize;

    let mut output = String::new();

    output.push_str(&format!(
        "Dead Code Analysis\n\nDefinitely dead: {} / {} functions ({:.1}% dead)\n",
        report.total_dead.to_string().red(),
        report.total_functions,
        report.dead_percentage
    ));

    if report.total_possibly_dead > 0 {
        output.push_str(&format!(
            "Possibly dead (public but uncalled): {}\n",
            report.total_possibly_dead.to_string().yellow()
        ));
    }

    output.push('\n');

    if !report.by_file.is_empty() {
        output.push_str("Definitely dead:\n");
        for (file, funcs) in &report.by_file {
            output.push_str(&format!("{}\n", file.display().to_string().green()));
            for func in funcs {
                output.push_str(&format!("  - {}\n", func.red()));
            }
            output.push('\n');
        }
    }

    if truncated {
        output.push_str(&format!(
            "\n[{}: showing {} of {} dead functions]\n",
            "TRUNCATED".yellow(),
            shown_count,
            total_count
        ));
    }

    output
}
