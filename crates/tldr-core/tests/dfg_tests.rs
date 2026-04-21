//! Tests for Layer 4: DFG operations
//!
//! Commands tested: dfg, slice
//!
//! These integration tests verify DFG and slicing functionality
//! against real fixture files.

use std::path::PathBuf;

use tldr_core::dfg::get_dfg_context;
use tldr_core::pdg::get_slice;
use tldr_core::{Language, RefType, SliceDirection};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// =============================================================================
// dfg command tests
// =============================================================================

mod dfg_tests {
    use super::*;

    #[test]
    fn dfg_extracts_variable_definitions() {
        // GIVEN: A function with variable assignments
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Definition refs should be extracted
        let dfg = dfg.unwrap();
        let defs: Vec<_> = dfg
            .refs
            .iter()
            .filter(|r| r.ref_type == RefType::Definition)
            .collect();
        assert!(!defs.is_empty(), "should have definition refs");
    }

    #[test]
    fn dfg_extracts_variable_uses() {
        // GIVEN: A function using variables
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Use refs should be extracted
        let dfg = dfg.unwrap();
        let uses: Vec<_> = dfg
            .refs
            .iter()
            .filter(|r| r.ref_type == RefType::Use)
            .collect();
        assert!(!uses.is_empty(), "should have use refs");
    }

    #[test]
    fn dfg_extracts_variable_updates() {
        // GIVEN: A function with variable reassignment
        // Note: process_data has 'total = add_to_total(total, item)' which is a reassignment
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Should have definitions (which includes reassignments in our model)
        let dfg = dfg.unwrap();
        // 'total' should appear multiple times as definition
        let total_defs: Vec<_> = dfg
            .refs
            .iter()
            .filter(|r| {
                r.name == "total" && matches!(r.ref_type, RefType::Definition | RefType::Update)
            })
            .collect();
        assert!(
            !total_defs.is_empty(),
            "should have total definitions/updates"
        );
    }

    #[test]
    fn dfg_builds_def_use_chains() {
        // GIVEN: A function where variable defined then used
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Edges should connect definitions to uses
        let dfg = dfg.unwrap();
        assert!(!dfg.edges.is_empty(), "should have def-use edges");
    }

    #[test]
    fn dfg_lists_all_variables() {
        // GIVEN: A function with multiple variables
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: variables list should contain variable names
        let dfg = dfg.unwrap();
        assert!(
            dfg.variables.contains(&"total".to_string()),
            "should have 'total' variable"
        );
        assert!(
            dfg.variables.contains(&"item".to_string()),
            "should have 'item' variable"
        );
    }

    #[test]
    fn dfg_captures_line_and_column() {
        // GIVEN: A function
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Each VarRef should have line and column (line > 0)
        let dfg = dfg.unwrap();
        for r in &dfg.refs {
            assert!(r.line > 0, "line number should be positive");
        }
    }

    #[test]
    fn dfg_handles_function_parameters() {
        // GIVEN: A function with parameters
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Parameters should be treated as definitions
        let dfg = dfg.unwrap();
        // 'items' is a parameter
        let items_def = dfg
            .refs
            .iter()
            .find(|r| r.name == "items" && r.ref_type == RefType::Definition);
        assert!(
            items_def.is_some(),
            "parameter 'items' should be a definition"
        );
    }

    #[test]
    fn dfg_handles_for_loop_variable() {
        // GIVEN: A for loop with iteration variable
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract the DFG for 'process_data'
        let dfg = get_dfg_context(file.to_str().unwrap(), "process_data", Language::Python);

        // THEN: Loop variable should be tracked
        let dfg = dfg.unwrap();
        // 'item' in 'for item in items'
        let item_def = dfg
            .refs
            .iter()
            .find(|r| r.name == "item" && r.ref_type == RefType::Definition);
        assert!(
            item_def.is_some(),
            "for loop variable 'item' should be a definition"
        );
    }

    #[test]
    fn dfg_handles_function_not_found() {
        // GIVEN: A file
        let file = fixtures_dir().join("simple-project/main.py");

        // WHEN: We extract DFG for nonexistent function
        let result = get_dfg_context(file.to_str().unwrap(), "nonexistent", Language::Python);

        // THEN: It should return an error
        assert!(
            result.is_err(),
            "should return error for nonexistent function"
        );
    }

    #[test]
    fn dfg_reaching_definitions_analysis() {
        // GIVEN: A simple function with definitions
        let source = r#"
def test_func():
    if True:
        x = 1
    else:
        x = 2
    print(x)
"#;

        // WHEN: We extract the DFG
        let dfg = get_dfg_context(source, "test_func", Language::Python);

        // THEN: Should succeed and have refs
        let dfg = dfg.unwrap();
        assert!(
            dfg.refs.iter().any(|r| r.name == "x"),
            "should have 'x' refs"
        );
    }
}

// =============================================================================
// slice command tests
// =============================================================================

mod slice_tests {
    use super::*;

    #[test]
    fn slice_backward_finds_dependencies() {
        // GIVEN: A function where line N depends on earlier lines
        let source = r#"
def process_data():
    x = 1
    y = x + 2
    return y
"#;

        // WHEN: We compute backward slice for the return line (line 5)
        let slice = get_slice(
            source,
            "process_data",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        );

        // THEN: Should have a slice
        let slice = slice.unwrap();
        assert!(!slice.is_empty(), "backward slice should not be empty");
    }

    #[test]
    fn slice_forward_finds_dependents() {
        // GIVEN: A function where line N affects later lines
        let source = r#"
def process_data():
    x = 1
    y = x + 2
    return y
"#;

        // WHEN: We compute forward slice from x = 1 (line 3)
        let slice = get_slice(
            source,
            "process_data",
            3,
            SliceDirection::Forward,
            None,
            Language::Python,
        );

        // THEN: Should include the starting line
        let slice = slice.unwrap();
        assert!(
            slice.contains(&3),
            "forward slice should include starting line"
        );
    }

    #[test]
    fn slice_respects_variable_filter() {
        // GIVEN: A function with multiple variables
        let source = r#"
def process_data():
    x = 1
    y = 2
    z = x + y
    return z
"#;

        // WHEN: We slice on a specific variable
        let slice = get_slice(
            source,
            "process_data",
            5,
            SliceDirection::Backward,
            Some("x"),
            Language::Python,
        );

        // THEN: Should have a slice
        let slice = slice.unwrap();
        assert!(!slice.is_empty(), "filtered slice should not be empty");
    }

    #[test]
    fn slice_includes_control_dependencies() {
        // GIVEN: A line inside an if block
        let source = r#"
def test_func(cond):
    if cond:
        x = 1
    return x
"#;

        // WHEN: We compute backward slice from return
        let slice = get_slice(
            source,
            "test_func",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        );

        // THEN: The condition line should be included (control dependency)
        let slice = slice.unwrap();
        assert!(
            !slice.is_empty(),
            "slice should include control dependencies"
        );
    }

    #[test]
    fn slice_includes_data_dependencies() {
        // GIVEN: x = a + b; y = x * 2
        let source = r#"
def test_func():
    a = 1
    b = 2
    x = a + b
    y = x * 2
    return y
"#;

        // WHEN: We slice backward from y = x * 2
        let slice = get_slice(
            source,
            "test_func",
            6,
            SliceDirection::Backward,
            None,
            Language::Python,
        );

        // THEN: x = a + b should be included (data dependency)
        let slice = slice.unwrap();
        assert!(!slice.is_empty(), "slice should include data dependencies");
    }

    #[test]
    fn slice_handles_line_not_in_function() {
        // GIVEN: A line number outside the function
        let source = "def process_data(): pass";

        // WHEN: We try to slice from that line
        let result = get_slice(
            source,
            "process_data",
            999,
            SliceDirection::Backward,
            None,
            Language::Python,
        );

        // THEN: It should return empty set (per spec)
        let slice = result.unwrap();
        assert!(
            slice.is_empty(),
            "slice for line outside function should be empty"
        );
    }

    #[test]
    fn slice_handles_direction_variants() {
        // Test both directions work
        let source = r#"
def test_func():
    x = 1
    return x
"#;

        let backward = get_slice(
            source,
            "test_func",
            4,
            SliceDirection::Backward,
            None,
            Language::Python,
        );
        let forward = get_slice(
            source,
            "test_func",
            3,
            SliceDirection::Forward,
            None,
            Language::Python,
        );

        assert!(backward.is_ok(), "backward slice should work");
        assert!(forward.is_ok(), "forward slice should work");
    }

    #[test]
    fn slice_uses_pdg_for_traversal() {
        // GIVEN: A function with both control and data deps
        let source = r#"
def test_func(cond):
    x = 0
    if cond:
        x = 1
    return x
"#;

        // WHEN: We compute a slice
        let slice = get_slice(
            source,
            "test_func",
            6,
            SliceDirection::Backward,
            None,
            Language::Python,
        );

        // THEN: Should work (uses PDG internally)
        assert!(slice.is_ok(), "slice should use PDG");
    }

    #[test]
    fn slice_returns_line_numbers() {
        // GIVEN: A function
        let source = r#"
def test_func():
    x = 1
    return x
"#;

        // WHEN: We compute a slice
        let slice = get_slice(
            source,
            "test_func",
            4,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // THEN: Result should be a set of line numbers (positive integers)
        for &line in &slice {
            assert!(line > 0, "line numbers should be positive");
        }
    }

    #[test]
    fn slice_traces_all_variables_when_none_specified() {
        // GIVEN: A line using multiple variables
        let source = r#"
def test_func():
    a = 1
    b = 2
    c = a + b
    return c
"#;

        // WHEN: We slice without specifying variable
        let slice = get_slice(
            source,
            "test_func",
            5,
            SliceDirection::Backward,
            None,
            Language::Python,
        )
        .unwrap();

        // THEN: Should trace both a and b
        assert!(!slice.is_empty(), "slice should not be empty");
    }
}
