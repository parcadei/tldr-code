//! Todo Orchestrator - Actionable improvement suggestions, priority-sorted
//!
//! This module wires together existing Rust analyzers to produce a prioritized
//! list of code improvement opportunities.
//!
//! # Priority Mapping (from spec)
//! - 1: Dead code (unreachable functions)
//! - 2: High complexity (CC > 20)
//! - 3: Low cohesion (LCOM4 > 2)
//! - 4: Similar functions (potential duplication)
//! - 5: Equivalence/redundancy (GVN)
//! - 6: Medium complexity (CC > 10)
//!
//! # Sub-Analyses
//! - dead: `quality::dead_code::analyze_dead_code`
//! - complexity: `quality::complexity::analyze_complexity`
//! - cohesion: `quality::cohesion::analyze_cohesion`
//! - similarity: `quality::similarity::find_similar_with_options`
//! - equivalence: `dfg::gvn::compute_gvn`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use super::base::{progress, safe_call, SubAnalysisResult};
use crate::dfg::gvn::compute_gvn;
use crate::quality::cohesion::{analyze_cohesion_with_options, CohesionOptions};
use crate::quality::complexity::{analyze_complexity, ComplexityOptions};
use crate::quality::dead_code::analyze_dead_code;
use crate::quality::similarity::{find_similar_with_options, SimilarityOptions};
use crate::types::Language;
use crate::TldrResult;

// =============================================================================
// Types
// =============================================================================

/// A single actionable improvement item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    /// Category of improvement (dead, complexity, cohesion, similar, equivalence)
    pub category: String,
    /// Priority level (1 = highest, 6 = lowest)
    pub priority: u8,
    /// Human-readable description of the issue
    pub description: String,
    /// File path where the issue occurs
    pub file: String,
    /// Line number where the issue occurs (0 if not applicable)
    pub line: usize,
    /// Severity level (high, medium, low)
    pub severity: String,
    /// Numeric score for the issue (e.g., similarity score, complexity value)
    pub score: f64,
}

impl TodoItem {
    /// Create a new TodoItem
    pub fn new(
        category: &str,
        priority: u8,
        description: &str,
        file: &str,
        line: usize,
        severity: &str,
        score: f64,
    ) -> Self {
        Self {
            category: category.to_string(),
            priority,
            description: description.to_string(),
            file: file.to_string(),
            line,
            severity: severity.to_string(),
            score,
        }
    }
}

/// Summary statistics for the todo report
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TodoSummary {
    /// Number of dead code functions found
    pub dead_count: usize,
    /// Number of complexity hotspots (CC > 10)
    pub hotspot_count: usize,
    /// Number of low-cohesion classes (LCOM4 > 2)
    pub low_cohesion_count: usize,
    /// Number of similar function pairs found
    pub similar_pairs: usize,
    /// Number of equivalence groups with redundant expressions
    pub equivalence_groups: usize,
    /// Total number of todo items
    pub total_items: usize,
}

/// Complete todo analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoReport {
    /// Wrapper identifier
    pub wrapper: String,
    /// Path that was analyzed
    pub path: String,
    /// Prioritized list of improvement items
    pub items: Vec<TodoItem>,
    /// Results from each sub-analysis
    pub sub_results: HashMap<String, SubAnalysisResult>,
    /// Summary statistics
    pub summary: TodoSummary,
    /// Total elapsed time in milliseconds
    pub total_elapsed_ms: f64,
}

impl TodoReport {
    /// Create a new TodoReport
    pub fn new(path: &str) -> Self {
        Self {
            wrapper: "todo".to_string(),
            path: path.to_string(),
            items: Vec::new(),
            sub_results: HashMap::new(),
            summary: TodoSummary::default(),
            total_elapsed_ms: 0.0,
        }
    }

    /// Format the report as human-readable text
    pub fn to_text(&self) -> String {
        let s = &self.summary;
        let mut lines = vec![
            format!("TODO: Improvement Opportunities in {}", self.path),
            "=".repeat(50),
            format!("Dead Code:    {} functions to remove", s.dead_count),
            format!(
                "Duplication:  {} function pairs to consolidate",
                s.similar_pairs
            ),
            format!(
                "Low Cohesion: {} classes to consider splitting",
                s.low_cohesion_count
            ),
            format!("Complexity:   {} functions above CC>10", s.hotspot_count),
            format!(
                "Redundancy:   {} equivalent expression groups",
                s.equivalence_groups
            ),
            String::new(),
            "Priority Items:".to_string(),
        ];

        for (i, item) in self.items.iter().take(20).enumerate() {
            lines.push(format!(
                "  {}. [{}] {}",
                i + 1,
                item.category.to_uppercase(),
                item.description
            ));
            if !item.file.is_empty() {
                lines.push(format!("     {}:{}", item.file, item.line));
            }
        }

        if self.items.len() > 20 {
            lines.push(format!("  ... and {} more", self.items.len() - 20));
        }

        lines.push(String::new());
        lines.push(format!("Elapsed: {:.0}ms", self.total_elapsed_ms));

        lines.join("\n")
    }
}

// =============================================================================
// Main API
// =============================================================================

/// Run improvement-finding analyses and produce priority-sorted items
///
/// # Arguments
/// * `path` - Directory or file to analyze
/// * `lang` - Optional language filter (auto-detect if None)
/// * `quick` - If true, skip similarity analysis (expensive O(n^2))
///
/// # Returns
/// A `TodoReport` with prioritized improvement items
///
/// # Priority Mapping
/// - 1: Dead code
/// - 2: High complexity (CC > 20)
/// - 3: Low cohesion (LCOM4 > 2)
/// - 4: Similar functions
/// - 5: Equivalence/redundancy (GVN)
/// - 6: Medium complexity (CC > 10)
pub fn run_todo(path: &str, lang: Option<&str>, quick: bool) -> TldrResult<TodoReport> {
    let t0 = Instant::now();
    let mut report = TodoReport::new(path);
    let total = if quick { 4 } else { 5 };
    let mut step = 0;

    let language = lang.and_then(|l| Language::from_extension(&format!(".{}", l)));
    let target_path = Path::new(path);

    // --- dead code analysis ---
    step += 1;
    progress(step, total, "dead code");
    report.sub_results.insert(
        "dead".to_string(),
        safe_call("dead", || {
            let result = analyze_dead_code(target_path, language, &[])?;
            Ok(serde_json::to_value(&result)?)
        }),
    );

    // --- complexity analysis ---
    step += 1;
    progress(step, total, "complexity");
    report.sub_results.insert(
        "complexity".to_string(),
        safe_call("complexity", || {
            let opts = ComplexityOptions {
                hotspot_threshold: 10,
                max_hotspots: 100,
                include_cognitive: true,
            };
            let result = analyze_complexity(target_path, language, Some(opts))?;
            Ok(serde_json::to_value(&result)?)
        }),
    );

    // --- cohesion analysis ---
    step += 1;
    progress(step, total, "cohesion");
    report.sub_results.insert(
        "cohesion".to_string(),
        safe_call("cohesion", || {
            let opts = CohesionOptions::default();
            let result = analyze_cohesion_with_options(target_path, language, opts)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            Ok(serde_json::to_value(&result)?)
        }),
    );

    // --- equivalence analysis (GVN) ---
    step += 1;
    progress(step, total, "equivalence");
    report.sub_results.insert(
        "equivalence".to_string(),
        safe_call("equivalence", || {
            let results = run_equivalence_sweep(path)?;
            Ok(serde_json::to_value(&results)?)
        }),
    );

    // --- similarity analysis (full mode only) ---
    if !quick {
        step += 1;
        progress(step, total, "similar");
        report.sub_results.insert(
            "similar".to_string(),
            safe_call("similar", || {
                let opts = SimilarityOptions {
                    threshold: 0.7,
                    max_functions: 500,
                    max_pairs: 50,
                };
                let result = find_similar_with_options(target_path, language, &opts)
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                Ok(serde_json::to_value(&result)?)
            }),
        );
    }

    // --- build items and summary ---
    report.items = build_todo_items(&report);
    report.summary = build_todo_summary(&report);
    report.total_elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    Ok(report)
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Run GVN equivalence analysis on all Python files in path
fn run_equivalence_sweep(path: &str) -> TldrResult<Vec<serde_json::Value>> {
    let target = Path::new(path);
    let mut results: Vec<serde_json::Value> = Vec::new();

    if target.is_file() {
        // Single file
        if let Ok(source) = fs::read_to_string(target) {
            let reports = compute_gvn(&source, None);
            for r in reports {
                results.push(r.to_dict());
            }
        }
    } else {
        // Directory: scan for Python files (max 200)
        let python_files: Vec<PathBuf> = WalkDir::new(target)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().map(|ext| ext == "py").unwrap_or(false))
            .map(|e| e.path().to_path_buf())
            .take(200)
            .collect();

        for file_path in python_files {
            if let Ok(source) = fs::read_to_string(&file_path) {
                let reports = compute_gvn(&source, None);
                for r in reports {
                    results.push(r.to_dict());
                }
            }
        }
    }

    Ok(results)
}

/// Extract actionable items from sub-results and sort by priority
fn build_todo_items(report: &TodoReport) -> Vec<TodoItem> {
    let mut items: Vec<TodoItem> = Vec::new();

    // Dead code items (priority 1)
    if let Some(dead_r) = report.sub_results.get("dead") {
        if dead_r.success {
            if let Some(data) = &dead_r.data {
                if let Some(dead_funcs) = data.get("dead_functions").and_then(|v| v.as_array()) {
                    for func in dead_funcs.iter().take(50) {
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let file = func.get("file").and_then(|v| v.as_str()).unwrap_or("");
                        let line = func.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                        items.push(TodoItem::new(
                            "dead",
                            1,
                            &format!("Remove {}() - never called", name),
                            file,
                            line,
                            "low",
                            0.0,
                        ));
                    }
                }
            }
        }
    }

    // Complexity items (priority 2 for CC > 20, priority 6 for CC > 10)
    if let Some(comp_r) = report.sub_results.get("complexity") {
        if comp_r.success {
            if let Some(data) = &comp_r.data {
                if let Some(functions) = data.get("functions").and_then(|v| v.as_array()) {
                    for func in functions {
                        let cc =
                            func.get("cyclomatic").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let file = func.get("file").and_then(|v| v.as_str()).unwrap_or("");
                        let line = func.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

                        if cc > 20 {
                            items.push(TodoItem::new(
                                "complexity",
                                2,
                                &format!("Simplify {}() CC={}", name, cc),
                                file,
                                line,
                                "high",
                                cc as f64,
                            ));
                        } else if cc > 10 {
                            items.push(TodoItem::new(
                                "complexity",
                                6,
                                &format!("Consider simplifying {}() CC={}", name, cc),
                                file,
                                line,
                                "medium",
                                cc as f64,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Cohesion items (priority 3)
    if let Some(coh_r) = report.sub_results.get("cohesion") {
        if coh_r.success {
            if let Some(data) = &coh_r.data {
                if let Some(classes) = data.get("classes").and_then(|v| v.as_array()) {
                    for cls in classes {
                        let lcom4 = cls.get("lcom4").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                        let name = cls.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                        let file = cls.get("file").and_then(|v| v.as_str()).unwrap_or("");

                        if lcom4 > 2 {
                            items.push(TodoItem::new(
                                "cohesion",
                                3,
                                &format!("Consider splitting {} (LCOM4={})", name, lcom4),
                                file,
                                0,
                                "medium",
                                lcom4 as f64,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Similar pairs (priority 4)
    if let Some(sim_r) = report.sub_results.get("similar") {
        if sim_r.success {
            if let Some(data) = &sim_r.data {
                let pairs = data.get("similar_pairs").and_then(|v| v.as_array());
                if let Some(pairs) = pairs {
                    for pair in pairs.iter().take(20) {
                        let func_a = pair
                            .get("func_a")
                            .and_then(|v| v.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let func_b = pair
                            .get("func_b")
                            .and_then(|v| v.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        let score = pair.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        items.push(TodoItem::new(
                            "similar",
                            4,
                            &format!(
                                "Consolidate similar: {} ~ {} (score={:.2})",
                                func_a, func_b, score
                            ),
                            "",
                            0,
                            "medium",
                            score,
                        ));
                    }
                }
            }
        }
    }

    // Equivalence items (priority 5)
    if let Some(eq_r) = report.sub_results.get("equivalence") {
        if eq_r.success {
            if let Some(data) = &eq_r.data {
                if let Some(reports) = data.as_array() {
                    for eq_report in reports.iter().take(20) {
                        let func_name = eq_report
                            .get("function")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?");
                        if let Some(groups) =
                            eq_report.get("equivalences").and_then(|v| v.as_array())
                        {
                            for group in groups {
                                let exprs = group.get("expressions").and_then(|v| v.as_array());
                                if let Some(exprs) = exprs {
                                    if exprs.len() > 1 {
                                        items.push(TodoItem::new(
                                            "equivalence",
                                            5,
                                            &format!("Redundant expressions in {}", func_name),
                                            "",
                                            0,
                                            "low",
                                            0.0,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by priority
    items.sort_by_key(|item| item.priority);
    items
}

/// Build summary counts from sub-results
fn build_todo_summary(report: &TodoReport) -> TodoSummary {
    let mut summary = TodoSummary::default();

    // Dead count
    if let Some(dead_r) = report.sub_results.get("dead") {
        if dead_r.success {
            if let Some(data) = &dead_r.data {
                summary.dead_count =
                    data.get("dead_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            }
        }
    }

    // Hotspot count (CC > 10)
    if let Some(comp_r) = report.sub_results.get("complexity") {
        if comp_r.success {
            if let Some(data) = &comp_r.data {
                summary.hotspot_count = data
                    .get("hotspot_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
            }
        }
    }

    // Low cohesion count (LCOM4 > 2)
    if let Some(coh_r) = report.sub_results.get("cohesion") {
        if coh_r.success {
            if let Some(data) = &coh_r.data {
                if let Some(classes) = data.get("classes").and_then(|v| v.as_array()) {
                    summary.low_cohesion_count = classes
                        .iter()
                        .filter(|c| {
                            c.get("lcom4")
                                .and_then(|v| v.as_u64())
                                .map(|v| v > 2)
                                .unwrap_or(false)
                        })
                        .count();
                }
            }
        }
    }

    // Similar pairs count
    if let Some(sim_r) = report.sub_results.get("similar") {
        if sim_r.success {
            if let Some(data) = &sim_r.data {
                summary.similar_pairs = data
                    .get("similar_pairs_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
            }
        }
    }

    // Equivalence groups count
    if let Some(eq_r) = report.sub_results.get("equivalence") {
        if eq_r.success {
            if let Some(data) = &eq_r.data {
                if let Some(reports) = data.as_array() {
                    for eq_report in reports {
                        if let Some(groups) =
                            eq_report.get("equivalences").and_then(|v| v.as_array())
                        {
                            summary.equivalence_groups += groups
                                .iter()
                                .filter(|g| {
                                    g.get("expressions")
                                        .and_then(|v| v.as_array())
                                        .map(|e| e.len() > 1)
                                        .unwrap_or(false)
                                })
                                .count();
                        }
                    }
                }
            }
        }
    }

    summary.total_items = report.items.len();
    summary
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_item_new() {
        let item = TodoItem::new(
            "dead",
            1,
            "Remove foo() - never called",
            "src/main.py",
            42,
            "low",
            0.0,
        );

        assert_eq!(item.category, "dead");
        assert_eq!(item.priority, 1);
        assert_eq!(item.description, "Remove foo() - never called");
        assert_eq!(item.file, "src/main.py");
        assert_eq!(item.line, 42);
        assert_eq!(item.severity, "low");
        assert_eq!(item.score, 0.0);
    }

    #[test]
    fn test_todo_report_new() {
        let report = TodoReport::new("src/");

        assert_eq!(report.wrapper, "todo");
        assert_eq!(report.path, "src/");
        assert!(report.items.is_empty());
        assert!(report.sub_results.is_empty());
        assert_eq!(report.summary.total_items, 0);
    }

    #[test]
    fn test_todo_report_to_text() {
        let mut report = TodoReport::new("src/");
        report.items.push(TodoItem::new(
            "dead",
            1,
            "Remove unused_func() - never called",
            "src/main.py",
            10,
            "low",
            0.0,
        ));
        report.items.push(TodoItem::new(
            "complexity",
            2,
            "Simplify complex_func() CC=25",
            "src/utils.py",
            20,
            "high",
            25.0,
        ));
        report.summary = TodoSummary {
            dead_count: 1,
            hotspot_count: 1,
            low_cohesion_count: 0,
            similar_pairs: 0,
            equivalence_groups: 0,
            total_items: 2,
        };
        report.total_elapsed_ms = 123.456;

        let text = report.to_text();

        assert!(text.contains("TODO: Improvement Opportunities"));
        assert!(text.contains("Dead Code:    1 functions to remove"));
        assert!(text.contains("[DEAD] Remove unused_func()"));
        assert!(text.contains("[COMPLEXITY] Simplify complex_func()"));
        assert!(text.contains("src/main.py:10"));
        assert!(text.contains("Elapsed: 123ms"));
    }

    #[test]
    fn test_build_todo_items_empty() {
        let report = TodoReport::new("src/");
        let items = build_todo_items(&report);
        assert!(items.is_empty());
    }

    #[test]
    fn test_build_todo_items_priority_sort() {
        let mut report = TodoReport::new("src/");

        // Add a complexity result with high CC function
        let complexity_data = serde_json::json!({
            "functions": [
                {"name": "complex_func", "cyclomatic": 25, "file": "test.py", "line": 10}
            ]
        });
        report.sub_results.insert(
            "complexity".to_string(),
            SubAnalysisResult {
                name: "complexity".to_string(),
                success: true,
                data: Some(complexity_data),
                error: None,
                elapsed_ms: 10.0,
            },
        );

        // Add a dead code result
        let dead_data = serde_json::json!({
            "dead_functions": [
                {"name": "unused_func", "file": "test.py", "line": 5}
            ]
        });
        report.sub_results.insert(
            "dead".to_string(),
            SubAnalysisResult {
                name: "dead".to_string(),
                success: true,
                data: Some(dead_data),
                error: None,
                elapsed_ms: 10.0,
            },
        );

        let items = build_todo_items(&report);

        // Dead code (priority 1) should come before complexity (priority 2)
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].priority, 1);
        assert_eq!(items[0].category, "dead");
        assert_eq!(items[1].priority, 2);
        assert_eq!(items[1].category, "complexity");
    }

    #[test]
    fn test_build_todo_summary() {
        let mut report = TodoReport::new("src/");

        // Add dead code result
        let dead_data = serde_json::json!({
            "dead_count": 3,
            "dead_functions": []
        });
        report.sub_results.insert(
            "dead".to_string(),
            SubAnalysisResult {
                name: "dead".to_string(),
                success: true,
                data: Some(dead_data),
                error: None,
                elapsed_ms: 10.0,
            },
        );

        // Add complexity result
        let complexity_data = serde_json::json!({
            "hotspot_count": 5,
            "functions": []
        });
        report.sub_results.insert(
            "complexity".to_string(),
            SubAnalysisResult {
                name: "complexity".to_string(),
                success: true,
                data: Some(complexity_data),
                error: None,
                elapsed_ms: 10.0,
            },
        );

        let summary = build_todo_summary(&report);

        assert_eq!(summary.dead_count, 3);
        assert_eq!(summary.hotspot_count, 5);
    }

    #[test]
    fn test_run_equivalence_sweep_empty() {
        // Create a temp directory with no Python files
        let temp_dir = tempfile::TempDir::new().unwrap();
        let results = run_equivalence_sweep(temp_dir.path().to_str().unwrap()).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_run_equivalence_sweep_single_file() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.py");
        std::fs::write(
            &file_path,
            r#"
def foo():
    x = a + b
    y = a + b
    return x + y
"#,
        )
        .unwrap();

        let results = run_equivalence_sweep(file_path.to_str().unwrap()).unwrap();
        // Should have at least one report for the foo function
        assert!(!results.is_empty());
    }
}
