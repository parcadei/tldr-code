//! Tests for Phase 16: API Compatibility
//!
//! These tests verify the compatibility layer implementation per
//! `migration/spec/phases-14-16-spec.md` Section 16.
//!
//! All tests are designed to fail initially (red phase of TDD) since
//! the implementation does not exist yet. They will pass once the
//! `compat` module is implemented.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// Types from compat module (to be created)
use super::compat::{
    callgraph_ir_to_old, compare_builders, format_edges_compatible, funcdef_to_functioninfo,
    importdef_to_importinfo, project_graph_to_edges, ComparisonResult, NormalizedEdge,
};

// New IR types
use super::builder_v2::{build_project_call_graph_v2, BuildConfig};
use super::cross_file_types::{
    CallGraphIR, CallSite, CallType, CrossFileCallEdge, FileIRBuilder, FuncDef, ImportDef,
    ProjectCallGraphV2,
};

// Old types from types.rs (CallEdge no longer needed, using NormalizedEdge instead)
#[allow(unused_imports)]
use crate::types::{FunctionInfo, ProjectCallGraph};

// =============================================================================
// Test Fixtures
// =============================================================================

/// Creates a FuncDef for testing conversion.
fn create_test_funcdef() -> FuncDef {
    FuncDef {
        name: "my_method".to_string(),
        line: 10,
        end_line: 25,
        is_method: true,
        class_name: Some("MyClass".to_string()),
        return_type: Some("str".to_string()),
        parent_function: None,
    }
}

/// Creates a simple function FuncDef.
fn create_simple_funcdef() -> FuncDef {
    FuncDef::function("simple_func", 1, 5)
}

/// Creates an ImportDef for testing conversion.
fn create_test_importdef() -> ImportDef {
    ImportDef::from_import(
        "mymodule",
        vec!["MyClass".to_string(), "helper".to_string()],
    )
}

/// Creates a ProjectCallGraphV2 for testing.
fn create_test_project_graph() -> ProjectCallGraphV2 {
    let mut graph = ProjectCallGraphV2::new();

    graph.add_edge(CrossFileCallEdge {
        src_file: PathBuf::from("main.py"),
        src_func: "main".to_string(),
        dst_file: PathBuf::from("helper.py"),
        dst_func: "process".to_string(),
        call_type: CallType::Direct,
        via_import: Some("helper".to_string()),
    });

    graph.add_edge(CrossFileCallEdge {
        src_file: PathBuf::from("main.py"),
        src_func: "main".to_string(),
        dst_file: PathBuf::from("utils.py"),
        dst_func: "validate".to_string(),
        call_type: CallType::Direct,
        via_import: Some("utils".to_string()),
    });

    graph
}

/// Creates a temporary Python project for builder comparison.
fn create_comparison_project() -> TempDir {
    let dir = TempDir::new().unwrap();

    // main.py
    let main_py = r#"
from helper import process
from utils import validate

def main():
    process()
    validate("test")

if __name__ == "__main__":
    main()
"#;
    fs::write(dir.path().join("main.py"), main_py).unwrap();

    // helper.py
    let helper_py = r#"
def process():
    print("processing")
"#;
    fs::write(dir.path().join("helper.py"), helper_py).unwrap();

    // utils.py
    let utils_py = r#"
def validate(data):
    return len(data) > 0
"#;
    fs::write(dir.path().join("utils.py"), utils_py).unwrap();

    dir
}

// =============================================================================
// Phase 16.2: Type Conversion Tests
// =============================================================================

mod type_conversion {
    use super::*;

    /// Test: FuncDef converts to FunctionInfo correctly.
    /// Spec Section 16.3: "funcdef_to_functioninfo"
    #[test]
    fn test_funcdef_to_functioninfo() {
        let func = create_test_funcdef();
        let file = "module.py";

        let info = funcdef_to_functioninfo(&func, file);

        assert_eq!(info.name, "my_method");
        assert_eq!(info.file, "module.py");
        assert_eq!(info.start_line, 10);
        assert_eq!(info.end_line, 25);
        assert!(info.is_method);
        assert_eq!(info.class_name, Some("MyClass".to_string()));
    }

    /// Test: Simple function FuncDef conversion.
    #[test]
    fn test_funcdef_to_functioninfo_simple() {
        let func = create_simple_funcdef();
        let file = "simple.py";

        let info = funcdef_to_functioninfo(&func, file);

        assert_eq!(info.name, "simple_func");
        assert_eq!(info.file, "simple.py");
        assert!(!info.is_method);
        assert_eq!(info.class_name, None);
    }

    /// Test: ImportDef converts to ImportInfo correctly.
    #[test]
    fn test_importdef_to_importinfo() {
        let import = create_test_importdef();

        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "mymodule");
        assert!(info.is_from);
        assert_eq!(info.names, vec!["MyClass", "helper"]);
    }

    /// Test: Simple import conversion.
    #[test]
    fn test_importdef_to_importinfo_simple() {
        let import = ImportDef::simple_import("json");

        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "json");
        assert!(!info.is_from);
    }

    /// Test: Import with alias conversion.
    #[test]
    fn test_importdef_to_importinfo_with_alias() {
        let import = ImportDef::import_as("numpy", "np");

        let info = importdef_to_importinfo(&import);

        assert_eq!(info.module, "numpy");
        assert_eq!(info.alias, Some("np".to_string()));
    }
}

// =============================================================================
// Phase 16.2: Graph Conversion Tests
// =============================================================================

mod graph_conversion {
    use super::*;

    /// Test: ProjectCallGraphV2 converts to CallEdge vector.
    /// Spec Section 16.2: "project_graph_to_edges"
    #[test]
    fn test_project_graph_to_edges() {
        let graph = create_test_project_graph();

        // Create file IRs for line number lookup
        let mut file_irs = std::collections::HashMap::new();
        file_irs.insert(
            "main.py".to_string(),
            FileIRBuilder::new(PathBuf::from("main.py"))
                .call(CallSite::direct("main", "process", Some(6)))
                .call(CallSite::direct("main", "validate", Some(7)))
                .build(),
        );

        let edges = project_graph_to_edges(&graph, &file_irs);

        assert_eq!(edges.len(), 2, "Should have 2 edges");

        // Verify edge format
        let edge = &edges[0];
        assert!(
            edge.caller.contains("main.py"),
            "Caller should include file"
        );
        assert!(
            edge.caller.contains("main"),
            "Caller should include function"
        );
    }

    /// Test: Edge format matches spec.
    /// Spec: "caller = {src_file}:{src_func}, callee = {dst_file}:{dst_func}"
    #[test]
    fn test_edge_format() {
        let mut graph = ProjectCallGraphV2::new();
        graph.add_edge(CrossFileCallEdge {
            src_file: PathBuf::from("src/main.py"),
            src_func: "main".to_string(),
            dst_file: PathBuf::from("src/helper.py"),
            dst_func: "process".to_string(),
            call_type: CallType::Direct,
            via_import: None,
        });

        let file_irs = std::collections::HashMap::new();
        let edges = project_graph_to_edges(&graph, &file_irs);

        let edge = &edges[0];
        // Format should be "file:func"
        assert!(
            edge.caller.contains(":"),
            "Caller should be file:func format"
        );
        assert!(
            edge.callee.contains(":"),
            "Callee should be file:func format"
        );
    }

    /// Test: CallGraphIR converts to old CallGraph format.
    #[test]
    fn test_callgraph_ir_to_old() {
        let mut ir = CallGraphIR::new(PathBuf::from("/project"), "python");

        let file_ir = FileIRBuilder::new(PathBuf::from("main.py"))
            .func(FuncDef::function("main", 1, 10))
            .call(CallSite::direct("main", "helper", Some(5)))
            .build();
        ir.add_file(file_ir);
        ir.build_indices();

        let old_graph = callgraph_ir_to_old(&ir);

        // Old graph should exist (may or may not have edges depending on resolution)
        let _ = old_graph;
    }
}

// =============================================================================
// Phase 16.3: Builder Comparison Tests
// =============================================================================

mod builder_comparison {
    use super::*;

    /// Test: compare_builders returns valid ComparisonResult.
    /// Spec Section 16.3: "compare_builders"
    #[test]
    fn test_compare_builders_result_structure() {
        let dir = create_comparison_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let result = compare_builders(dir.path(), &config);

        // Result should have the expected structure
        assert!(result.is_ok(), "Comparison should succeed");

        let comparison = result.unwrap();

        // ComparisonResult should have these fields (using NormalizedEdge per M3.4)
        let _only_old: &HashSet<NormalizedEdge> = &comparison.only_in_old;
        let _only_new: &HashSet<NormalizedEdge> = &comparison.only_in_new;
        let _in_both: &HashSet<NormalizedEdge> = &comparison.in_both;
    }

    /// Test: Identical results when both builders work correctly.
    /// Spec: "V2 should find at least as many edges as V1"
    #[test]
    fn test_compare_builders_identical() {
        let dir = create_comparison_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let comparison = compare_builders(dir.path(), &config).unwrap();

        // V2 should not miss any edges that V1 found
        assert!(
            comparison.only_in_old.is_empty(),
            "V2 should find all edges that V1 finds. Missing: {:?}",
            comparison.only_in_old
        );
    }

    /// Test: V2 may find additional edges (more complete).
    #[test]
    fn test_compare_builders_v2_may_find_more() {
        let dir = create_comparison_project();
        let config = BuildConfig {
            language: "python".to_string(),
            use_type_resolution: true, // V2 feature
            ..Default::default()
        };

        let comparison = compare_builders(dir.path(), &config).unwrap();

        // V2 with type resolution may find additional method calls
        // This is expected and acceptable
        // The important thing is only_in_old should be empty
        assert!(
            comparison.only_in_old.is_empty(),
            "V2 should not miss V1 edges"
        );
    }
}

// =============================================================================
// Phase 16.4: CLI Integration Tests
// =============================================================================

mod cli_integration {
    use super::*;

    /// Test: experimental_callgraph flag routes to V2.
    /// Spec Section 16.5: "Feature Flag Implementation"
    #[test]
    fn test_experimental_flag_routing() {
        let dir = create_comparison_project();

        // With experimental flag = false, use V1
        let _config_v1 = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        // With experimental flag = true, use V2
        let config_v2 = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        // Both should produce valid results
        // The actual routing is tested at CLI level
        let result = build_project_call_graph_v2(dir.path(), config_v2);
        assert!(result.is_ok(), "V2 builder should work");
    }
}

// =============================================================================
// Phase 16.7: Output Format Compatibility Tests
// =============================================================================

mod output_format {
    use super::*;

    /// Test: format_edges_compatible produces V1-compatible output.
    /// Spec Section 16.7: "Output format matches existing CLI output"
    #[test]
    fn test_format_edges_compatible() {
        let edges = vec![
            (
                "main.py".to_string(),
                "main".to_string(),
                "helper.py".to_string(),
                "process".to_string(),
            ),
            (
                "main.py".to_string(),
                "main".to_string(),
                "utils.py".to_string(),
                "validate".to_string(),
            ),
        ];

        let output = format_edges_compatible(&edges);

        // Expected format: "file:func -> file:func"
        assert!(output.contains("main.py:main -> helper.py:process"));
        assert!(output.contains("main.py:main -> utils.py:validate"));
    }

    /// Test: Output is sorted for determinism.
    #[test]
    fn test_format_edges_sorted() {
        let edges = vec![
            (
                "z.py".to_string(),
                "z".to_string(),
                "a.py".to_string(),
                "a".to_string(),
            ),
            (
                "a.py".to_string(),
                "a".to_string(),
                "b.py".to_string(),
                "b".to_string(),
            ),
        ];

        let output = format_edges_compatible(&edges);
        let lines: Vec<&str> = output.lines().collect();

        // Should be sorted - a.py comes before z.py
        assert!(
            lines[0].starts_with("a.py"),
            "Output should be sorted alphabetically"
        );
    }

    /// Test: Empty edges produce empty output.
    #[test]
    fn test_format_edges_empty() {
        let edges: Vec<(String, String, String, String)> = vec![];
        let output = format_edges_compatible(&edges);
        assert!(output.is_empty() || output.trim().is_empty());
    }
}

// =============================================================================
// Phase 16.6: A/B Testing Tests
// =============================================================================

mod ab_testing {
    use super::*;

    /// Test: ComparisonResult structure is complete.
    #[test]
    fn test_comparison_result_structure() {
        let result = ComparisonResult {
            only_in_old: HashSet::new(),
            only_in_new: HashSet::new(),
            in_both: HashSet::new(),
        };

        assert!(result.only_in_old.is_empty());
        assert!(result.only_in_new.is_empty());
        assert!(result.in_both.is_empty());
    }

    /// Test: ComparisonResult can report differences.
    #[test]
    fn test_comparison_result_with_differences() {
        let mut only_old = HashSet::new();
        only_old.insert(NormalizedEdge::new(
            "old.py".to_string(),
            "old_func".to_string(),
            "target.py".to_string(),
            "target".to_string(),
        ));

        let mut only_new = HashSet::new();
        only_new.insert(NormalizedEdge::new(
            "new.py".to_string(),
            "new_func".to_string(),
            "target.py".to_string(),
            "target".to_string(),
        ));

        let result = ComparisonResult {
            only_in_old: only_old,
            only_in_new: only_new,
            in_both: HashSet::new(),
        };

        assert_eq!(result.only_in_old.len(), 1);
        assert_eq!(result.only_in_new.len(), 1);
    }
}

// =============================================================================
// Integration Tests (Slow)
// =============================================================================

mod integration {
    use super::*;

    /// Test: A/B comparison on a real project structure.
    /// Spec: "V2 should find at least as many edges as V1"
    #[test]
    #[ignore] // Slow: runs on real project structure
    fn test_ab_comparison_on_project() {
        // Use the current crate's test fixtures or a known project
        let dir = create_comparison_project();
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };

        let comparison = compare_builders(dir.path(), &config).unwrap();

        // Report differences
        if !comparison.only_in_old.is_empty() {
            eprintln!("Edges only in V1:");
            for edge in &comparison.only_in_old {
                eprintln!("  {:?}", edge);
            }
        }

        if !comparison.only_in_new.is_empty() {
            eprintln!("Edges only in V2:");
            for edge in &comparison.only_in_new {
                eprintln!("  {:?}", edge);
            }
        }

        // V2 should not miss any edges
        assert!(
            comparison.only_in_old.is_empty(),
            "V2 missing {} edges from V1",
            comparison.only_in_old.len()
        );
    }

    /// Test: Full pipeline - build, serialize, compare.
    #[test]
    #[ignore] // Slow: full pipeline test
    fn test_full_pipeline() {
        let dir = create_comparison_project();

        // Build with V2
        let config = BuildConfig {
            language: "python".to_string(),
            ..Default::default()
        };
        let ir = build_project_call_graph_v2(dir.path(), config.clone()).unwrap();

        // Serialize
        let json = ir.to_json().unwrap();

        // Deserialize
        let ir2 = CallGraphIR::from_json(&json).unwrap();

        // Convert to old format
        let _old_graph = callgraph_ir_to_old(&ir2);

        // Compare with direct V1 build
        let comparison = compare_builders(dir.path(), &config).unwrap();

        assert!(
            comparison.only_in_old.is_empty(),
            "Full pipeline should preserve all edges"
        );
    }
}
