//! Call graph builder (spec Section 2.2.1)
//!
//! This module is a thin wrapper around the V2 builder (Phase 14),
//! which implements the full cross-file call graph pipeline.

use std::path::Path;

use crate::types::{CallEdge, FunctionInfo, Language, ProjectCallGraph, WorkspaceConfig};
use crate::TldrResult;

use super::builder_v2::{build_project_call_graph_v2, BuildConfig, BuildError};
use super::cross_file_types::CallGraphIR;

/// Build project-wide call graph.
///
/// # Arguments
/// * `root` - Project root directory
/// * `language` - Programming language
/// * `workspace_config` - Optional workspace configuration for multi-root projects
/// * `respect_ignore` - Whether to respect .gitignore/.tldrignore patterns
///
/// # Returns
/// * `Ok(ProjectCallGraph)` - The constructed call graph
/// * `Err(TldrError)` - On file system or parse errors
///
/// # Performance
/// - Target: <5s for 10K LOC
/// - Target: <30s for 100K LOC
pub fn build_project_call_graph(
    root: &Path,
    language: Language,
    workspace_config: Option<&WorkspaceConfig>,
    respect_ignore: bool,
) -> TldrResult<ProjectCallGraph> {
    let mut config = BuildConfig {
        language: language.as_str().to_string(),
        respect_ignore,
        ..Default::default()
    };
    config.use_type_resolution = true;

    if let Some(config_roots) = workspace_config {
        if !config_roots.roots.is_empty() {
            config.use_workspace_config = true;
            config.workspace_roots = config_roots.roots.clone();
        }
    }

    let ir = build_project_call_graph_v2(root, config).map_err(map_build_error)?;
    Ok(project_graph_from_ir(ir))
}

fn project_graph_from_ir(ir: CallGraphIR) -> ProjectCallGraph {
    let mut graph = ProjectCallGraph::new();
    for edge in ir.edges {
        graph.add_edge(CallEdge {
            src_file: edge.src_file,
            src_func: edge.src_func,
            dst_file: edge.dst_file,
            dst_func: edge.dst_func,
        });
    }
    graph
}

/// Convert a `CallGraphIR` reference to `ProjectCallGraph` without consuming the IR.
///
/// This is useful when the IR needs to be shared between multiple consumers
/// (e.g., coupling analysis needs `ProjectCallGraph` while Tier-2 smell
/// detectors need `CallGraphIR`).
pub fn project_graph_from_ir_ref(ir: &CallGraphIR) -> ProjectCallGraph {
    let mut graph = ProjectCallGraph::new();
    for edge in &ir.edges {
        graph.add_edge(CallEdge {
            src_file: edge.src_file.clone(),
            src_func: edge.src_func.clone(),
            dst_file: edge.dst_file.clone(),
            dst_func: edge.dst_func.clone(),
        });
    }
    graph
}

fn map_build_error(err: BuildError) -> crate::error::TldrError {
    use crate::error::TldrError;
    match err {
        BuildError::RootNotFound(path) => TldrError::PathNotFound(path),
        BuildError::UnsupportedLanguage(lang) => TldrError::UnsupportedLanguage(lang),
        BuildError::WorkspaceConfig(message) => TldrError::InvalidArgs {
            arg: "workspace_config".to_string(),
            message,
            suggestion: None,
        },
        BuildError::Io(err) => TldrError::IoError(err),
        BuildError::ParseError { file, message } => TldrError::ParseError {
            file,
            line: None,
            message,
        },
        BuildError::ThreadPool(message) => TldrError::InvalidArgs {
            arg: "thread_pool".to_string(),
            message,
            suggestion: None,
        },
        BuildError::FeatureNotEnabled { feature, message } => TldrError::InvalidArgs {
            arg: feature,
            message,
            suggestion: None,
        },
    }
}

/// Check if a function is likely an entry point based on name/decorators (M23)
pub fn is_entry_point(func: &FunctionInfo) -> bool {
    let name = &func.name;

    // Standard entry point names
    if matches!(
        name.as_str(),
        "main" | "__main__" | "cli" | "app" | "run" | "start" | "setup" | "teardown"
    ) {
        return true;
    }

    // Test functions
    if name.starts_with("test_") || name.starts_with("pytest_") {
        return true;
    }

    // Dunder methods (likely called by Python runtime)
    if name.starts_with("__") && name.ends_with("__") {
        return true;
    }

    // Check decorators for framework entry points
    for decorator in &func.decorators {
        // Flask/FastAPI routes
        if decorator.contains("route")
            || decorator.contains("get")
            || decorator.contains("post")
            || decorator.contains("put")
            || decorator.contains("delete")
            || decorator.contains("patch")
        {
            return true;
        }

        // pytest fixtures
        if decorator.contains("fixture")
            || decorator.contains("pytest")
            || decorator.contains("parametrize")
        {
            return true;
        }

        // Click CLI
        if decorator.contains("command") || decorator.contains("option") {
            return true;
        }

        // Property/staticmethod/classmethod (called by Python)
        if decorator == "property"
            || decorator == "staticmethod"
            || decorator == "classmethod"
            || decorator == "abstractmethod"
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_entry_point() {
        let main_func = FunctionInfo {
            name: "main".to_string(),
            params: vec![],
            return_type: None,
            docstring: None,
            is_method: false,
            is_async: false,
            decorators: vec![],
            line_number: 1,
        };
        assert!(is_entry_point(&main_func));

        let test_func = FunctionInfo {
            name: "test_something".to_string(),
            params: vec![],
            return_type: None,
            docstring: None,
            is_method: false,
            is_async: false,
            decorators: vec![],
            line_number: 1,
        };
        assert!(is_entry_point(&test_func));

        let route_func = FunctionInfo {
            name: "get_user".to_string(),
            params: vec![],
            return_type: None,
            docstring: None,
            is_method: false,
            is_async: false,
            decorators: vec!["app.route(\"/user\")".to_string()],
            line_number: 1,
        };
        assert!(is_entry_point(&route_func));

        let normal_func = FunctionInfo {
            name: "helper".to_string(),
            params: vec![],
            return_type: None,
            docstring: None,
            is_method: false,
            is_async: false,
            decorators: vec![],
            line_number: 1,
        };
        assert!(!is_entry_point(&normal_func));
    }

    #[test]
    fn test_project_graph_from_ir_ref_empty() {
        use crate::callgraph::cross_file_types::CallGraphIR;
        use std::path::PathBuf;

        let ir = CallGraphIR::new(PathBuf::from("/tmp"), "python");
        let graph = project_graph_from_ir_ref(&ir);
        assert_eq!(graph.edges().count(), 0);
        // Verify the IR is not consumed - we can still access it
        assert_eq!(ir.language, "python");
    }

    #[test]
    fn test_project_graph_from_ir_ref_preserves_edges() {
        use crate::callgraph::cross_file_types::{CallGraphIR, CallType, CrossFileCallEdge};
        use std::path::PathBuf;

        let mut ir = CallGraphIR::new(PathBuf::from("/tmp"), "python");
        ir.edges.push(CrossFileCallEdge {
            src_file: PathBuf::from("src/a.py"),
            src_func: "func_a".to_string(),
            dst_file: PathBuf::from("src/b.py"),
            dst_func: "func_b".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });
        ir.edges.push(CrossFileCallEdge {
            src_file: PathBuf::from("src/b.py"),
            src_func: "func_b".to_string(),
            dst_file: PathBuf::from("src/c.py"),
            dst_func: "func_c".to_string(),
            call_type: CallType::Method,
            via_import: Some("c".to_string()),
        });

        let graph = project_graph_from_ir_ref(&ir);

        // Verify edges are converted
        let edges: Vec<_> = graph.edges().collect();
        assert_eq!(edges.len(), 2);

        // Verify IR is still available (not consumed)
        assert_eq!(ir.edges.len(), 2);
        assert_eq!(ir.edges[0].src_func, "func_a");
        assert_eq!(ir.edges[1].dst_func, "func_c");
    }

    #[test]
    fn test_project_graph_from_ir_ref_matches_consuming_version() {
        use crate::callgraph::cross_file_types::{CallGraphIR, CallType, CrossFileCallEdge};
        use std::path::PathBuf;

        // Build two identical IRs
        let make_ir = || {
            let mut ir = CallGraphIR::new(PathBuf::from("/tmp"), "python");
            ir.edges.push(CrossFileCallEdge {
                src_file: PathBuf::from("src/a.py"),
                src_func: "foo".to_string(),
                dst_file: PathBuf::from("src/b.py"),
                dst_func: "bar".to_string(),
                call_type: CallType::Direct,
                via_import: None,
            });
            ir
        };

        let ir1 = make_ir();
        let ir2 = make_ir();

        // Compare borrowing vs consuming
        let graph_ref = project_graph_from_ir_ref(&ir1);
        let graph_own = project_graph_from_ir(ir2);

        let edges_ref: Vec<_> = graph_ref.edges().collect();
        let edges_own: Vec<_> = graph_own.edges().collect();
        assert_eq!(edges_ref.len(), edges_own.len());

        // Both should have the same edge data
        assert_eq!(edges_ref[0].src_file, edges_own[0].src_file);
        assert_eq!(edges_ref[0].src_func, edges_own[0].src_func);
        assert_eq!(edges_ref[0].dst_file, edges_own[0].dst_file);
        assert_eq!(edges_ref[0].dst_func, edges_own[0].dst_func);
    }
}
