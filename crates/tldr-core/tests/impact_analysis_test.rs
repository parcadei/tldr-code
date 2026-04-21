//! Test for impact_analysis API
//!
//! This API is used by the `impact` and `change-impact` CLI commands
//!
//! impact_analysis finds all callers of a function via reverse call graph traversal.

use std::path::PathBuf;
use tldr_core::analysis::impact_analysis;
use tldr_core::error::TldrError;
use tldr_core::types::{CallEdge, ProjectCallGraph};

/// Helper function to create a test call graph with a simple chain:
/// main() -> process() -> helper() -> utils()
///          process() -> validate()
fn create_simple_call_graph() -> ProjectCallGraph {
    let mut graph = ProjectCallGraph::new();

    // main() calls process()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("main.py"),
        src_func: "main".to_string(),
        dst_file: PathBuf::from("app.py"),
        dst_func: "process".to_string(),
    });

    // process() calls helper()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("app.py"),
        src_func: "process".to_string(),
        dst_file: PathBuf::from("helpers.py"),
        dst_func: "helper".to_string(),
    });

    // helper() calls utils()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("helpers.py"),
        src_func: "helper".to_string(),
        dst_file: PathBuf::from("utils.py"),
        dst_func: "utils".to_string(),
    });

    // process() also calls validate()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("app.py"),
        src_func: "process".to_string(),
        dst_file: PathBuf::from("validators.py"),
        dst_func: "validate".to_string(),
    });

    graph
}

/// Helper function to create a test call graph with multiple callers
/// A() -> C()
/// B() -> C()
fn create_multi_caller_graph() -> ProjectCallGraph {
    let mut graph = ProjectCallGraph::new();

    // A() calls shared()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("a.py"),
        src_func: "func_a".to_string(),
        dst_file: PathBuf::from("shared.py"),
        dst_func: "shared".to_string(),
    });

    // B() also calls shared()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("b.py"),
        src_func: "func_b".to_string(),
        dst_file: PathBuf::from("shared.py"),
        dst_func: "shared".to_string(),
    });

    // shared() calls util()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("shared.py"),
        src_func: "shared".to_string(),
        dst_file: PathBuf::from("util.py"),
        dst_func: "util".to_string(),
    });

    graph
}

#[test]
fn test_impact_analysis_happy_path_simple_chain() {
    // Arrange: Create a simple call chain
    let graph = create_simple_call_graph();

    // Act: Analyze impact of changing utils()
    let result = impact_analysis(&graph, "utils", 3, None);

    // Assert: Should succeed
    assert!(
        result.is_ok(),
        "impact_analysis should succeed for existing function"
    );

    let report = result.unwrap();
    assert_eq!(
        report.total_targets, 1,
        "Should find exactly one target function"
    );

    let tree = report.targets.values().next().unwrap();
    assert_eq!(tree.function, "utils", "Target function name should match");
    assert_eq!(
        tree.caller_count, 1,
        "utils() should have 1 direct caller (helper)"
    );
    assert_eq!(tree.callers.len(), 1, "Should have 1 caller in tree");

    // Check the caller chain
    let helper = &tree.callers[0];
    assert_eq!(helper.function, "helper", "First caller should be helper");
    assert_eq!(
        helper.caller_count, 1,
        "helper() should have 1 caller (process)"
    );

    let process = &helper.callers[0];
    assert_eq!(
        process.function, "process",
        "Second caller should be process"
    );

    println!("PASS: Impact analysis correctly traces call chain: utils <- helper <- process");
}

#[test]
fn test_impact_analysis_multiple_callers() {
    // Arrange: Create a graph with multiple callers
    let graph = create_multi_caller_graph();

    // Act: Analyze impact of changing shared()
    let result = impact_analysis(&graph, "shared", 2, None);

    // Assert: Should succeed and find both callers
    assert!(result.is_ok(), "impact_analysis should succeed");

    let report = result.unwrap();
    let tree = report.targets.values().next().unwrap();

    assert_eq!(
        tree.caller_count, 2,
        "shared() should have 2 direct callers"
    );
    assert_eq!(tree.callers.len(), 2, "Should have 2 callers in tree");

    // Verify both callers are found
    let caller_names: Vec<&str> = tree.callers.iter().map(|c| c.function.as_str()).collect();
    assert!(
        caller_names.contains(&"func_a"),
        "Should find func_a as caller"
    );
    assert!(
        caller_names.contains(&"func_b"),
        "Should find func_b as caller"
    );

    println!(
        "PASS: Impact analysis correctly finds multiple callers: {:?}",
        caller_names
    );
}

#[test]
fn test_impact_analysis_respects_depth_limit() {
    // Arrange: Create a deep call chain
    let graph = create_simple_call_graph();

    // Act: Analyze with depth limit of 1
    let result = impact_analysis(&graph, "utils", 1, None);

    // Assert: Should only show direct callers, truncate the rest
    assert!(result.is_ok());
    let report = result.unwrap();
    let tree = report.targets.values().next().unwrap();

    // At depth 1, we should see helper() but it should be truncated
    assert_eq!(tree.callers.len(), 1, "Should have 1 caller at depth 1");

    let helper = &tree.callers[0];
    assert!(
        helper.truncated,
        "helper() should be marked as truncated at depth limit"
    );
    assert!(helper.note.is_some(), "Truncated node should have a note");

    println!("PASS: Depth limit correctly truncates the call tree");
}

#[test]
fn test_impact_analysis_entry_point_no_callers() {
    // Arrange: Create a graph
    let graph = create_simple_call_graph();

    // Act: Analyze impact of main() which is an entry point (no callers)
    let result = impact_analysis(&graph, "main", 3, None);

    // Assert: Should succeed with 0 callers
    assert!(
        result.is_ok(),
        "impact_analysis should succeed for entry point"
    );

    let report = result.unwrap();
    let tree = report.targets.values().next().unwrap();

    assert_eq!(tree.caller_count, 0, "Entry point should have 0 callers");
    assert!(
        tree.callers.is_empty(),
        "Entry point should have empty callers list"
    );
    assert!(
        tree.note.is_some(),
        "Entry point should have a note explaining no callers"
    );

    println!("PASS: Entry point correctly identified with 0 callers");
}

#[test]
fn test_impact_analysis_nonexistent_function() {
    // Arrange: Create a graph
    let graph = create_simple_call_graph();

    // Act: Analyze impact of a non-existent function
    let result = impact_analysis(&graph, "nonexistent_function", 3, None);

    // Assert: Should return FunctionNotFound error
    assert!(result.is_err(), "Should error for non-existent function");

    match result {
        Err(TldrError::FunctionNotFound {
            name,
            file,
            suggestions,
        }) => {
            assert_eq!(
                name, "nonexistent_function",
                "Error should contain function name"
            );
            assert!(
                file.is_none(),
                "File should be None when no filter was provided"
            );
            // May have suggestions for similar names
            println!("PASS: Correctly returns FunctionNotFound error for 'nonexistent_function'");
            println!("   Suggestions: {:?}", suggestions);
        }
        Err(other) => {
            panic!("Expected FunctionNotFound error, got: {:?}", other);
        }
        Ok(_) => {
            panic!("Expected error for non-existent function");
        }
    }
}

#[test]
fn test_impact_analysis_empty_graph() {
    // Arrange: Create an empty graph
    let graph = ProjectCallGraph::new();

    // Act: Analyze impact on empty graph
    let result = impact_analysis(&graph, "any_function", 3, None);

    // Assert: Should return FunctionNotFound error
    assert!(
        result.is_err(),
        "Should error when function not in empty graph"
    );

    match result {
        Err(TldrError::FunctionNotFound { name, .. }) => {
            assert_eq!(name, "any_function");
            println!("PASS: Correctly returns error for empty graph");
        }
        _ => panic!("Expected FunctionNotFound error for empty graph"),
    }
}

#[test]
fn test_impact_analysis_with_file_filter() {
    // Arrange: Create a graph with functions of same name in different files
    let mut graph = ProjectCallGraph::new();

    // process() in app.py calls helper()
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("app.py"),
        src_func: "process".to_string(),
        dst_file: PathBuf::from("helpers.py"),
        dst_func: "helper".to_string(),
    });

    // process() in other.py also calls something
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("other.py"),
        src_func: "process".to_string(),
        dst_file: PathBuf::from("util.py"),
        dst_func: "util".to_string(),
    });

    // Act: Filter by specific file
    let result = impact_analysis(&graph, "process", 3, Some(&PathBuf::from("app.py")));

    // Assert: Should find only the process() in app.py
    assert!(result.is_ok(), "Should succeed with file filter");

    let report = result.unwrap();
    assert_eq!(
        report.total_targets, 1,
        "Should find exactly one target with file filter"
    );

    let tree = report.targets.values().next().unwrap();
    assert!(
        tree.file.to_string_lossy().contains("app.py"),
        "Should match app.py"
    );

    println!("PASS: File filter correctly narrows down targets");
}

#[test]
fn test_impact_analysis_cyclic_call_detection() {
    // Arrange: Create a graph with a cycle: A -> B -> C -> A
    let mut graph = ProjectCallGraph::new();

    graph.add_edge(CallEdge {
        src_file: PathBuf::from("a.py"),
        src_func: "func_a".to_string(),
        dst_file: PathBuf::from("b.py"),
        dst_func: "func_b".to_string(),
    });

    graph.add_edge(CallEdge {
        src_file: PathBuf::from("b.py"),
        src_func: "func_b".to_string(),
        dst_file: PathBuf::from("c.py"),
        dst_func: "func_c".to_string(),
    });

    // Cycle back to A
    graph.add_edge(CallEdge {
        src_file: PathBuf::from("c.py"),
        src_func: "func_c".to_string(),
        dst_file: PathBuf::from("a.py"),
        dst_func: "func_a".to_string(),
    });

    // Act: Analyze impact starting from func_c
    let result = impact_analysis(&graph, "func_c", 5, None);

    // Assert: Should detect and handle cycle
    assert!(result.is_ok(), "Should handle cyclic calls");

    let report = result.unwrap();
    let tree = report.targets.values().next().unwrap();

    // Should find func_b as caller
    assert!(!tree.callers.is_empty(), "Should have callers");

    println!("PASS: Cycle detection works correctly in impact analysis");
}
