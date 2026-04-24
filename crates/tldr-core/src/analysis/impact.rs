//! Impact analysis (spec Section 2.2.2)
//!
//! Find all callers of a function via reverse call graph traversal.
//!
//! # Algorithm
//! 1. Build reverse graph (callee -> callers)
//! 2. Find all functions matching target_func
//! 3. BFS traversal up to max_depth
//! 4. Detect cycles (mark as truncated)
//!
//! # Edge Cases
//! - Function not in graph: Fall back to AST search if project root provided
//! - Function in AST but no edges: Return with caller_count: 0 and note
//! - Entry point (no callers): Return with caller_count: 0 and note
//! - Cycle detected: Mark as truncated: true
//! - Ambiguous name: Return all matches

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::extractor::{extract_functions, extract_methods};
use crate::ast::parser::parse_file;
use crate::error::TldrError;
use crate::fs::tree::{collect_files, get_file_tree};
use crate::types::{CallerTree, ImpactReport, ProjectCallGraph};
use crate::{Language, TldrResult};

/// Analyze impact of changing a function.
///
/// # Arguments
/// * `call_graph` - Project call graph
/// * `target_func` - Name of the function to analyze
/// * `max_depth` - Maximum traversal depth
/// * `target_file` - Optional file filter for disambiguation
///
/// # Returns
/// * `Ok(ImpactReport)` - Impact analysis results
/// * `Err(TldrError::FunctionNotFound)` - Function not found in graph
pub fn impact_analysis(
    call_graph: &ProjectCallGraph,
    target_func: &str,
    max_depth: usize,
    target_file: Option<&Path>,
) -> TldrResult<ImpactReport> {
    // Build reverse graph (callee -> callers)
    let reverse_graph = build_reverse_graph(call_graph);

    // Find all functions matching the target
    let mut targets: HashMap<String, CallerTree> = HashMap::new();
    let mut found_any = false;

    for edge in call_graph.edges() {
        // Check if this edge points to our target function
        if edge.dst_func == target_func || edge.dst_func.ends_with(&format!(".{}", target_func)) {
            // Apply file filter if provided
            if let Some(filter) = target_file {
                if !edge.dst_file.ends_with(filter) && edge.dst_file != filter {
                    continue;
                }
            }
            found_any = true;

            let key = format!("{}:{}", edge.dst_file.display(), edge.dst_func);
            targets.entry(key).or_insert_with(|| {
                // Build caller tree for this target
                build_caller_tree(&edge.dst_file, &edge.dst_func, &reverse_graph, max_depth)
            });
        }
    }

    // Also check if target is a callee (it might have no callers)
    if !found_any {
        // Look for the function as a source in any edge
        for edge in call_graph.edges() {
            if edge.src_func == target_func || edge.src_func.ends_with(&format!(".{}", target_func))
            {
                if let Some(filter) = target_file {
                    if !edge.src_file.ends_with(filter) && edge.src_file != filter {
                        continue;
                    }
                }
                let key = format!("{}:{}", edge.src_file.display(), edge.src_func);
                targets.entry(key).or_insert_with(|| {
                    build_caller_tree(&edge.src_file, &edge.src_func, &reverse_graph, max_depth)
                });
            }
        }
    }

    if targets.is_empty() {
        // Try to find similar function names for suggestions
        let suggestions = find_similar_functions(call_graph, target_func);
        return Err(TldrError::FunctionNotFound {
            name: target_func.to_string(),
            file: target_file.map(|p| p.to_path_buf()),
            suggestions,
        });
    }

    let total_targets = targets.len();
    Ok(ImpactReport {
        targets,
        total_targets,
        type_resolution: None, // Type-aware not enabled in basic analysis
    })
}

/// Impact analysis with AST fallback for isolated functions.
///
/// Tries normal call-graph-based impact analysis first. If the function is not
/// found in the call graph (no edges at all), falls back to AST-based function
/// discovery. This handles the case where a function exists in the codebase but
/// has no callers or callees within the analyzed scope.
///
/// # Arguments
/// * `call_graph` - Project call graph
/// * `target_func` - Name of the function to analyze
/// * `max_depth` - Maximum traversal depth
/// * `target_file` - Optional file filter for disambiguation
/// * `project_root` - Root directory for AST-based fallback search
/// * `language` - Programming language for AST parsing
///
/// # Returns
/// * `Ok(ImpactReport)` - Impact analysis results (possibly with zero callers via AST fallback)
/// * `Err(TldrError::FunctionNotFound)` - Function not found in graph or AST
pub fn impact_analysis_with_ast_fallback(
    call_graph: &ProjectCallGraph,
    target_func: &str,
    max_depth: usize,
    target_file: Option<&Path>,
    project_root: &Path,
    language: Language,
) -> TldrResult<ImpactReport> {
    // Try normal call-graph-based analysis first
    match impact_analysis(call_graph, target_func, max_depth, target_file) {
        Ok(report) => Ok(report),
        Err(TldrError::FunctionNotFound {
            name,
            file,
            suggestions,
        }) => {
            // Call graph lookup failed -- try AST-based discovery
            match find_function_in_ast(project_root, target_func, target_file, language) {
                Some(locations) => {
                    // Function exists in AST but has no call edges
                    let mut targets = HashMap::new();
                    for (func_name, func_file) in &locations {
                        let key = format!("{}:{}", func_file.display(), func_name);
                        targets.insert(
                            key,
                            CallerTree {
                                function: func_name.clone(),
                                file: func_file.clone(),
                                caller_count: 0,
                                callers: vec![],
                                truncated: false,
                                note: Some(
                                    "Function found via AST but has no call edges in analyzed scope"
                                        .to_string(),
                                ),
                                confidence: None,
                                receiver_type: None,
                            },
                        );
                    }
                    let total_targets = targets.len();
                    Ok(ImpactReport {
                        targets,
                        total_targets,
                        type_resolution: None,
                    })
                }
                None => {
                    // Not in AST either -- propagate original error
                    Err(TldrError::FunctionNotFound {
                        name,
                        file,
                        suggestions,
                    })
                }
            }
        }
        Err(other) => Err(other),
    }
}

/// Search for a function in the AST of files under `root`.
///
/// Returns a list of (function_name, file_path) pairs if found, or None.
fn find_function_in_ast(
    root: &Path,
    target_func: &str,
    target_file: Option<&Path>,
    language: Language,
) -> Option<Vec<(String, PathBuf)>> {
    let extensions: HashSet<String> = language
        .extensions()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let files = if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        match get_file_tree(root, Some(&extensions), true, None) {
            Ok(tree) => collect_files(&tree, root),
            Err(_) => return None,
        }
    };

    let mut found: Vec<(String, PathBuf)> = Vec::new();

    for file_path in &files {
        // Apply target_file filter if provided
        if let Some(filter) = target_file {
            if !file_path.ends_with(filter) && file_path.as_path() != filter {
                continue;
            }
        }

        // Parse the file
        let (tree, source, _detected_lang) = match parse_file(file_path) {
            Ok(result) => result,
            Err(_) => continue,
        };

        // Extract functions and methods
        let functions = extract_functions(&tree, &source, language);
        let methods = extract_methods(&tree, &source, language);

        for func_name in functions.iter().chain(methods.iter()) {
            // Match: exact name or Class.method suffix
            if func_name == target_func || func_name.ends_with(&format!(".{}", target_func)) {
                found.push((func_name.clone(), file_path.clone()));
            }
        }
    }

    if found.is_empty() {
        None
    } else {
        Some(found)
    }
}

/// Key for the reverse graph: (file, function)
type FunctionKey = (std::path::PathBuf, String);

/// Build reverse graph: (dst_file, dst_func) -> [(src_file, src_func)]
fn build_reverse_graph(call_graph: &ProjectCallGraph) -> HashMap<FunctionKey, Vec<FunctionKey>> {
    let mut reverse: HashMap<FunctionKey, Vec<FunctionKey>> = HashMap::new();

    for edge in call_graph.edges() {
        let dst_key = (edge.dst_file.clone(), edge.dst_func.clone());
        let src_key = (edge.src_file.clone(), edge.src_func.clone());

        reverse.entry(dst_key).or_default().push(src_key);
    }

    reverse
}

/// Build caller tree via BFS traversal
fn build_caller_tree(
    file: &Path,
    func: &str,
    reverse_graph: &HashMap<FunctionKey, Vec<FunctionKey>>,
    max_depth: usize,
) -> CallerTree {
    let key = (file.to_path_buf(), func.to_string());

    // Get direct callers
    let callers = reverse_graph.get(&key);
    let caller_count = callers.map(|c| c.len()).unwrap_or(0);

    // Handle entry point (no callers)
    if caller_count == 0 {
        return CallerTree {
            function: func.to_string(),
            file: file.to_path_buf(),
            caller_count: 0,
            callers: vec![],
            truncated: false,
            note: Some("Entry point - no callers found".to_string()),
            confidence: None,
            receiver_type: None,
        };
    }

    // BFS traversal with depth tracking
    let mut visited: HashSet<FunctionKey> = HashSet::new();
    visited.insert(key.clone());

    let mut child_trees = Vec::new();

    if max_depth > 0 {
        if let Some(callers) = callers {
            for (caller_file, caller_func) in callers {
                let caller_key = (caller_file.clone(), caller_func.clone());

                // Cycle detection
                if visited.contains(&caller_key) {
                    child_trees.push(CallerTree {
                        function: caller_func.clone(),
                        file: caller_file.clone(),
                        caller_count: 0,
                        callers: vec![],
                        truncated: true,
                        note: Some("Cycle detected".to_string()),
                        confidence: None,
                        receiver_type: None,
                    });
                    continue;
                }

                visited.insert(caller_key);

                // Recursively build subtree with reduced depth
                let subtree =
                    build_caller_tree(caller_file, caller_func, reverse_graph, max_depth - 1);
                child_trees.push(subtree);
            }
        }
    }

    CallerTree {
        function: func.to_string(),
        file: file.to_path_buf(),
        caller_count,
        callers: child_trees,
        truncated: max_depth == 0 && caller_count > 0,
        note: if max_depth == 0 && caller_count > 0 {
            Some(format!(
                "Truncated at depth limit ({} callers)",
                caller_count
            ))
        } else {
            None
        },
        confidence: None,
        receiver_type: None,
    }
}

/// Find similar function names for error suggestions
fn find_similar_functions(call_graph: &ProjectCallGraph, target: &str) -> Vec<String> {
    let mut all_functions: HashSet<String> = HashSet::new();

    for edge in call_graph.edges() {
        all_functions.insert(edge.src_func.clone());
        all_functions.insert(edge.dst_func.clone());
    }

    // Find functions with similar names (simple substring/prefix matching)
    let target_lower = target.to_lowercase();
    let mut suggestions: Vec<String> = all_functions
        .into_iter()
        .filter(|f| {
            let f_lower = f.to_lowercase();
            f_lower.contains(&target_lower)
                || target_lower.contains(&f_lower)
                || levenshtein_distance(&f_lower, &target_lower) <= 3
        })
        .take(5)
        .collect();

    suggestions.sort();
    suggestions
}

/// Simple Levenshtein distance for fuzzy matching
fn levenshtein_distance(s1: &str, s2: &str) -> usize {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();

    if len1 == 0 {
        return len2;
    }
    if len2 == 0 {
        return len1;
    }

    let mut matrix: Vec<Vec<usize>> = vec![vec![0; len2 + 1]; len1 + 1];

    for (i, row) in matrix.iter_mut().enumerate().take(len1 + 1) {
        row[0] = i;
    }
    for (j, val) in matrix[0].iter_mut().enumerate().take(len2 + 1) {
        *val = j;
    }

    for (i, c1) in s1.chars().enumerate() {
        for (j, c2) in s2.chars().enumerate() {
            let cost = if c1 == c2 { 0 } else { 1 };
            matrix[i + 1][j + 1] = std::cmp::min(
                std::cmp::min(matrix[i][j + 1] + 1, matrix[i + 1][j] + 1),
                matrix[i][j] + cost,
            );
        }
    }

    matrix[len1][len2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CallEdge;

    fn create_test_graph() -> ProjectCallGraph {
        let mut graph = ProjectCallGraph::new();

        // A calls B, B calls C
        graph.add_edge(CallEdge {
            src_file: "a.py".into(),
            src_func: "func_a".to_string(),
            dst_file: "b.py".into(),
            dst_func: "func_b".to_string(),
        });
        graph.add_edge(CallEdge {
            src_file: "b.py".into(),
            src_func: "func_b".to_string(),
            dst_file: "c.py".into(),
            dst_func: "func_c".to_string(),
        });
        // D also calls C
        graph.add_edge(CallEdge {
            src_file: "d.py".into(),
            src_func: "func_d".to_string(),
            dst_file: "c.py".into(),
            dst_func: "func_c".to_string(),
        });

        graph
    }

    #[test]
    fn test_impact_finds_direct_callers() {
        let graph = create_test_graph();
        let result = impact_analysis(&graph, "func_c", 1, None).unwrap();

        assert_eq!(result.total_targets, 1);
        let tree = result.targets.values().next().unwrap();
        assert_eq!(tree.caller_count, 2); // func_b and func_d
    }

    #[test]
    fn test_impact_respects_depth() {
        let graph = create_test_graph();

        // Depth 1 should only show direct callers
        let result = impact_analysis(&graph, "func_c", 1, None).unwrap();
        let tree = result.targets.values().next().unwrap();

        // At depth 1, callers of func_c (func_b, func_d) are shown
        // but their callers should be truncated
        assert_eq!(tree.callers.len(), 2);
    }

    #[test]
    fn test_impact_handles_not_found() {
        let graph = create_test_graph();
        let result = impact_analysis(&graph, "nonexistent", 3, None);

        assert!(result.is_err());
        if let Err(TldrError::FunctionNotFound { name, .. }) = result {
            assert_eq!(name, "nonexistent");
        } else {
            panic!("Expected FunctionNotFound error");
        }
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
    }

    // =========================================================================
    // AST fallback tests
    // =========================================================================

    #[test]
    fn test_impact_ast_fallback_finds_isolated_function() {
        // Function exists in AST but has no call edges
        let graph = ProjectCallGraph::new(); // empty graph
        let dir = std::env::temp_dir().join("tldr_impact_test_isolated");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("isolated.go"),
            "package main\n\nfunc CreateIssue() {\n\tprintln(\"hello\")\n}\n",
        )
        .unwrap();

        let result = impact_analysis_with_ast_fallback(
            &graph,
            "CreateIssue",
            5,
            None,
            &dir,
            crate::Language::Go,
        );

        assert!(
            result.is_ok(),
            "Should succeed via AST fallback, got: {:?}",
            result
        );
        let report = result.unwrap();
        assert_eq!(report.total_targets, 1);
        let tree = report.targets.values().next().unwrap();
        assert_eq!(tree.function, "CreateIssue");
        assert_eq!(tree.caller_count, 0);
        assert!(
            tree.note.as_ref().unwrap().contains("no call edges"),
            "Note should mention no call edges, got: {:?}",
            tree.note
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_impact_ast_fallback_returns_correct_file() {
        // Function exists in a specific file; verify the file path is set
        let graph = ProjectCallGraph::new();
        let dir = std::env::temp_dir().join("tldr_impact_test_file");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("handler.py"), "def create_handler():\n    pass\n").unwrap();

        let result = impact_analysis_with_ast_fallback(
            &graph,
            "create_handler",
            5,
            None,
            &dir,
            crate::Language::Python,
        );

        assert!(result.is_ok());
        let report = result.unwrap();
        let tree = report.targets.values().next().unwrap();
        // File path should reference handler.py
        let file_str = tree.file.to_string_lossy();
        assert!(
            file_str.contains("handler.py"),
            "Expected file path to contain handler.py, got: {}",
            file_str
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_impact_ast_fallback_not_triggered_when_graph_has_function() {
        // If function is in the call graph, don't fall back to AST
        let graph = create_test_graph();

        let dir = std::env::temp_dir().join("tldr_impact_test_no_fallback");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("c.py"), "def func_c():\n    pass\n").unwrap();

        let result = impact_analysis_with_ast_fallback(
            &graph,
            "func_c",
            3,
            None,
            &dir,
            crate::Language::Python,
        );

        assert!(result.is_ok());
        let report = result.unwrap();
        let tree = report.targets.values().next().unwrap();
        // Should have actual callers (func_b and func_d), not a zero-caller fallback
        assert_eq!(tree.caller_count, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_impact_ast_fallback_still_errors_when_truly_not_found() {
        // Function doesn't exist in graph OR AST - should still error
        let graph = ProjectCallGraph::new();
        let dir = std::env::temp_dir().join("tldr_impact_test_truly_missing");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("other.py"), "def something_else():\n    pass\n").unwrap();

        let result = impact_analysis_with_ast_fallback(
            &graph,
            "nonexistent_function",
            5,
            None,
            &dir,
            crate::Language::Python,
        );

        assert!(result.is_err());
        if let Err(TldrError::FunctionNotFound { name, .. }) = result {
            assert_eq!(name, "nonexistent_function");
        } else {
            panic!("Expected FunctionNotFound error");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_impact_ast_fallback_finds_method() {
        // Method inside a class should also be found via AST fallback
        let graph = ProjectCallGraph::new();
        let dir = std::env::temp_dir().join("tldr_impact_test_method");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(
            dir.join("service.py"),
            "class MyService:\n    def handle_request(self):\n        pass\n",
        )
        .unwrap();

        let result = impact_analysis_with_ast_fallback(
            &graph,
            "handle_request",
            5,
            None,
            &dir,
            crate::Language::Python,
        );

        assert!(result.is_ok());
        let report = result.unwrap();
        assert_eq!(report.total_targets, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
