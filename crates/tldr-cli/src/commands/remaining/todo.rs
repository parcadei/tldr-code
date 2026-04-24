//! Todo command - Improvement aggregator
//!
//! Aggregates improvement suggestions from multiple sub-analyses:
//! - Dead code analysis (existing `tldr dead`)
//! - Complexity analysis (existing `tldr complexity`)
//! - Cohesion analysis (existing `tldr cohesion`)
//! - Equivalence analysis (implement later, stub for now)
//! - Similar code analysis (existing `tldr similar`)
//!
//! # Example
//!
//! ```bash
//! tldr todo src/
//! tldr todo src/main.py --quick
//! tldr todo src/ --detail dead --format text
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use clap::Args;
use serde_json::Value;
use tldr_core::walker::ProjectWalker;

use super::ast_cache::AstCache;
use super::error::{RemainingError, RemainingResult};
use super::types::{TodoItem, TodoReport, TodoSummary};

use crate::output::OutputWriter;

// Import existing analysis modules
use crate::commands::dead::collect_module_infos_with_refcounts;
use tldr_core::analysis::dead::dead_code_analysis_refcount;
use tldr_core::{collect_all_functions, get_code_structure, FunctionRef, IgnoreSpec, Language};

// =============================================================================
// Constants
// =============================================================================

/// Priority levels for different categories
const PRIORITY_DEAD_CODE: u32 = 1;
const PRIORITY_COMPLEXITY: u32 = 2;
const PRIORITY_COHESION: u32 = 3;
const PRIORITY_EQUIVALENCE: u32 = 4;
const PRIORITY_SIMILAR: u32 = 5;

// =============================================================================
// Sub-Analysis Enum
// =============================================================================

/// Types of sub-analyses that todo command orchestrates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubAnalysis {
    Dead,
    Complexity,
    Cohesion,
    Equivalence,
    Similar,
}

impl SubAnalysis {
    /// Get all analyses for full mode
    pub fn all() -> &'static [SubAnalysis] {
        &[
            SubAnalysis::Dead,
            SubAnalysis::Complexity,
            SubAnalysis::Cohesion,
            SubAnalysis::Equivalence,
            SubAnalysis::Similar,
        ]
    }

    /// Get analyses for quick mode (skip similar which is slowest)
    pub fn quick() -> &'static [SubAnalysis] {
        &[
            SubAnalysis::Dead,
            SubAnalysis::Complexity,
            SubAnalysis::Cohesion,
            SubAnalysis::Equivalence,
        ]
    }

    /// Get the priority for this analysis type
    pub fn priority(&self) -> u32 {
        match self {
            SubAnalysis::Dead => PRIORITY_DEAD_CODE,
            SubAnalysis::Complexity => PRIORITY_COMPLEXITY,
            SubAnalysis::Cohesion => PRIORITY_COHESION,
            SubAnalysis::Equivalence => PRIORITY_EQUIVALENCE,
            SubAnalysis::Similar => PRIORITY_SIMILAR,
        }
    }

    /// Get the category name for this analysis
    pub fn category(&self) -> &'static str {
        match self {
            SubAnalysis::Dead => "dead_code",
            SubAnalysis::Complexity => "complexity",
            SubAnalysis::Cohesion => "cohesion",
            SubAnalysis::Equivalence => "equivalence",
            SubAnalysis::Similar => "similar",
        }
    }
}

impl std::str::FromStr for SubAnalysis {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "dead" | "dead_code" => Ok(SubAnalysis::Dead),
            "complexity" | "complex" => Ok(SubAnalysis::Complexity),
            "cohesion" | "lcom4" => Ok(SubAnalysis::Cohesion),
            "equivalence" | "equiv" | "gvn" => Ok(SubAnalysis::Equivalence),
            "similar" | "sim" => Ok(SubAnalysis::Similar),
            _ => Err(format!("Unknown analysis: {}", s)),
        }
    }
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Aggregate improvement suggestions from multiple analyses
///
/// Runs dead code, complexity, cohesion, equivalence, and similar code analyses,
/// then aggregates findings into a priority-sorted list of improvement items.
///
/// # Example
///
/// ```bash
/// tldr todo src/
/// tldr todo src/main.py --quick
/// tldr todo src/ --detail dead
/// ```
#[derive(Debug, Args)]
pub struct TodoArgs {
    /// File or directory to analyze
    pub path: PathBuf,

    /// Show details for specific sub-analysis
    #[arg(long)]
    pub detail: Option<String>,

    /// Run quick mode (skip similar analysis)
    #[arg(long)]
    pub quick: bool,

    /// Maximum number of items to display (0 = show all)
    #[arg(long, default_value = "20")]
    pub max_items: usize,

    /// Output file (optional, stdout if not specified)
    #[arg(long, short = 'O')]
    pub output: Option<PathBuf>,
}

impl TodoArgs {
    /// Run the todo command
    pub fn run(
        &self,
        format: crate::output::OutputFormat,
        quiet: bool,
        lang: Option<Language>,
    ) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);
        let start = Instant::now();

        writer.progress(&format!(
            "Analyzing {} for improvements...",
            self.path.display()
        ));

        // Validate path exists
        if !self.path.exists() {
            return Err(RemainingError::file_not_found(&self.path).into());
        }

        // Determine language from CLI option or auto-detect
        let language = if let Some(l) = lang {
            l
        } else {
            detect_language(&self.path)?
        };

        // Create AST cache for shared parsing
        let mut cache = AstCache::default();

        // Determine which analyses to run
        let analyses = if self.quick {
            SubAnalysis::quick()
        } else {
            SubAnalysis::all()
        };

        // Run sub-analyses and collect results
        let mut sub_results: HashMap<String, Value> = HashMap::new();
        let mut all_items: Vec<TodoItem> = Vec::new();
        let mut summary = TodoSummary::default();

        for analysis in analyses {
            writer.progress(&format!("Running {} analysis...", analysis.category()));

            match run_sub_analysis(*analysis, &self.path, language, &mut cache) {
                Ok((items, result_value)) => {
                    // Update summary
                    update_summary(&mut summary, *analysis, &items);

                    // Store raw results if detail requested (match by parsing the detail arg)
                    if let Some(ref detail) = self.detail {
                        if let Ok(detail_analysis) = detail.parse::<SubAnalysis>() {
                            if detail_analysis == *analysis {
                                sub_results.insert(analysis.category().to_string(), result_value);
                            }
                        }
                    }

                    // Add items to aggregate list
                    all_items.extend(items);
                }
                Err(e) => {
                    // Log error but continue with other analyses
                    writer.progress(&format!(
                        "Warning: {} analysis failed: {}",
                        analysis.category(),
                        e
                    ));
                }
            }
        }

        // Sort items by priority
        all_items.sort_by_key(|item| item.priority);

        // Apply max_items truncation
        let total_items = all_items.len();
        let truncated = self.max_items > 0 && total_items > self.max_items;
        if truncated {
            all_items.truncate(self.max_items);
        }

        // Build report
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let report = TodoReport {
            wrapper: "todo".to_string(),
            path: self.path.display().to_string(),
            items: all_items,
            summary,
            sub_results,
            total_elapsed_ms: elapsed_ms,
        };

        // Write output
        if let Some(ref output_path) = self.output {
            // Write to file based on format
            if writer.is_text() {
                let text = format_todo_text(&report, truncated, total_items);
                fs::write(output_path, text)?;
            } else {
                let json = serde_json::to_string_pretty(&report)?;
                fs::write(output_path, json)?;
            }
        } else {
            // Write to stdout
            if writer.is_text() {
                let text = format_todo_text(&report, truncated, total_items);
                writer.write_text(&text)?;
            } else {
                writer.write(&report)?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Sub-Analysis Runners
// =============================================================================

/// Run a sub-analysis and return items + raw result
fn run_sub_analysis(
    analysis: SubAnalysis,
    path: &Path,
    language: Language,
    _cache: &mut AstCache,
) -> RemainingResult<(Vec<TodoItem>, Value)> {
    match analysis {
        SubAnalysis::Dead => run_dead_analysis(path, language),
        SubAnalysis::Complexity => run_complexity_analysis(path, language),
        SubAnalysis::Cohesion => run_cohesion_analysis(path),
        SubAnalysis::Equivalence => run_equivalence_analysis(path),
        SubAnalysis::Similar => run_similar_analysis(path),
    }
}

/// Run dead code analysis using reference counting (low false-positive rate)
fn run_dead_analysis(path: &Path, language: Language) -> RemainingResult<(Vec<TodoItem>, Value)> {
    // For single files, use parent directory for scanning (needs directory context)
    let project_root = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    // Single-pass: collect module infos and identifier reference counts together
    let (module_infos, merged_ref_counts) =
        collect_module_infos_with_refcounts(project_root, language, false);
    let all_functions: Vec<FunctionRef> = collect_all_functions(&module_infos);

    // Run refcount-based analysis (rescues functions that are referenced by name)
    let report = dead_code_analysis_refcount(&all_functions, &merged_ref_counts, None)
        .map_err(|e| RemainingError::analysis_error(format!("Dead code analysis failed: {}", e)))?;

    // Convert to TodoItems
    let items: Vec<TodoItem> = report
        .dead_functions
        .iter()
        .map(|func| {
            TodoItem::new(
                "dead_code",
                PRIORITY_DEAD_CODE,
                format!("Unused function: {}", func.name),
            )
            .with_location(func.file.display().to_string(), 0)
            .with_severity("medium")
        })
        .collect();

    let result_value = serde_json::to_value(&report).unwrap_or(Value::Null);

    Ok((items, result_value))
}

/// Run complexity analysis (hotspots)
fn run_complexity_analysis(
    path: &Path,
    language: Language,
) -> RemainingResult<(Vec<TodoItem>, Value)> {
    // Get structure to find functions
    let structure = get_code_structure(path, language, 0, Some(&IgnoreSpec::default()))
        .map_err(|e| RemainingError::analysis_error(format!("Failed to get structure: {}", e)))?;

    let mut items = Vec::new();

    // Check each function for high complexity (threshold: cyclomatic > 10)
    for file in &structure.files {
        for func_name in &file.functions {
            let file_path = path.join(&file.path);
            if let Ok(metrics) = tldr_core::calculate_complexity(
                file_path.to_str().unwrap_or_default(),
                func_name,
                language,
            ) {
                if metrics.cyclomatic > 10 {
                    items.push(
                        TodoItem::new(
                            "complexity",
                            PRIORITY_COMPLEXITY,
                            format!(
                                "High complexity in {}: cyclomatic={}, consider refactoring",
                                func_name, metrics.cyclomatic
                            ),
                        )
                        .with_location(file.path.display().to_string(), 1)
                        .with_severity(if metrics.cyclomatic > 20 {
                            "high"
                        } else {
                            "medium"
                        })
                        .with_score(metrics.cyclomatic as f64 / 50.0),
                    );
                }
            }
        }
    }

    let result_value = serde_json::json!({
        "hotspots": items.len(),
        "threshold": 10
    });

    Ok((items, result_value))
}

/// Run cohesion analysis (LCOM4)
fn run_cohesion_analysis(path: &Path) -> RemainingResult<(Vec<TodoItem>, Value)> {
    use crate::commands::patterns::cohesion::{run as run_cohesion, CohesionArgs};

    let args = CohesionArgs {
        path: path.to_path_buf(),
        min_methods: 1,
        include_dunder: false,
        output_format: crate::commands::patterns::cohesion::OutputFormat::Json,
        timeout: 30,
        project_root: None,
        lang: None,
    };

    let report = run_cohesion(args)
        .map_err(|e| RemainingError::analysis_error(format!("Cohesion analysis failed: {}", e)))?;

    let items: Vec<TodoItem> = report
        .classes
        .iter()
        .filter(|c| c.lcom4 > 1)
        .map(|c| {
            TodoItem::new(
                "cohesion",
                PRIORITY_COHESION,
                format!(
                    "Low cohesion in class {}: LCOM4={}, consider splitting",
                    c.class_name, c.lcom4
                ),
            )
            .with_location(c.file_path.clone(), c.line)
            .with_severity(if c.lcom4 > 3 { "high" } else { "medium" })
            .with_score(c.lcom4 as f64 / 5.0)
        })
        .collect();

    let result_value = serde_json::to_value(&report).unwrap_or(Value::Null);

    Ok((items, result_value))
}

/// Run equivalence analysis (GVN - stub for now)
fn run_equivalence_analysis(_path: &Path) -> RemainingResult<(Vec<TodoItem>, Value)> {
    // TODO: Implement GVN equivalence detection in Phase 9
    // For now, return empty results
    let result_value = serde_json::json!({
        "status": "not_implemented",
        "message": "GVN equivalence analysis will be implemented in Phase 9"
    });

    Ok((Vec::new(), result_value))
}

/// Run similar code analysis (stub - uses semantic search)
fn run_similar_analysis(_path: &Path) -> RemainingResult<(Vec<TodoItem>, Value)> {
    // TODO: Integrate with tldr similar command
    // For now, return empty results as similar analysis is expensive
    let result_value = serde_json::json!({
        "status": "skipped",
        "message": "Similar code analysis is expensive, consider using 'tldr similar' directly"
    });

    Ok((Vec::new(), result_value))
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Detect language from path (auto-detect from extension or directory contents)
fn detect_language(path: &Path) -> RemainingResult<Language> {
    // Auto-detect from file extension or directory contents
    if path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        match ext {
            "py" => Ok(Language::Python),
            "ts" | "tsx" => Ok(Language::TypeScript),
            "js" | "jsx" => Ok(Language::JavaScript),
            "rs" => Ok(Language::Rust),
            "go" => Ok(Language::Go),
            _ => Err(RemainingError::unsupported_language(ext)),
        }
    } else if path.is_dir() {
        // Check for common files to detect language
        for entry in ProjectWalker::new(path).max_depth(2).iter() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                match ext {
                    "py" => return Ok(Language::Python),
                    "ts" | "tsx" => return Ok(Language::TypeScript),
                    "js" | "jsx" => return Ok(Language::JavaScript),
                    "rs" => return Ok(Language::Rust),
                    "go" => return Ok(Language::Go),
                    _ => continue,
                }
            }
        }
        // Default to Python if no language detected
        Ok(Language::Python)
    } else {
        Err(RemainingError::file_not_found(path))
    }
}

/// Update summary based on analysis results
fn update_summary(summary: &mut TodoSummary, analysis: SubAnalysis, items: &[TodoItem]) {
    match analysis {
        SubAnalysis::Dead => summary.dead_count = items.len() as u32,
        SubAnalysis::Complexity => summary.hotspot_count = items.len() as u32,
        SubAnalysis::Cohesion => summary.low_cohesion_count = items.len() as u32,
        SubAnalysis::Equivalence => summary.equivalence_groups = items.len() as u32,
        SubAnalysis::Similar => summary.similar_pairs = items.len() as u32,
    }
}

/// Format todo report as human-readable text
///
/// When `truncated` is true, a footer message is appended indicating how many
/// items were omitted and how to see all of them. `total_items` is the count
/// before truncation.
pub fn format_todo_text(report: &TodoReport, truncated: bool, total_items: usize) -> String {
    let mut lines = Vec::new();

    lines.push(format!("TODO Report for: {}", report.path));
    lines.push(format!("Total items: {}", total_items));
    lines.push(String::new());

    // Summary
    lines.push("Summary:".to_string());
    lines.push(format!("  Dead code items: {}", report.summary.dead_count));
    lines.push(format!(
        "  Complexity hotspots: {}",
        report.summary.hotspot_count
    ));
    lines.push(format!(
        "  Low cohesion classes: {}",
        report.summary.low_cohesion_count
    ));
    lines.push(format!(
        "  Similar code pairs: {}",
        report.summary.similar_pairs
    ));
    lines.push(format!(
        "  Equivalence groups: {}",
        report.summary.equivalence_groups
    ));
    lines.push(String::new());

    if report.items.is_empty() {
        lines.push("No improvement items found.".to_string());
    } else {
        lines.push("Items (sorted by priority):".to_string());
        lines.push(String::new());

        for (i, item) in report.items.iter().enumerate() {
            lines.push(format!(
                "{}. [{}] {} (priority: {})",
                i + 1,
                item.category,
                item.description,
                item.priority
            ));

            if !item.file.is_empty() {
                lines.push(format!("   Location: {}:{}", item.file, item.line));
            }

            if !item.severity.is_empty() {
                lines.push(format!("   Severity: {}", item.severity));
            }
        }

        if truncated {
            let remaining = total_items - report.items.len();
            lines.push(String::new());
            lines.push(format!(
                "... and {} more items. Use --max-items 0 to show all.",
                remaining
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!("Analysis time: {:.2}ms", report.total_elapsed_ms));

    lines.join("\n")
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sub_analysis_from_str() {
        assert_eq!("dead".parse::<SubAnalysis>().unwrap(), SubAnalysis::Dead);
        assert_eq!(
            "complexity".parse::<SubAnalysis>().unwrap(),
            SubAnalysis::Complexity
        );
        assert_eq!(
            "cohesion".parse::<SubAnalysis>().unwrap(),
            SubAnalysis::Cohesion
        );
        assert!("unknown".parse::<SubAnalysis>().is_err());
    }

    #[test]
    fn test_sub_analysis_priority() {
        assert!(SubAnalysis::Dead.priority() < SubAnalysis::Complexity.priority());
        assert!(SubAnalysis::Complexity.priority() < SubAnalysis::Cohesion.priority());
    }

    #[test]
    fn test_quick_mode_skips_similar() {
        let quick = SubAnalysis::quick();
        let all = SubAnalysis::all();

        assert!(quick.len() < all.len());
        assert!(!quick.contains(&SubAnalysis::Similar));
        assert!(all.contains(&SubAnalysis::Similar));
    }

    #[test]
    fn test_format_todo_text() {
        let mut report = TodoReport::new("/path/to/project");
        report
            .items
            .push(TodoItem::new("dead_code", 1, "Unused function"));
        report.summary.dead_count = 1;
        report.total_elapsed_ms = 100.5;

        let text = format_todo_text(&report, false, 1);
        assert!(text.contains("TODO Report"));
        assert!(text.contains("Dead code items: 1"));
        assert!(text.contains("Unused function"));
    }

    #[test]
    fn test_todo_args_max_items_default() {
        // Default max_items should be 20
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct Wrapper {
            #[command(flatten)]
            todo: TodoArgs,
        }

        let w = Wrapper::parse_from(["test", "src/"]);
        assert_eq!(w.todo.max_items, 20, "default max_items should be 20");
    }

    #[test]
    fn test_todo_args_max_items_flag() {
        // --max-items 10 should parse correctly
        use clap::Parser;

        #[derive(Debug, Parser)]
        struct Wrapper {
            #[command(flatten)]
            todo: TodoArgs,
        }

        let w = Wrapper::parse_from(["test", "src/", "--max-items", "10"]);
        assert_eq!(w.todo.max_items, 10);
    }

    #[test]
    fn test_todo_output_respects_max_items() {
        // When max_items is set, format_todo_text should only show that many items
        let mut report = TodoReport::new("/path/to/project");
        for i in 0..20 {
            report.items.push(TodoItem::new(
                "dead_code",
                1,
                format!("Unused function: fn_{}", i),
            ));
        }
        report.summary.dead_count = 20;
        report.total_elapsed_ms = 50.0;

        // Apply max_items=5 truncation
        let max_items: usize = 5;
        let total = report.items.len();
        let truncated = total > max_items && max_items > 0;
        if truncated {
            report.items.truncate(max_items);
        }

        let text = format_todo_text(&report, truncated, total);
        // Should contain exactly 5 numbered items
        assert!(text.contains("1. [dead_code]"));
        assert!(text.contains("5. [dead_code]"));
        assert!(!text.contains("6. [dead_code]"));
        // Should contain truncation message
        assert!(text.contains("... and 15 more items"));
        assert!(text.contains("--max-items 0"));
    }

    #[test]
    fn test_todo_output_no_truncation_message_when_not_truncated() {
        let mut report = TodoReport::new("/path/to/project");
        for i in 0..3 {
            report.items.push(TodoItem::new(
                "dead_code",
                1,
                format!("Unused function: fn_{}", i),
            ));
        }
        report.summary.dead_count = 3;
        report.total_elapsed_ms = 10.0;

        let text = format_todo_text(&report, false, 3);
        assert!(!text.contains("... and"));
        assert!(!text.contains("--max-items"));
    }

    #[test]
    fn test_detect_language_from_extension() {
        use std::fs::File;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let py_file = temp.path().join("test.py");
        File::create(&py_file).unwrap();

        let lang = detect_language(&py_file).unwrap();
        assert_eq!(lang, Language::Python);
    }

    #[test]
    fn test_run_dead_analysis_uses_refcount() {
        // Verify run_dead_analysis uses the refcount-based analyzer (not old call-graph).
        // Create a minimal Python project with one "dead" function.
        use std::fs;
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let py_file = temp.path().join("sample.py");
        // _dead_func is private (leading underscore) and only appears once (definition),
        // so refcount=1 -> dead. used_func appears twice (def + call), so refcount=2 -> alive.
        fs::write(
            &py_file,
            "def used_func():\n    pass\n\ndef _dead_func():\n    pass\n\nused_func()\n",
        )
        .unwrap();

        let (items, value) = run_dead_analysis(temp.path(), Language::Python).unwrap();
        // The refcount analyzer should find _dead_func as dead (private, ref_count=1)
        // but not used_func (ref_count=2, rescued by refcount).
        let dead_names: Vec<&str> = items.iter().map(|i| i.description.as_str()).collect();
        assert!(
            dead_names.iter().any(|d| d.contains("_dead_func")),
            "Expected _dead_func to be reported as dead, got: {:?}",
            dead_names
        );
        assert!(
            !dead_names.iter().any(|d| d.contains("used_func")),
            "used_func should NOT be reported as dead, got: {:?}",
            dead_names
        );
        // The result value should serialize successfully
        assert!(!value.is_null(), "Expected non-null result value");
    }
}
