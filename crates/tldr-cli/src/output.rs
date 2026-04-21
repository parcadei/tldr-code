//! Output formatting for CLI commands
//!
//! Supports three output formats:
//! - JSON: Structured output for programmatic use
//! - Text: Human-readable formatted output
//! - Compact: Minified JSON for piping
//!
//! # Mitigations Addressed
//! - M19: JSON output uses serde with preserve_order for consistent field order
//! - M20: Text output includes helpful context and suggestions

use std::io::{self, Write};
use std::path::Path;

use colored::Colorize;
use comfy_table::{presets::UTF8_FULL, Cell, Color, ContentArrangement, Table};
use serde::Serialize;

/// Compute the common directory prefix of a list of paths.
/// Returns the longest shared directory ancestor (never a partial component).
/// Returns empty path if paths share no common ancestor.
pub fn common_path_prefix(paths: &[&Path]) -> std::path::PathBuf {
    if paths.is_empty() {
        return std::path::PathBuf::new();
    }
    if paths.len() == 1 {
        return paths[0].parent().unwrap_or(Path::new("")).to_path_buf();
    }

    let first = paths[0];
    let components: Vec<_> = first.components().collect();
    let mut prefix_len = components.len();

    for path in &paths[1..] {
        let other: Vec<_> = path.components().collect();
        let mut match_len = 0;
        for (a, b) in components.iter().zip(other.iter()) {
            if a == b {
                match_len += 1;
            } else {
                break;
            }
        }
        prefix_len = prefix_len.min(match_len);
    }

    // Build the prefix path from matching components
    let mut result = std::path::PathBuf::new();
    for comp in components.iter().take(prefix_len) {
        result.push(comp);
    }
    result
}

/// Strip a common prefix from a path, returning a relative display string.
/// If stripping fails or results in empty, returns the original path display.
pub fn strip_prefix_display(path: &Path, prefix: &Path) -> String {
    if prefix.as_os_str().is_empty() {
        return path.display().to_string();
    }
    match path.strip_prefix(prefix) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
        _ => path.display().to_string(),
    }
}

/// Output format options
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    /// JSON output (default) - machine readable with consistent field order
    #[default]
    Json,
    /// Human-readable text output
    Text,
    /// Compact/minified JSON for piping
    Compact,
    /// SARIF format for IDE/CI integration (GitHub, VS Code, etc.)
    Sarif,
    /// DOT/Graphviz format for visualization
    Dot,
}

/// Output writer that handles different formats
pub struct OutputWriter {
    format: OutputFormat,
    quiet: bool,
}

impl OutputWriter {
    /// Create a new output writer with the specified format
    pub fn new(format: OutputFormat, quiet: bool) -> Self {
        Self { format, quiet }
    }

    /// Write a serializable value to stdout
    pub fn write<T: Serialize>(&self, value: &T) -> io::Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        match self.format {
            OutputFormat::Json | OutputFormat::Sarif => {
                // SARIF is handled by specialized methods; generic write uses JSON
                serde_json::to_writer_pretty(&mut handle, value)?;
                writeln!(handle)?;
            }
            OutputFormat::Compact => {
                serde_json::to_writer(&mut handle, value)?;
                writeln!(handle)?;
            }
            OutputFormat::Text | OutputFormat::Dot => {
                // Text/DOT format is handled by specialized methods
                serde_json::to_writer_pretty(&mut handle, value)?;
                writeln!(handle)?;
            }
        }

        Ok(())
    }

    /// Write a string directly (for text format)
    pub fn write_text(&self, text: &str) -> io::Result<()> {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        writeln!(handle, "{}", text)?;
        Ok(())
    }

    /// Write progress message (only if not quiet)
    pub fn progress(&self, message: &str) {
        if !self.quiet {
            eprintln!("{}", message.dimmed());
        }
    }

    /// Check if we should use text format
    pub fn is_text(&self) -> bool {
        matches!(self.format, OutputFormat::Text)
    }

    /// Check if we should use JSON format
    #[allow(dead_code)]
    pub fn is_json(&self) -> bool {
        matches!(
            self.format,
            OutputFormat::Json | OutputFormat::Compact | OutputFormat::Sarif
        )
    }

    /// Check if we should use DOT format
    pub fn is_dot(&self) -> bool {
        matches!(self.format, OutputFormat::Dot)
    }
}

// =============================================================================
// Text formatters for specific types
// =============================================================================

/// Format a file tree for text output
pub fn format_file_tree_text(tree: &tldr_core::FileTree, indent: usize) -> String {
    let mut output = String::new();
    format_tree_node(tree, &mut output, indent, "");
    output
}

fn format_tree_node(tree: &tldr_core::FileTree, output: &mut String, indent: usize, prefix: &str) {
    let indent_str = "  ".repeat(indent);
    // Use plain text icons for non-emoji terminals
    let icon_plain = match tree.node_type {
        tldr_core::NodeType::Dir => "[D]".yellow().to_string(),
        tldr_core::NodeType::File => "[F]".blue().to_string(),
    };

    output.push_str(&format!(
        "{}{}{} {}\n",
        prefix, indent_str, icon_plain, tree.name
    ));

    for (i, child) in tree.children.iter().enumerate() {
        let is_last = i == tree.children.len() - 1;
        let new_prefix = if is_last { "`-- " } else { "|-- " };
        let cont_prefix = if is_last { "    " } else { "|   " };
        format_tree_node(
            child,
            output,
            0,
            &format!("{}{}{}", prefix, cont_prefix, new_prefix),
        );
    }
}

/// Format code structure for text output
pub fn format_structure_text(structure: &tldr_core::CodeStructure) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "{} ({} files)\n",
        structure.root.display().to_string().bold(),
        structure.files.len()
    ));
    output.push_str(&format!(
        "Language: {}\n\n",
        format!("{:?}", structure.language).cyan()
    ));

    // Use root as prefix for relative path display
    let prefix = &structure.root;

    for file in &structure.files {
        let rel = strip_prefix_display(&file.path, prefix);
        output.push_str(&format!("{}\n", rel.green()));

        if !file.functions.is_empty() {
            output.push_str("  Functions:\n");
            for func in &file.functions {
                output.push_str(&format!("    - {}\n", func));
            }
        }

        if !file.classes.is_empty() {
            output.push_str("  Classes:\n");
            for class in &file.classes {
                output.push_str(&format!("    - {}\n", class));
            }
        }

        output.push('\n');
    }

    output
}

/// Format imports for text output
///
/// Groups imports by module for compact, readable output:
/// ```text
/// file.py (12 imports)
///
///   from .exceptions: Abort, BadParameter, MissingParameter, UsageError
///   from .core: Command, Group, Context
///   import os, sys, typing
/// ```
pub fn format_imports_text(imports: &[tldr_core::types::ImportInfo]) -> String {
    use std::collections::BTreeMap;

    let mut output = String::new();

    if imports.is_empty() {
        output.push_str("No imports found.\n");
        return output;
    }

    // Group: from-imports by module, bare imports separately
    let mut from_groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut bare_imports: Vec<String> = Vec::new();

    for imp in imports {
        if imp.is_from && !imp.names.is_empty() {
            let names = from_groups.entry(&imp.module).or_default();
            for name in &imp.names {
                names.push(name);
            }
        } else if let Some(alias) = &imp.alias {
            bare_imports.push(format!("{} as {}", imp.module, alias));
        } else {
            bare_imports.push(imp.module.clone());
        }
    }

    // From-imports grouped by module
    for (module, names) in &from_groups {
        output.push_str(&format!(
            "from {}: {}\n",
            module.cyan(),
            names.join(", ").green(),
        ));
    }

    // Bare imports on one line
    if !bare_imports.is_empty() {
        if !from_groups.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("import {}\n", bare_imports.join(", ").cyan()));
    }

    output
}

/// Format importers report for text output
///
/// Shows each importing file with line number and the import statement.
/// Strips the common path prefix for token efficiency.
/// ```text
///   click/core.py:3            import os
///   click/_compat.py:1         import os
///   click/utils.py:5           from os.path import join
/// ```
pub fn format_importers_text(report: &tldr_core::types::ImportersReport) -> String {
    let mut output = String::new();

    if report.importers.is_empty() {
        output.push_str("No files import this module.\n");
        return output;
    }

    // Compute common path prefix for relative display
    let paths: Vec<&Path> = report.importers.iter().map(|i| i.file.as_path()).collect();
    let prefix = common_path_prefix(&paths);

    // Find max path:line width for alignment (using stripped paths)
    let max_loc_width = report
        .importers
        .iter()
        .map(|i| format!("{}:{}", strip_prefix_display(&i.file, &prefix), i.line).len())
        .max()
        .unwrap_or(20);

    for imp in &report.importers {
        let rel_path = strip_prefix_display(&imp.file, &prefix);
        let loc = format!("{}:{}", rel_path, imp.line);
        output.push_str(&format!(
            "  {:<width$}  {}\n",
            loc.green(),
            imp.import_statement.dimmed(),
            width = max_loc_width,
        ));
    }

    output
}

/// Format CFG info for text output
pub fn format_cfg_text(cfg: &tldr_core::CfgInfo) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "CFG for {} (complexity: {})\n\n",
        cfg.function.bold().cyan(),
        cfg.cyclomatic_complexity.to_string().yellow()
    ));

    // Create blocks table
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Block").fg(Color::Cyan),
            Cell::new("Type").fg(Color::Cyan),
            Cell::new("Lines").fg(Color::Cyan),
            Cell::new("Calls").fg(Color::Cyan),
        ]);

    for block in &cfg.blocks {
        table.add_row(vec![
            Cell::new(block.id),
            Cell::new(format!("{:?}", block.block_type)),
            Cell::new(format!("{}-{}", block.lines.0, block.lines.1)),
            Cell::new(block.calls.join(", ")),
        ]);
    }

    output.push_str(&table.to_string());
    output.push_str("\n\nEdges:\n");

    for edge in &cfg.edges {
        let edge_str = match edge.edge_type {
            tldr_core::EdgeType::True => format!("{} -> {} (true)", edge.from, edge.to).green(),
            tldr_core::EdgeType::False => format!("{} -> {} (false)", edge.from, edge.to).red(),
            tldr_core::EdgeType::Unconditional => format!("{} -> {}", edge.from, edge.to).normal(),
            tldr_core::EdgeType::BackEdge => {
                format!("{} -> {} (back)", edge.from, edge.to).yellow()
            }
            _ => format!("{} -> {} ({:?})", edge.from, edge.to, edge.edge_type).normal(),
        };
        output.push_str(&format!("  {}\n", edge_str));
    }

    output
}

/// Format DFG info for text output
pub fn format_dfg_text(dfg: &tldr_core::DfgInfo) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "DFG for {} ({} variables)\n\n",
        dfg.function.bold().cyan(),
        dfg.variables.len().to_string().yellow()
    ));

    output.push_str("Variables: ");
    output.push_str(&dfg.variables.join(", "));
    output.push_str("\n\n");

    // Create refs table
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Var").fg(Color::Cyan),
            Cell::new("Type").fg(Color::Cyan),
            Cell::new("Line").fg(Color::Cyan),
            Cell::new("Col").fg(Color::Cyan),
        ]);

    for var_ref in &dfg.refs {
        let type_str = match var_ref.ref_type {
            tldr_core::RefType::Definition => "def",
            tldr_core::RefType::Update => "upd",
            tldr_core::RefType::Use => "use",
        };
        table.add_row(vec![
            Cell::new(&var_ref.name),
            Cell::new(type_str),
            Cell::new(var_ref.line),
            Cell::new(var_ref.column),
        ]);
    }

    output.push_str(&table.to_string());
    output
}

/// Collect all file paths from a caller tree recursively
fn collect_caller_tree_paths<'a>(tree: &'a tldr_core::CallerTree, paths: &mut Vec<&'a Path>) {
    paths.push(tree.file.as_path());
    for caller in &tree.callers {
        collect_caller_tree_paths(caller, paths);
    }
}

/// Format impact report for text output
pub fn format_impact_text(report: &tldr_core::ImpactReport, type_aware: bool) -> String {
    let mut output = String::new();

    let type_aware_suffix = if type_aware { " (type-aware)" } else { "" };
    output.push_str(&format!(
        "Impact Analysis{} ({} targets)\n\n",
        type_aware_suffix,
        report.total_targets.to_string().yellow()
    ));

    // Show type resolution stats if enabled
    if let Some(ref stats) = report.type_resolution {
        if stats.enabled {
            output.push_str(&stats.summary());
            output.push_str("\n\n");
        }
    }

    // Collect all paths from all trees for common prefix
    let mut all_paths = Vec::new();
    for tree in report.targets.values() {
        collect_caller_tree_paths(tree, &mut all_paths);
    }
    let prefix = common_path_prefix(&all_paths);

    for (key, tree) in &report.targets {
        output.push_str(&format!("{}\n", key.bold().cyan()));
        format_caller_tree(tree, &mut output, 1, type_aware, &prefix);
        output.push('\n');
    }

    output
}

fn format_caller_tree(
    tree: &tldr_core::CallerTree,
    output: &mut String,
    depth: usize,
    type_aware: bool,
    prefix: &Path,
) {
    let indent = "  ".repeat(depth);
    let file_str = strip_prefix_display(&tree.file, prefix);

    // Show confidence if type-aware and available
    let confidence_str = if type_aware {
        if let Some(confidence) = &tree.confidence {
            format!(" [{}]", confidence)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    output.push_str(&format!(
        "{}{}:{} ({} callers){}\n",
        indent,
        file_str.dimmed(),
        tree.function.green(),
        tree.caller_count,
        confidence_str
    ));

    if tree.truncated {
        output.push_str(&format!("{}  [truncated - cycle detected]\n", indent));
    }

    if let Some(note) = &tree.note {
        output.push_str(&format!("{}  Note: {}\n", indent, note.dimmed()));
    }

    for caller in &tree.callers {
        format_caller_tree(caller, output, depth + 1, type_aware, prefix);
    }
}

/// Format dead code report for text output
pub fn format_dead_code_text(report: &tldr_core::DeadCodeReport) -> String {
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
        // Compute common prefix for relative display
        let paths: Vec<&Path> = report.by_file.keys().map(|p| p.as_path()).collect();
        let prefix = common_path_prefix(&paths);

        output.push_str("Definitely dead:\n");
        for (file, funcs) in &report.by_file {
            let rel = strip_prefix_display(file, &prefix);
            output.push_str(&format!("{}\n", rel.green()));
            for func in funcs {
                output.push_str(&format!("  - {}\n", func.red()));
            }
            output.push('\n');
        }
    }

    output
}

/// Format complexity metrics for text output
///
/// Compact single-function report:
/// ```text
/// Complexity: process_request
///   Cyclomatic:    12
///   Cognitive:     8
///   Nesting depth: 4
///   Lines of code: 45
/// ```
pub fn format_complexity_text(metrics: &tldr_core::types::ComplexityMetrics) -> String {
    let mut output = String::new();

    output.push_str(&format!("Complexity: {}\n", metrics.function.bold().cyan()));
    output.push_str(&format!("  Cyclomatic:    {}\n", metrics.cyclomatic));
    output.push_str(&format!("  Cognitive:     {}\n", metrics.cognitive));
    output.push_str(&format!("  Nesting depth: {}\n", metrics.nesting_depth));
    output.push_str(&format!("  Lines of code: {}\n", metrics.lines_of_code));

    output
}

/// Format cognitive complexity report for text output
///
/// Shows top-N functions ranked by cognitive complexity, with threshold violations highlighted.
/// Strips common path prefix for compact display.
///
/// ```text
/// Cognitive Complexity (12 functions, 3 violations)
///
///  #  Score  Nest  Status     Function                     File
///  1     18     4  SEVERE     parse_args                   core.py:142
///  2     15     3  VIOLATION  make_context                 core.py:1200
///  3     12     2  ok         invoke                       core.py:987
/// ```
pub fn format_cognitive_text(report: &tldr_core::metrics::CognitiveReport) -> String {
    let mut output = String::new();

    let violation_count = report.violations.len();
    output.push_str(&format!(
        "Cognitive Complexity ({} functions, {} violations)\n\n",
        report.summary.total_functions,
        if violation_count > 0 {
            violation_count.to_string().red().to_string()
        } else {
            "0".green().to_string()
        }
    ));

    if report.functions.is_empty() {
        output.push_str("  No functions found.\n");
        return output;
    }

    // Compute common path prefix for relative display
    // Use parent directories so single-file reports still get path stripping
    let parents: Vec<&Path> = report
        .functions
        .iter()
        .filter_map(|f| Path::new(f.file.as_str()).parent())
        .collect();
    let prefix = if parents.is_empty() {
        std::path::PathBuf::new()
    } else {
        common_path_prefix(&parents)
    };

    // Header
    output.push_str(&format!(
        " {:>3}  {:>5}  {:>4}  {:<9}  {:<28}  {}\n",
        "#", "Score", "Nest", "Status", "Function", "File"
    ));

    for (i, f) in report.functions.iter().enumerate() {
        let rel = strip_prefix_display(Path::new(&f.file), &prefix);
        let status = match f.threshold_status {
            tldr_core::metrics::CognitiveThresholdStatus::Severe => {
                "SEVERE".red().bold().to_string()
            }
            tldr_core::metrics::CognitiveThresholdStatus::Violation => {
                "VIOLATION".yellow().to_string()
            }
            _ => "ok".green().to_string(),
        };

        // Truncate function name to 28 chars
        let name = if f.name.len() > 28 {
            format!("{}...", &f.name[..25])
        } else {
            f.name.clone()
        };

        output.push_str(&format!(
            " {:>3}  {:>5}  {:>4}  {:<9}  {:<28}  {}:{}\n",
            i + 1,
            f.cognitive,
            f.max_nesting,
            status,
            name,
            rel,
            f.line
        ));
    }

    // Summary
    output.push_str(&format!(
        "\nSummary: avg={:.1}, max={}, compliance={:.1}%\n",
        report.summary.avg_cognitive, report.summary.max_cognitive, report.summary.compliance_rate
    ));

    output
}

/// Format maintainability index report for text output
///
/// Shows per-file MI scores sorted worst-first, with grade distribution summary.
/// Top 30 files shown by default (worst MI first).
///
/// ```text
/// Maintainability Index (47 files, avg MI=42.3)
///
/// Grade distribution: A=5 B=12 C=18 D=10 F=2
///
///  #   MI  Grade  LOC  AvgCC  File
///  1  10.8    F   612   18.2  core.py
///  2  22.1    F   445   12.6  parser.py
///  3  28.4    D   203    8.1  utils.py
/// ```
pub fn format_maintainability_text(
    report: &tldr_core::quality::maintainability::MaintainabilityReport,
) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Maintainability Index ({} files, avg MI={:.1})\n\n",
        report.summary.files_analyzed, report.summary.average_mi
    ));

    // Grade distribution
    let grades = ['A', 'B', 'C', 'D', 'F'];
    let mut grade_parts = Vec::new();
    for g in &grades {
        let count = report.summary.by_grade.get(g).unwrap_or(&0);
        if *count > 0 {
            grade_parts.push(format!("{}={}", g, count));
        }
    }
    output.push_str(&format!(
        "Grade distribution: {}\n\n",
        grade_parts.join(" ")
    ));

    if report.files.is_empty() {
        output.push_str("  No files analyzed.\n");
        return output;
    }

    // Sort files by MI ascending (worst first) — clone since report is borrowed
    let mut files: Vec<_> = report.files.iter().collect();
    files.sort_by(|a, b| a.mi.partial_cmp(&b.mi).unwrap_or(std::cmp::Ordering::Equal));

    // Compute common path prefix
    let paths: Vec<&Path> = files.iter().filter_map(|f| f.path.parent()).collect();
    let prefix = common_path_prefix(&paths);

    // Header
    output.push_str(&format!(
        " {:>3}  {:>5}  {:>5}  {:>4}  {:>5}  {}\n",
        "#", "MI", "Grade", "LOC", "AvgCC", "File"
    ));

    // Show top 30
    let limit = files.len().min(30);
    for (i, f) in files.iter().take(limit).enumerate() {
        let rel = strip_prefix_display(&f.path, &prefix);
        let grade_str = match f.grade {
            'F' => format!("{}", f.grade).red().bold().to_string(),
            'D' => format!("{}", f.grade).yellow().to_string(),
            _ => format!("{}", f.grade),
        };

        output.push_str(&format!(
            " {:>3}  {:>5.1}  {:>5}  {:>4}  {:>5.1}  {}\n",
            i + 1,
            f.mi,
            grade_str,
            f.loc,
            f.avg_complexity,
            rel
        ));
    }

    if files.len() > limit {
        output.push_str(&format!("\n  ... and {} more files\n", files.len() - limit));
    }

    output
}

/// Format search matches for text output
pub fn format_search_text(matches: &[tldr_core::SearchMatch]) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Found {} matches\n\n",
        matches.len().to_string().yellow()
    ));

    // Compute common prefix for relative display
    let paths: Vec<&Path> = matches.iter().map(|m| m.file.as_path()).collect();
    let prefix = common_path_prefix(&paths);

    for m in matches {
        let rel = strip_prefix_display(&m.file, &prefix);
        output.push_str(&format!(
            "{}:{}: {}\n",
            rel.green(),
            m.line.to_string().cyan(),
            m.content.trim()
        ));

        if let Some(context) = &m.context {
            for line in context {
                output.push_str(&format!("  {}\n", line.dimmed()));
            }
        }
    }

    output
}

/// Format enriched search report for text output.
///
/// Each result is a compact card showing:
/// - Function/class name, file, line range, and score
/// - Signature (definition line)
/// - Callers and callees (if available)
pub fn format_enriched_search_text(report: &tldr_core::EnrichedSearchReport) -> String {
    let mut output = String::new();

    output.push_str(&format!("query: \"{}\"\n", report.query));
    output.push_str(&format!(
        "{} results from {} files ({})\n\n",
        report.results.len(),
        report.total_files_searched,
        report.search_mode
    ));

    if report.results.is_empty() {
        output.push_str("  No results found.\n");
        return output;
    }

    // Compute common path prefix for compact display
    let paths: Vec<&Path> = report.results.iter().map(|r| r.file.as_path()).collect();
    let prefix = common_path_prefix(&paths);

    for (i, result) in report.results.iter().enumerate() {
        let rel = strip_prefix_display(&result.file, &prefix);
        let line_range = format!("{}-{}", result.line_range.0, result.line_range.1);

        // Line 1: index. kind:name (file:lines) [score]
        let kind_prefix = match result.kind.as_str() {
            "function" => "fn ",
            "method" => "method ",
            "class" => "class ",
            "struct" => "struct ",
            "module" => "mod ",
            _ => "",
        };
        output.push_str(&format!(
            "{}. {}{} ({}:{}) [{:.2}]\n",
            i + 1,
            kind_prefix,
            result.name,
            rel,
            line_range,
            result.score
        ));

        // Line 2: signature
        if !result.signature.is_empty() {
            output.push_str(&format!("   {}\n", result.signature));
        }

        // Line 3: callers (if any)
        if !result.callers.is_empty() {
            let callers_str = format_name_list(&result.callers, 5);
            output.push_str(&format!("   Called by: {}\n", callers_str));
        }

        // Line 4: callees (if any)
        if !result.callees.is_empty() {
            let callees_str = format_name_list(&result.callees, 5);
            output.push_str(&format!("   Calls: {}\n", callees_str));
        }

        // Code preview (indented, skip first line if it matches signature)
        if !result.preview.is_empty() && result.kind != "module" {
            let preview_lines: Vec<&str> = result.preview.lines().collect();
            // Skip the first line if it matches the signature (already shown above)
            let start =
                if preview_lines.first().map(|l| l.trim()) == Some(result.signature.as_str()) {
                    1
                } else {
                    0
                };
            if start < preview_lines.len() {
                output.push_str("   ---\n");
                for line in &preview_lines[start..preview_lines.len().min(start + 4)] {
                    output.push_str(&format!("   {}\n", line));
                }
            }
        }

        // Blank line between cards
        if i < report.results.len() - 1 {
            output.push('\n');
        }
    }

    output
}

/// Format a list of names, showing up to `max` items then "... and N more".
fn format_name_list(names: &[String], max: usize) -> String {
    if names.len() <= max {
        names.join(", ")
    } else {
        let shown: Vec<&str> = names[..max].iter().map(|s| s.as_str()).collect();
        format!("{}, ... and {} more", shown.join(", "), names.len() - max)
    }
}

/// Format smells report for text output
pub fn format_smells_text(report: &tldr_core::SmellsReport) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Code Smells Report ({} issues)\n\n",
        report.smells.len().to_string().yellow()
    ));

    if report.smells.is_empty() {
        output.push_str("  No code smells detected.\n");
        return output;
    }

    // Compute common path prefix for relative display
    let paths: Vec<&Path> = report.smells.iter().map(|s| s.file.as_path()).collect();
    let prefix = if paths.is_empty() {
        std::path::PathBuf::new()
    } else {
        common_path_prefix(&paths)
    };

    // Header
    output.push_str(&format!(
        " {:>3}  {:>3}  {:<20}  {:<28}  {}\n",
        "#", "Sev", "Type", "Name", "File:Line"
    ));

    for (i, smell) in report.smells.iter().enumerate() {
        // Severity coloring
        let sev_str = match smell.severity {
            3 => smell.severity.to_string().red(),
            2 => smell.severity.to_string().yellow(),
            _ => smell.severity.to_string().white(),
        }
        .to_string();

        // Smell type coloring
        let type_str = {
            let base = format!("{}", smell.smell_type);
            let colored = match smell.smell_type {
                tldr_core::SmellType::GodClass => base.red(),
                tldr_core::SmellType::LongMethod => base.yellow(),
                tldr_core::SmellType::LongParameterList => base.magenta(),
                tldr_core::SmellType::LowCohesion => base.yellow(),
                tldr_core::SmellType::TightCoupling => base.red(),
                tldr_core::SmellType::DeadCode => base.dimmed(),
                tldr_core::SmellType::CodeClone => base.cyan(),
                tldr_core::SmellType::HighCognitiveComplexity => base.red(),
                tldr_core::SmellType::DeepNesting => base.yellow(),
                tldr_core::SmellType::DataClass => base.cyan(),
                tldr_core::SmellType::LazyElement => base.dimmed(),
                tldr_core::SmellType::MessageChain => base.magenta(),
                tldr_core::SmellType::PrimitiveObsession => base.cyan(),
                tldr_core::SmellType::FeatureEnvy => base.yellow(),
                tldr_core::SmellType::MiddleMan => base.yellow(),
                tldr_core::SmellType::RefusedBequest => base.magenta(),
                tldr_core::SmellType::InappropriateIntimacy => base.red(),
                tldr_core::SmellType::DataClumps => base.white(),
            };
            colored.to_string()
        };

        // Truncate name to 28 chars
        let name = if smell.name.len() > 28 {
            format!("{}...", &smell.name[..25])
        } else {
            smell.name.clone()
        };

        // Strip path prefix
        let rel_file = strip_prefix_display(&smell.file, &prefix);

        output.push_str(&format!(
            " {:>3}  {:>3}  {:<20}  {:<28}  {}:{}\n",
            i + 1,
            sev_str,
            type_str,
            name,
            rel_file,
            smell.line
        ));
    }

    // Summary with per-type counts
    output.push('\n');

    let sev3 = report.smells.iter().filter(|s| s.severity == 3).count();
    let sev2 = report.smells.iter().filter(|s| s.severity == 2).count();
    let sev1 = report.smells.iter().filter(|s| s.severity == 1).count();
    let unique_files = report.by_file.len();
    output.push_str(&format!(
        "Summary: {} smells found ({} {}, {} {}, {} {}) across {} files\n",
        report.smells.len(),
        sev3,
        "sev-3".red(),
        sev2,
        "sev-2".yellow(),
        sev1,
        "sev-1",
        unique_files,
    ));

    // Per-type breakdown
    let mut type_counts: Vec<(String, usize)> = report
        .summary
        .by_type
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    type_counts.sort_by(|a, b| b.1.cmp(&a.1));
    let breakdown: Vec<String> = type_counts
        .iter()
        .map(|(name, count)| format!("{}: {}", name, count))
        .collect();
    output.push_str(&format!("  {}\n", breakdown.join(", ")));

    output
}

/// Format secrets report for text output
pub fn format_secrets_text(report: &tldr_core::SecretsReport) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "Secrets Scan ({} findings, {} files scanned)\n\n",
        report.findings.len().to_string().yellow(),
        report.files_scanned
    ));

    if report.findings.is_empty() {
        output.push_str("  No secrets detected.\n");
        return output;
    }

    // Compute common path prefix for relative display
    let paths: Vec<&Path> = report.findings.iter().map(|f| f.file.as_path()).collect();
    let prefix = if paths.is_empty() {
        std::path::PathBuf::new()
    } else {
        common_path_prefix(&paths)
    };

    // Header
    output.push_str(&format!(
        " {:<8}  {:<14}  {:<40}  {:>5}  {}\n",
        "Severity", "Pattern", "File", "Line", "Value"
    ));

    for finding in &report.findings {
        let sev_str = match finding.severity {
            tldr_core::Severity::Critical => finding.severity.to_string().red(),
            tldr_core::Severity::High => finding.severity.to_string().red(),
            tldr_core::Severity::Medium => finding.severity.to_string().yellow(),
            tldr_core::Severity::Low => finding.severity.to_string().white(),
        }
        .to_string();

        let rel_file = strip_prefix_display(&finding.file, &prefix);

        // Truncate file path to 40 chars
        let file_display = if rel_file.len() > 40 {
            format!("...{}", &rel_file[rel_file.len() - 37..])
        } else {
            rel_file
        };

        output.push_str(&format!(
            " {:<8}  {:<14}  {:<40}  {:>5}  {}\n",
            sev_str, finding.pattern, file_display, finding.line, finding.masked_value
        ));
    }

    // Summary by severity
    output.push('\n');
    let critical = report
        .findings
        .iter()
        .filter(|f| f.severity == tldr_core::Severity::Critical)
        .count();
    let high = report
        .findings
        .iter()
        .filter(|f| f.severity == tldr_core::Severity::High)
        .count();
    let medium = report
        .findings
        .iter()
        .filter(|f| f.severity == tldr_core::Severity::Medium)
        .count();
    let low = report
        .findings
        .iter()
        .filter(|f| f.severity == tldr_core::Severity::Low)
        .count();
    let mut parts = Vec::new();
    if critical > 0 {
        parts.push(format!("{} {}", critical, "critical".red()));
    }
    if high > 0 {
        parts.push(format!("{} {}", high, "high".red()));
    }
    if medium > 0 {
        parts.push(format!("{} {}", medium, "medium".yellow()));
    }
    if low > 0 {
        parts.push(format!("{} {}", low, "low"));
    }
    output.push_str(&format!("Summary: {}\n", parts.join(", ")));

    output
}

/// Format whatbreaks report for text output
///
/// Follows spec text output format:
/// - Header with target and detected type
/// - Summary statistics (callers, importers, tests)
/// - Sub-analysis status (success/error/skipped)
/// - Total elapsed time
///
/// # Example output
///
/// ```text
/// What Breaks: user_service.py (file)
/// ==================================================
/// Direct callers:     N/A
/// Transitive callers: N/A
/// Importing modules:  2 files
/// Affected tests:     2 test files
///
/// Sub-analyses:
///   [OK]   importers        (45ms)
///   [OK]   change-impact    (121ms)
///
/// Elapsed: 166ms
/// ```
pub fn format_whatbreaks_text(
    report: &tldr_core::analysis::whatbreaks::WhatbreaksReport,
) -> String {
    let mut output = String::new();

    // Header with target and type
    output.push_str(&format!(
        "What Breaks: {} ({})\n",
        report.target.bold().cyan(),
        report.target_type.to_string().yellow()
    ));
    output.push('\n');

    // Summary statistics
    let summary = &report.summary;

    if summary.direct_caller_count > 0
        || report.target_type == tldr_core::analysis::whatbreaks::TargetType::Function
    {
        output.push_str(&format!(
            "Direct callers:     {}\n",
            if summary.direct_caller_count > 0 {
                summary.direct_caller_count.to_string().green().to_string()
            } else {
                "0".to_string()
            }
        ));
        output.push_str(&format!(
            "Transitive callers: {}\n",
            if summary.transitive_caller_count > 0 {
                summary
                    .transitive_caller_count
                    .to_string()
                    .green()
                    .to_string()
            } else {
                "0".to_string()
            }
        ));
    }

    if summary.importer_count > 0
        || report.target_type != tldr_core::analysis::whatbreaks::TargetType::Function
    {
        output.push_str(&format!(
            "Importing modules:  {}\n",
            if summary.importer_count > 0 {
                format!("{} files", summary.importer_count)
                    .green()
                    .to_string()
            } else {
                "0 files".to_string()
            }
        ));
    }

    if summary.affected_test_count > 0
        || report.target_type == tldr_core::analysis::whatbreaks::TargetType::File
    {
        output.push_str(&format!(
            "Affected tests:     {}\n",
            if summary.affected_test_count > 0 {
                format!("{} test files", summary.affected_test_count)
                    .yellow()
                    .to_string()
            } else {
                "0 test files".to_string()
            }
        ));
    }

    output.push('\n');

    // Sub-analyses status (only show errors/warnings, skip timing noise)
    let has_errors = report
        .sub_results
        .values()
        .any(|r| r.error.is_some() || !r.warnings.is_empty());
    if has_errors {
        output.push_str("Issues:\n");

        let mut sub_results: Vec<_> = report.sub_results.iter().collect();
        sub_results.sort_by_key(|(name, _)| *name);

        for (name, result) in sub_results {
            if let Some(error) = &result.error {
                output.push_str(&format!("  {} error: {}\n", name, error.red()));
            }
            for warning in &result.warnings {
                output.push_str(&format!("  {} warning: {}\n", name, warning.yellow()));
            }
        }
    }

    output
}

/// Format hubs report for text output
///
/// Plain text format (no box-drawing tables) for token efficiency:
/// ```text
/// Hub Detection (5 hubs / 120 nodes)
///
///  #  Risk      Function              File                Score  In  Out
///  1  CRITICAL  process_request       server/handler.py   0.92   15   8
///  2  HIGH      validate_input        core/validator.py   0.71   10   5
/// ```
pub fn format_hubs_text(report: &tldr_core::analysis::hubs::HubReport) -> String {
    let mut output = String::new();

    // Compact header
    output.push_str(&format!(
        "Hub Detection ({} hubs / {} nodes)\n\n",
        report.hub_count.to_string().yellow(),
        report.total_nodes,
    ));

    // Handle empty results
    if report.hubs.is_empty() {
        output.push_str("No hubs found.\n");
        return output;
    }

    // Compute common path prefix for relative display
    let paths: Vec<&Path> = report.hubs.iter().map(|h| h.file.as_path()).collect();
    let prefix = common_path_prefix(&paths);

    // Compute column widths
    let max_func = report
        .hubs
        .iter()
        .map(|h| h.name.len())
        .max()
        .unwrap_or(8)
        .max(8);
    let max_file = report
        .hubs
        .iter()
        .map(|h| strip_prefix_display(&h.file, &prefix).len())
        .max()
        .unwrap_or(4)
        .max(4);

    // Header line
    output.push_str(&format!(
        " {:<3} {:<8}  {:<width_f$}  {:<width_p$}  {:>5}  {:>3}  {:>3}\n",
        "#",
        "Risk",
        "Function",
        "File",
        "Score",
        "In",
        "Out",
        width_f = max_func,
        width_p = max_file,
    ));

    for (i, hub) in report.hubs.iter().enumerate() {
        let risk_str = format!("{}", hub.risk_level).to_uppercase();
        let rel_file = strip_prefix_display(&hub.file, &prefix);

        output.push_str(&format!(
            " {:<3} {:<8}  {:<width_f$}  {:<width_p$}  {:>5.3}  {:>3}  {:>3}\n",
            i + 1,
            risk_str,
            hub.name,
            rel_file,
            hub.composite_score,
            hub.callers_count,
            hub.callees_count,
            width_f = max_func,
            width_p = max_file,
        ));
    }

    output
}

/// Format change impact report for text output
///
/// Shows changed files, affected tests, and detection method.
/// Session 6 Phase 1: Basic text output format.
pub fn format_change_impact_text(report: &tldr_core::ChangeImpactReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&"Change Impact Analysis\n".bold().to_string());
    output.push_str("======================\n\n");

    // Detection method
    output.push_str(&format!("Detection: {}\n", report.detection_method.cyan()));

    // Changed files section
    output.push_str(&format!(
        "Changed: {} files\n\n",
        report.changed_files.len().to_string().yellow()
    ));

    if !report.changed_files.is_empty() {
        output.push_str(&"Changed Files:\n".bold().to_string());
        for file in &report.changed_files {
            output.push_str(&format!("  {}\n", file.display().to_string().green()));
        }
        output.push('\n');
    }

    // Affected tests section with function granularity
    let test_func_count = report.affected_test_functions.len();
    output.push_str(&format!(
        "Affected Tests: {} files, {} functions\n",
        report.affected_tests.len().to_string().yellow(),
        test_func_count.to_string().yellow()
    ));

    if !report.affected_tests.is_empty() {
        for test in &report.affected_tests {
            output.push_str(&format!("  {}\n", test.display().to_string().cyan()));
            // Show test functions for this file
            for tf in &report.affected_test_functions {
                if tf.file == *test {
                    let func_name = if let Some(ref class) = tf.class {
                        format!("{}::{}", class, tf.function)
                    } else {
                        tf.function.clone()
                    };
                    output.push_str(&format!("    - {} (line {})\n", func_name.green(), tf.line));
                }
            }
        }
        output.push('\n');
    } else {
        output.push_str("  No tests affected.\n\n");
    }

    // Affected functions section
    if !report.affected_functions.is_empty() {
        output.push_str(&format!(
            "Affected Functions: {}\n",
            report.affected_functions.len().to_string().yellow()
        ));
        for func in &report.affected_functions {
            output.push_str(&format!(
                "  {} ({})\n",
                func.name.green(),
                func.file.display().to_string().dimmed()
            ));
        }
        output.push('\n');
    }

    // Metadata
    if let Some(ref metadata) = report.metadata {
        output.push_str(&format!(
            "Call Graph: {} edges\n",
            metadata.call_graph_edges
        ));
        output.push_str(&format!(
            "Traversal Depth: {}\n",
            metadata.analysis_depth.unwrap_or(0)
        ));
    }

    output
}

/// Format diagnostics report for compact, token-efficient text output
///
/// R1: One-line summary header, one-line-per-diagnostic, no decorations, no ANSI colors
/// R2: Strips absolute paths to relative using common_path_prefix
/// R3: Truncates multi-line messages (pyright nested explanations) to first line
pub fn format_diagnostics_text(
    report: &tldr_core::diagnostics::DiagnosticsReport,
    filtered_count: usize,
) -> String {
    let mut output = String::new();

    // --- R1: Compact one-line summary header ---
    // Format: "pyright + ruff | 42 files | 3 errors, 1 warning"
    let tool_names: Vec<&str> = report.tools_run.iter().map(|t| t.name.as_str()).collect();
    let tools_part = tool_names.join(" + ");

    let summary = &report.summary;
    let mut counts: Vec<String> = Vec::new();
    if summary.errors > 0 {
        counts.push(format!(
            "{} {}",
            summary.errors,
            if summary.errors == 1 {
                "error"
            } else {
                "errors"
            }
        ));
    }
    if summary.warnings > 0 {
        counts.push(format!(
            "{} {}",
            summary.warnings,
            if summary.warnings == 1 {
                "warning"
            } else {
                "warnings"
            }
        ));
    }
    if summary.info > 0 {
        counts.push(format!(
            "{} {}",
            summary.info,
            if summary.info == 1 { "info" } else { "infos" }
        ));
    }
    if summary.hints > 0 {
        counts.push(format!(
            "{} {}",
            summary.hints,
            if summary.hints == 1 { "hint" } else { "hints" }
        ));
    }

    let counts_part = if counts.is_empty() {
        "No issues found".to_string()
    } else {
        counts.join(", ")
    };

    output.push_str(&format!(
        "{} | {} files | {}\n",
        tools_part, report.files_analyzed, counts_part
    ));

    // --- Diagnostics ---
    if report.diagnostics.is_empty() {
        // Header already says "No issues found"
    } else {
        output.push('\n');

        // R2: Compute common path prefix for relative display
        // Use parent directories (not file paths) to avoid the single-file bug:
        // when all diagnostics are from one file, common_path_prefix returns the
        // file itself, strip_prefix yields empty, and falls back to full path.
        let parents: Vec<&std::path::Path> = report
            .diagnostics
            .iter()
            .filter_map(|d| d.file.parent())
            .collect();
        let prefix = common_path_prefix(&parents);

        // Sort diagnostics by file then line for consistent output
        let mut sorted_diags: Vec<&tldr_core::diagnostics::Diagnostic> =
            report.diagnostics.iter().collect();
        sorted_diags.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.column.cmp(&b.column))
        });

        for diag in &sorted_diags {
            let rel_path = strip_prefix_display(&diag.file, &prefix);

            // R1: severity as plain text (no ANSI)
            let severity_str = match diag.severity {
                tldr_core::diagnostics::Severity::Error => "error",
                tldr_core::diagnostics::Severity::Warning => "warning",
                tldr_core::diagnostics::Severity::Information => "info",
                tldr_core::diagnostics::Severity::Hint => "hint",
            };

            // Code part: [code] if present, empty string if not
            let code_str = diag
                .code
                .as_ref()
                .map(|c| format!("[{}]", c))
                .unwrap_or_default();

            // R3: Truncate multi-line messages to first line only
            let message = diag.message.lines().next().unwrap_or(&diag.message);

            // R1: One-line format: file:line:col: severity[code] message (tool)
            // No URLs emitted.
            output.push_str(&format!(
                "{}:{}:{}: {}{} {} ({})\n",
                rel_path, diag.line, diag.column, severity_str, code_str, message, diag.source
            ));
        }
    }

    // Show filtered count if any
    if filtered_count > 0 {
        output.push_str(&format!(
            "\n({} issues filtered by severity/ignore settings)\n",
            filtered_count
        ));
    }

    output
}

// =============================================================================
// Clone Detection Output Formatters (Phase 11)
// =============================================================================

/// Human-readable description of clone types (S8-P3-T5 mitigation)
///
/// Provides explanations that non-experts can understand, avoiding jargon
/// like "Type-2 parameterized clone".
pub fn clone_type_description(clone_type: &tldr_core::analysis::CloneType) -> &'static str {
    use tldr_core::analysis::CloneType;
    match clone_type {
        CloneType::Type1 => "exact match (identical code)",
        CloneType::Type2 => "identical structure, renamed identifiers/literals",
        CloneType::Type3 => "similar structure with additions/deletions",
    }
}

/// Generate hints for empty clone detection results (S8-P3-T7 mitigation)
///
/// When no clones are found, users need guidance on why and what to try.
pub fn empty_results_hints(
    options: &tldr_core::analysis::ClonesOptions,
    stats: &tldr_core::analysis::CloneStats,
) -> Vec<String> {
    vec![
        format!(
            "Analyzed {} files, {} tokens",
            stats.files_analyzed, stats.total_tokens
        ),
        format!(
            "Current threshold: {:.0}% - try --threshold 0.6 for more matches",
            options.threshold * 100.0
        ),
        format!(
            "Current min-tokens: {} - try --min-tokens 30 for smaller clones",
            options.min_tokens
        ),
    ]
}

/// Escape special characters for DOT node IDs (S8-P3-T11 mitigation)
///
/// Handles:
/// - Backslashes (Windows paths) -> forward slashes
/// - Quotes -> escaped quotes
/// - Spaces -> quoted node IDs
pub fn escape_dot_id(id: &str) -> String {
    // Convert backslashes to forward slashes (normalizes Windows paths)
    let normalized = id.replace('\\', "/");

    // Escape internal quotes
    let escaped = normalized.replace('"', r#"\""#);

    // Always quote the ID to handle spaces and special chars
    format!("\"{}\"", escaped)
}

/// Format clone detection report as compact human-readable text
///
/// Output format:
/// ```text
/// Clone Detection: 8 pairs in 42 files (15234 tokens)
///
///  #  Sim  Type  File A                          Lines    File B                          Lines
///  1  92%  T2    auth/login.py                   45-62    auth/signup.py                  23-40
///  2  85%  T3    core.py                         112-130  helpers.py                      88-106
/// ```
///
/// Key design decisions for LLM-friendly output:
/// - No ANSI color codes (wastes tokens, garbles non-terminal contexts)
/// - One line per clone pair (compact table)
/// - Common path prefix stripped from file paths
/// - No configuration echo (user knows what they ran)
/// - Compact type column: T1/T2/T3 instead of verbose descriptions
pub fn format_clones_text(report: &tldr_core::analysis::ClonesReport) -> String {
    let mut output = String::new();

    // Compact header with essential stats only
    output.push_str(&format!(
        "Clone Detection: {} pairs in {} files ({} tokens)\n",
        report.stats.clones_found, report.stats.files_analyzed, report.stats.total_tokens
    ));

    if report.clone_pairs.is_empty() {
        output.push_str("\nNo clones found.\n");
        return output;
    }

    output.push('\n');

    // Collect all file paths for common prefix computation
    let all_paths: Vec<&Path> = report
        .clone_pairs
        .iter()
        .flat_map(|p| [p.fragment1.file.as_path(), p.fragment2.file.as_path()])
        .collect();
    let prefix = common_path_prefix(&all_paths);

    // Table header
    output.push_str(&format!(
        " {:>2}  {:>3}  {:<4}  {:<30}  {:>9}  {:<30}  {:>9}\n",
        "#", "Sim", "Type", "File A", "Lines", "File B", "Lines"
    ));

    for pair in &report.clone_pairs {
        let sim = (pair.similarity * 100.0) as u32;
        let type_short = match pair.clone_type {
            tldr_core::analysis::CloneType::Type1 => "T1",
            tldr_core::analysis::CloneType::Type2 => "T2",
            tldr_core::analysis::CloneType::Type3 => "T3",
        };

        let file_a = strip_prefix_display(&pair.fragment1.file, &prefix);
        let file_b = strip_prefix_display(&pair.fragment2.file, &prefix);
        let lines_a = format!("{}-{}", pair.fragment1.start_line, pair.fragment1.end_line);
        let lines_b = format!("{}-{}", pair.fragment2.start_line, pair.fragment2.end_line);

        // Truncate file names if too long (show tail for readability)
        let file_a_display = if file_a.len() > 30 {
            format!("...{}", &file_a[file_a.len() - 27..])
        } else {
            file_a
        };
        let file_b_display = if file_b.len() > 30 {
            format!("...{}", &file_b[file_b.len() - 27..])
        } else {
            file_b
        };

        output.push_str(&format!(
            " {:>2}  {:>3}%  {:<4}  {:<30}  {:>9}  {:<30}  {:>9}\n",
            pair.id, sim, type_short, file_a_display, lines_a, file_b_display, lines_b
        ));
    }

    output
}

/// Format clone detection report as DOT graph for Graphviz
///
/// Output format:
/// ```dot
/// digraph clones {
///     rankdir=LR;
///     node [shape=box];
///
///     "src/auth/login.py:45-62" -> "src/auth/signup.py:23-40" [label="92%"];
/// }
/// ```
///
/// Handles special characters in paths (S8-P3-T11).
pub fn format_clones_dot(report: &tldr_core::analysis::ClonesReport) -> String {
    let mut output = String::new();

    output.push_str("digraph clones {\n");
    output.push_str("    rankdir=LR;\n");
    output.push_str("    node [shape=box, fontname=\"Helvetica\"];\n");
    output.push_str("    edge [fontname=\"Helvetica\", fontsize=10];\n");
    output.push('\n');

    // Add edges for each clone pair
    for pair in &report.clone_pairs {
        let node1 = format!(
            "{}:{}-{}",
            pair.fragment1.file.display(),
            pair.fragment1.start_line,
            pair.fragment1.end_line
        );
        let node2 = format!(
            "{}:{}-{}",
            pair.fragment2.file.display(),
            pair.fragment2.start_line,
            pair.fragment2.end_line
        );

        // Escape node IDs for special characters (S8-P3-T11)
        let node1_escaped = escape_dot_id(&node1);
        let node2_escaped = escape_dot_id(&node2);

        let similarity_pct = (pair.similarity * 100.0) as u32;
        let type_abbrev = match pair.clone_type {
            tldr_core::analysis::CloneType::Type1 => "T1",
            tldr_core::analysis::CloneType::Type2 => "T2",
            tldr_core::analysis::CloneType::Type3 => "T3",
        };

        output.push_str(&format!(
            "    {} -> {} [label=\"{}% {}\"];\n",
            node1_escaped, node2_escaped, similarity_pct, type_abbrev
        ));
    }

    output.push_str("}\n");
    output
}

// =============================================================================
// Similarity Analysis Output Formatters (Phase 11)
// =============================================================================

/// Format similarity report as human-readable text
///
/// Output format:
/// ```text
/// Similarity Analysis
/// ===================
///
/// Fragment 1: src/a.py (100 tokens, 20 lines)
/// Fragment 2: src/b.py (95 tokens, 18 lines)
///
/// Similarity Scores:
///   Dice:    0.85 (85%)
///   Jaccard: 0.74 (74%)
///
/// Interpretation: highly similar - likely refactoring candidates
///
/// Token Breakdown:
///   Shared tokens:  80
///   Unique to #1:   20
///   Unique to #2:   15
///   Total unique:   115
/// ```
pub fn format_similarity_text(report: &tldr_core::analysis::SimilarityReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&"Similarity Analysis\n".bold().to_string());
    output.push_str("===================\n\n");

    // Fragment info
    output.push_str(&format!(
        "Fragment 1: {} ({} tokens, {} lines)\n",
        report.fragment1.file.display().to_string().cyan(),
        report.fragment1.tokens,
        report.fragment1.lines
    ));
    if let Some(func) = &report.fragment1.function {
        output.push_str(&format!("  Function: {}\n", func.green()));
    }
    if let Some((start, end)) = report.fragment1.line_range {
        output.push_str(&format!("  Lines: {}-{}\n", start, end));
    }

    output.push_str(&format!(
        "Fragment 2: {} ({} tokens, {} lines)\n",
        report.fragment2.file.display().to_string().cyan(),
        report.fragment2.tokens,
        report.fragment2.lines
    ));
    if let Some(func) = &report.fragment2.function {
        output.push_str(&format!("  Function: {}\n", func.green()));
    }
    if let Some((start, end)) = report.fragment2.line_range {
        output.push_str(&format!("  Lines: {}-{}\n", start, end));
    }

    output.push('\n');

    // Similarity scores
    output.push_str(&"Similarity Scores:\n".bold().to_string());
    let dice_pct = (report.similarity.dice * 100.0) as u32;
    let jaccard_pct = (report.similarity.jaccard * 100.0) as u32;

    output.push_str(&format!(
        "  Dice:    {:.4} ({}%)\n",
        report.similarity.dice,
        dice_pct.to_string().green()
    ));
    output.push_str(&format!(
        "  Jaccard: {:.4} ({}%)\n",
        report.similarity.jaccard,
        jaccard_pct.to_string().green()
    ));

    if let Some(cosine) = report.similarity.cosine {
        let cosine_pct = (cosine * 100.0) as u32;
        output.push_str(&format!(
            "  Cosine:  {:.4} ({}%)\n",
            cosine,
            cosine_pct.to_string().green()
        ));
    }

    output.push('\n');

    // Interpretation
    output.push_str(&format!(
        "Interpretation: {}\n\n",
        report.similarity.interpretation.cyan()
    ));

    // Token breakdown
    output.push_str(&"Token Breakdown:\n".bold().to_string());
    output.push_str(&format!(
        "  Shared tokens:  {}\n",
        report.token_breakdown.shared_tokens.to_string().green()
    ));
    output.push_str(&format!(
        "  Unique to #1:   {}\n",
        report.token_breakdown.unique_to_fragment1
    ));
    output.push_str(&format!(
        "  Unique to #2:   {}\n",
        report.token_breakdown.unique_to_fragment2
    ));
    output.push_str(&format!(
        "  Total unique:   {}\n",
        report.token_breakdown.total_unique
    ));

    // Config info
    output.push('\n');
    output.push_str(&format!(
        "Metric: {:?}, N-gram size: {}\n",
        report.config.metric, report.config.ngram_size
    ));

    if let Some(lang) = &report.config.language {
        output.push_str(&format!("Language: {}\n", lang));
    }

    output
}

// =============================================================================
// SARIF Output Format (IDE/CI Integration)
// =============================================================================

/// SARIF 2.1.0 compliant output for IDE/CI integration
///
/// Supported by:
/// - GitHub Code Scanning
/// - VS Code SARIF Viewer
/// - Azure DevOps
/// - Many CI/CD systems
///
/// Reference: https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html
pub mod sarif {
    use serde::Serialize;
    use std::path::Path;
    use tldr_core::analysis::{CloneType, ClonesReport};

    /// SARIF log root
    #[derive(Debug, Serialize)]
    pub struct SarifLog {
        #[serde(rename = "$schema")]
        pub schema: String,
        pub version: String,
        pub runs: Vec<SarifRun>,
    }

    /// A single analysis run
    #[derive(Debug, Serialize)]
    pub struct SarifRun {
        pub tool: SarifTool,
        pub results: Vec<SarifResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub invocations: Option<Vec<SarifInvocation>>,
    }

    /// Tool information
    #[derive(Debug, Serialize)]
    pub struct SarifTool {
        pub driver: SarifDriver,
    }

    /// Tool driver (the actual analysis tool)
    #[derive(Debug, Serialize)]
    pub struct SarifDriver {
        pub name: String,
        pub version: String,
        #[serde(rename = "informationUri", skip_serializing_if = "Option::is_none")]
        pub information_uri: Option<String>,
        pub rules: Vec<SarifRule>,
    }

    /// Analysis rule definition
    #[derive(Debug, Serialize)]
    pub struct SarifRule {
        pub id: String,
        pub name: String,
        #[serde(rename = "shortDescription")]
        pub short_description: SarifMessage,
        #[serde(rename = "fullDescription", skip_serializing_if = "Option::is_none")]
        pub full_description: Option<SarifMessage>,
        #[serde(rename = "helpUri", skip_serializing_if = "Option::is_none")]
        pub help_uri: Option<String>,
        #[serde(
            rename = "defaultConfiguration",
            skip_serializing_if = "Option::is_none"
        )]
        pub default_configuration: Option<SarifConfiguration>,
    }

    /// Rule configuration
    #[derive(Debug, Serialize)]
    pub struct SarifConfiguration {
        pub level: String,
    }

    /// A single analysis result/finding
    #[derive(Debug, Serialize)]
    pub struct SarifResult {
        #[serde(rename = "ruleId")]
        pub rule_id: String,
        pub level: String,
        pub message: SarifMessage,
        pub locations: Vec<SarifLocation>,
        #[serde(rename = "relatedLocations", skip_serializing_if = "Vec::is_empty")]
        pub related_locations: Vec<SarifLocation>,
        #[serde(
            rename = "partialFingerprints",
            skip_serializing_if = "Option::is_none"
        )]
        pub partial_fingerprints: Option<SarifFingerprints>,
    }

    /// Message text
    #[derive(Debug, Serialize)]
    pub struct SarifMessage {
        pub text: String,
    }

    /// Code location
    #[derive(Debug, Serialize)]
    pub struct SarifLocation {
        #[serde(rename = "physicalLocation")]
        pub physical_location: SarifPhysicalLocation,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub id: Option<usize>,
    }

    /// Physical location in a file
    #[derive(Debug, Serialize)]
    pub struct SarifPhysicalLocation {
        #[serde(rename = "artifactLocation")]
        pub artifact_location: SarifArtifactLocation,
        pub region: SarifRegion,
    }

    /// File artifact location
    #[derive(Debug, Serialize)]
    pub struct SarifArtifactLocation {
        pub uri: String,
        #[serde(rename = "uriBaseId", skip_serializing_if = "Option::is_none")]
        pub uri_base_id: Option<String>,
    }

    /// Code region (lines/columns)
    #[derive(Debug, Serialize)]
    pub struct SarifRegion {
        #[serde(rename = "startLine")]
        pub start_line: usize,
        #[serde(rename = "endLine", skip_serializing_if = "Option::is_none")]
        pub end_line: Option<usize>,
    }

    /// Fingerprints for deduplication
    #[derive(Debug, Serialize)]
    pub struct SarifFingerprints {
        #[serde(
            rename = "primaryLocationLineHash",
            skip_serializing_if = "Option::is_none"
        )]
        pub primary_location_line_hash: Option<String>,
    }

    /// Invocation details
    #[derive(Debug, Serialize)]
    pub struct SarifInvocation {
        #[serde(rename = "executionSuccessful")]
        pub execution_successful: bool,
    }

    /// Get rule ID for clone type
    fn clone_type_rule_id(clone_type: CloneType) -> &'static str {
        match clone_type {
            CloneType::Type1 => "clone/type-1",
            CloneType::Type2 => "clone/type-2",
            CloneType::Type3 => "clone/type-3",
        }
    }

    /// Get human-readable clone type description
    fn clone_type_description(clone_type: CloneType) -> &'static str {
        match clone_type {
            CloneType::Type1 => "Exact code clone (identical except whitespace/comments)",
            CloneType::Type2 => "Parameterized clone (renamed identifiers/literals)",
            CloneType::Type3 => "Gapped clone (structural similarity with modifications)",
        }
    }

    /// Get severity level for clone type
    fn clone_type_level(clone_type: CloneType) -> &'static str {
        match clone_type {
            CloneType::Type1 => "warning", // Exact duplicates are more severe
            CloneType::Type2 => "warning",
            CloneType::Type3 => "note", // Similar code is informational
        }
    }

    /// Convert a path to URI format
    fn path_to_uri(path: &Path, root: &Path) -> String {
        // Try to make path relative to root
        let relative = path.strip_prefix(root).unwrap_or(path);
        relative.to_string_lossy().replace('\\', "/")
    }

    /// Convert ClonesReport to SARIF format
    pub fn format_clones_sarif(report: &ClonesReport) -> SarifLog {
        // Define rules for each clone type
        let rules = vec![
            SarifRule {
                id: "clone/type-1".to_string(),
                name: "ExactClone".to_string(),
                short_description: SarifMessage {
                    text: "Exact code clone detected".to_string(),
                },
                full_description: Some(SarifMessage {
                    text: "Type-1 clone: Identical code fragments (ignoring whitespace and comments). Consider extracting to a shared function or module.".to_string(),
                }),
                help_uri: None,
                default_configuration: Some(SarifConfiguration {
                    level: "warning".to_string(),
                }),
            },
            SarifRule {
                id: "clone/type-2".to_string(),
                name: "ParameterizedClone".to_string(),
                short_description: SarifMessage {
                    text: "Parameterized clone detected".to_string(),
                },
                full_description: Some(SarifMessage {
                    text: "Type-2 clone: Code fragments with renamed identifiers or different literal values. The structure is identical. Consider refactoring to accept parameters.".to_string(),
                }),
                help_uri: None,
                default_configuration: Some(SarifConfiguration {
                    level: "warning".to_string(),
                }),
            },
            SarifRule {
                id: "clone/type-3".to_string(),
                name: "GappedClone".to_string(),
                short_description: SarifMessage {
                    text: "Similar code pattern detected".to_string(),
                },
                full_description: Some(SarifMessage {
                    text: "Type-3 clone: Code fragments with similar structure but some statements added, removed, or modified. May indicate copy-paste programming.".to_string(),
                }),
                help_uri: None,
                default_configuration: Some(SarifConfiguration {
                    level: "note".to_string(),
                }),
            },
        ];

        // Convert clone pairs to SARIF results
        let results: Vec<SarifResult> = report
            .clone_pairs
            .iter()
            .map(|pair| {
                let rule_id = clone_type_rule_id(pair.clone_type).to_string();
                let level = clone_type_level(pair.clone_type).to_string();

                // Primary location (fragment1)
                let primary_location = SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation {
                            uri: path_to_uri(&pair.fragment1.file, &report.root),
                            uri_base_id: Some("%SRCROOT%".to_string()),
                        },
                        region: SarifRegion {
                            start_line: pair.fragment1.start_line,
                            end_line: Some(pair.fragment1.end_line),
                        },
                    },
                    id: None,
                };

                // Related location (fragment2)
                let related_location = SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation {
                            uri: path_to_uri(&pair.fragment2.file, &report.root),
                            uri_base_id: Some("%SRCROOT%".to_string()),
                        },
                        region: SarifRegion {
                            start_line: pair.fragment2.start_line,
                            end_line: Some(pair.fragment2.end_line),
                        },
                    },
                    id: Some(1),
                };

                let message = format!(
                    "{} ({:.0}% similar to {}:{})",
                    clone_type_description(pair.clone_type),
                    pair.similarity * 100.0,
                    path_to_uri(&pair.fragment2.file, &report.root),
                    pair.fragment2.start_line
                );

                SarifResult {
                    rule_id,
                    level,
                    message: SarifMessage { text: message },
                    locations: vec![primary_location],
                    related_locations: vec![related_location],
                    partial_fingerprints: Some(SarifFingerprints {
                        primary_location_line_hash: Some(format!(
                            "{}:{}:{}:{}",
                            path_to_uri(&pair.fragment1.file, &report.root),
                            pair.fragment1.start_line,
                            path_to_uri(&pair.fragment2.file, &report.root),
                            pair.fragment2.start_line
                        )),
                    }),
                }
            })
            .collect();

        SarifLog {
            schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json".to_string(),
            version: "2.1.0".to_string(),
            runs: vec![SarifRun {
                tool: SarifTool {
                    driver: SarifDriver {
                        name: "tldr".to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        information_uri: Some("https://github.com/anthropics/claude-code".to_string()),
                        rules,
                    },
                },
                results,
                invocations: Some(vec![SarifInvocation {
                    execution_successful: true,
                }]),
            }],
        }
    }
}

/// Format ModuleInfo for text output
///
/// Compact, human-readable summary of a module's contents:
/// ```text
/// /src/example.py (python)
///   "Example module for testing."
///
/// Imports (2)
///   import os
///   from typing: List, Optional
///
/// Functions (2)
///   async process_data(data: list, config: dict) -> bool  L10
///     "Process input data."
///   helper()  L25
///
/// Classes (1)
///   DataHandler(BaseHandler, Serializable)  L30
///     "Handles data processing."
///     Fields: config: dict
///     Methods: __init__(self, config: dict), async run(self) -> Result
///
/// Constants (1)
///   MAX_RETRIES: int = 3  L5
///
/// Call Graph (2 edges)
///   process_data -> helper
///   DataHandler.run -> process_data
/// ```
pub fn format_module_info_text(info: &tldr_core::types::ModuleInfo) -> String {
    let mut output = String::new();

    // Header: file path + language
    output.push_str(&format!(
        "{} ({})\n",
        info.file_path.display().to_string().bold(),
        info.language.as_str().cyan()
    ));

    // Docstring (truncated to 80 chars)
    if let Some(ref doc) = info.docstring {
        let truncated = if doc.len() > 80 {
            format!("{}...", &doc[..77])
        } else {
            doc.clone()
        };
        output.push_str(&format!("  \"{}\"\n", truncated.dimmed()));
    }

    output.push('\n');

    // Imports
    if !info.imports.is_empty() {
        output.push_str(&format!("{} ({})\n", "Imports".bold(), info.imports.len()));
        output.push_str(&format!(
            "  {}",
            format_imports_text(&info.imports)
                .lines()
                .collect::<Vec<_>>()
                .join("\n  ")
        ));
        output.push('\n');
    }

    // Functions
    if !info.functions.is_empty() {
        output.push_str(&format!(
            "{} ({})\n",
            "Functions".bold(),
            info.functions.len()
        ));
        for func in &info.functions {
            format_function_line(&mut output, func, "  ");
        }
        output.push('\n');
    }

    // Classes
    if !info.classes.is_empty() {
        output.push_str(&format!("{} ({})\n", "Classes".bold(), info.classes.len()));
        for class in &info.classes {
            // Class name with bases
            let bases_str = if class.bases.is_empty() {
                String::new()
            } else {
                format!("({})", class.bases.join(", "))
            };
            output.push_str(&format!(
                "  {}{}  L{}\n",
                class.name.green(),
                bases_str,
                class.line_number
            ));

            // Class docstring
            if let Some(ref doc) = class.docstring {
                let truncated = if doc.len() > 80 {
                    format!("{}...", &doc[..77])
                } else {
                    doc.clone()
                };
                output.push_str(&format!("    \"{}\"\n", truncated.dimmed()));
            }

            // Fields summary (compact one-liner)
            if !class.fields.is_empty() {
                let fields_summary: Vec<String> = class
                    .fields
                    .iter()
                    .map(|f| {
                        if let Some(ref ft) = f.field_type {
                            format!("{}: {}", f.name, ft)
                        } else {
                            f.name.clone()
                        }
                    })
                    .collect();
                output.push_str(&format!("    Fields: {}\n", fields_summary.join(", ")));
            }

            // Methods summary (compact one-liner)
            if !class.methods.is_empty() {
                let methods_summary: Vec<String> = class
                    .methods
                    .iter()
                    .map(|m| {
                        let async_prefix = if m.is_async { "async " } else { "" };
                        let params_str = m.params.join(", ");
                        let ret = m
                            .return_type
                            .as_ref()
                            .map(|r| format!(" -> {}", r))
                            .unwrap_or_default();
                        format!("{}{}({}){}", async_prefix, m.name, params_str, ret)
                    })
                    .collect();
                output.push_str(&format!("    Methods: {}\n", methods_summary.join(", ")));
            }
        }
        output.push('\n');
    }

    // Constants
    if !info.constants.is_empty() {
        output.push_str(&format!(
            "{} ({})\n",
            "Constants".bold(),
            info.constants.len()
        ));
        for c in &info.constants {
            let type_str = c
                .field_type
                .as_ref()
                .map(|t| format!(": {}", t))
                .unwrap_or_default();
            let val_str = c
                .default_value
                .as_ref()
                .map(|v| format!(" = {}", v))
                .unwrap_or_default();
            output.push_str(&format!(
                "  {}{}{}  L{}\n",
                c.name.cyan(),
                type_str,
                val_str,
                c.line_number
            ));
        }
        output.push('\n');
    }

    // Call Graph summary (top 10 edges, grouped by caller)
    let total_edges: usize = info.call_graph.calls.values().map(|v| v.len()).sum();
    if total_edges > 0 {
        output.push_str(&format!(
            "{} ({} edges)\n",
            "Call Graph".bold(),
            total_edges
        ));

        // Sort callers for deterministic output
        let mut callers: Vec<_> = info.call_graph.calls.keys().collect();
        callers.sort();

        let mut shown = 0;
        for caller in callers {
            if shown >= 10 {
                let remaining = total_edges - shown;
                if remaining > 0 {
                    output.push_str(&format!("  ... and {} more edges\n", remaining));
                }
                break;
            }
            if let Some(callees) = info.call_graph.calls.get(caller.as_str()) {
                for callee in callees {
                    output.push_str(&format!("  {} -> {}\n", caller.dimmed(), callee.green()));
                    shown += 1;
                    if shown >= 10 {
                        break;
                    }
                }
            }
        }
    }

    output
}

/// Format a single function entry for module info text output
fn format_function_line(output: &mut String, func: &tldr_core::types::FunctionInfo, indent: &str) {
    let async_prefix = if func.is_async { "async " } else { "" };
    let params_str = func.params.join(", ");
    let ret_str = func
        .return_type
        .as_ref()
        .map(|r| format!(" -> {}", r))
        .unwrap_or_default();
    output.push_str(&format!(
        "{}{}{}({}){}  L{}\n",
        indent,
        async_prefix.cyan(),
        func.name.green(),
        params_str,
        ret_str,
        func.line_number
    ));

    // Docstring preview (truncated)
    if let Some(ref doc) = func.docstring {
        let truncated = if doc.len() > 60 {
            format!("{}...", &doc[..57])
        } else {
            doc.clone()
        };
        output.push_str(&format!("{}  \"{}\"\n", indent, truncated.dimmed()));
    }
}

/// Format ClonesReport as SARIF JSON
pub fn format_clones_sarif(report: &tldr_core::analysis::ClonesReport) -> String {
    let sarif_log = sarif::format_clones_sarif(report);
    serde_json::to_string_pretty(&sarif_log).unwrap_or_else(|_| "{}".to_string())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
#[path = "output_tests.rs"]
mod output_tests;
