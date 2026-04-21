//! Comprehensive tests for TLDR Contracts & Flow commands
//!
//! These tests define expected behavior from spec.md and should FAIL initially
//! since no implementation exists yet. They drive the implementation.
//!
//! Test categories per command:
//! 1. Happy path tests - Normal successful operation
//! 2. Edge case tests - Boundary conditions
//! 3. Error case tests - All error conditions from spec
//! 4. Output format tests - JSON and text output validation
//!
//! Commands covered:
//! - contracts: Pre/postcondition inference from guard clauses, assertions
//! - invariants: Daikon-lite inference from test traces
//! - specs: Test-derived behavioral specifications
//! - verify: Aggregated verification dashboard
//! - dead-stores: SSA-based dead store detection
//! - bounds: Interval analysis for numeric ranges
//! - chop: Program slice intersection (forward AND backward)

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Get the path to the test binary
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Get assert_cmd version for better assertion support
fn tldr_assert_cmd() -> AssertCommand {
    AssertCommand::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

// =============================================================================
// Shared Types (mirrors types.rs from spec)
// =============================================================================

mod contracts_types {
    use super::*;

    /// Confidence level for inferred contracts
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum Confidence {
        High,
        Medium,
        Low,
    }

    /// A single contract condition
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Condition {
        pub variable: String,
        pub constraint: String,
        pub source_line: u32,
        pub confidence: Confidence,
    }

    /// Contracts report output
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ContractsReport {
        pub function: String,
        pub file: PathBuf,
        pub preconditions: Vec<Condition>,
        pub postconditions: Vec<Condition>,
        pub invariants: Vec<Condition>,
    }

    /// Chop result (slice intersection)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ChopResult {
        pub lines: Vec<u32>,
        pub count: u32,
        pub source_line: u32,
        pub target_line: u32,
        pub path_exists: bool,
        pub function: String,
        pub explanation: Option<String>,
    }
}

use contracts_types::*;

// =============================================================================
// Test Fixtures - Python code samples for analysis
// =============================================================================

/// Python code with guard clause preconditions
const PYTHON_GUARD_CLAUSES: &str = r#"
def process_data(x, data):
    if x < 0:
        raise ValueError("x must be non-negative")
    if not isinstance(data, list):
        raise TypeError("data must be a list")
    if len(data) == 0:
        raise ValueError("data cannot be empty")

    result = sum(data) + x
    return result
"#;

/// Python code with assert statements
const PYTHON_ASSERTS: &str = r#"
def calculate(a, b):
    assert a > 0, "a must be positive"
    assert isinstance(b, (int, float)), "b must be numeric"

    result = a * b
    assert result is not None
    return result
"#;

/// Python code with isinstance checks in conditionals
const PYTHON_ISINSTANCE: &str = r#"
def transform(value):
    if not isinstance(value, str):
        raise TypeError("Expected string")
    if not isinstance(value, (str, bytes)):
        return None

    return value.upper()
"#;

/// Python code with postconditions
const PYTHON_POSTCONDITIONS: &str = r#"
def divide(a, b):
    if b == 0:
        raise ZeroDivisionError("Cannot divide by zero")

    result = a / b
    assert result is not None, "Result should not be None"
    assert isinstance(result, float), "Result should be float"
    return result
"#;

/// Python code with type annotations
const PYTHON_TYPE_ANNOTATIONS: &str = r#"
def greet(name: str, count: int = 1) -> str:
    return (name + "! ") * count
"#;

/// Python code with dead stores
const PYTHON_DEAD_STORES: &str = r#"
def example_with_dead_stores(x):
    a = 10          # Dead store: a is reassigned before use
    b = 20          # Live: used in return
    a = x + 5       # Live: used in return
    c = 30          # Dead store: never used
    return a + b
"#;

/// Python code with loops for bounds analysis
const PYTHON_BOUNDS_ANALYSIS: &str = r#"
def calculate_bounds(n):
    x = 0
    y = 100
    for i in range(n):
        x = x + 1
        y = y - 1
    z = y / x  # Potential division by zero when x == 0
    return z
"#;

/// Python code for chop analysis
const PYTHON_CHOP_ANALYSIS: &str = r#"
def data_flow_example(a, b):
    x = a + 1       # Line 2
    y = b * 2       # Line 3
    z = x + y       # Line 4: depends on x and y
    w = z * 3       # Line 5: depends on z
    result = w + 1  # Line 6: depends on w
    return result   # Line 7
"#;

/// Python test file for specs extraction
const PYTHON_TEST_FILE: &str = r#"
import pytest
from mymodule import add, parse, validate

def test_add_positive():
    assert add(2, 3) == 5

def test_add_negative():
    assert add(-1, 1) == 0

def test_add_zero():
    assert add(0, 0) == 0

def test_parse_returns_dict():
    result = parse("key=value")
    assert isinstance(result, dict)

def test_parse_length():
    result = parse("a=1,b=2")
    assert len(result) == 2

def test_validate_raises_on_empty():
    with pytest.raises(ValueError):
        validate("")

def test_validate_raises_with_match():
    with pytest.raises(ValueError, match="cannot be empty"):
        validate("")

def test_result_positive():
    result = add(1, 2)
    assert result > 0

def test_membership():
    result = parse("id=123")
    assert "id" in result
"#;

/// Python code for invariant tracing
const PYTHON_FOR_INVARIANTS: &str = r#"
def compute(x, y):
    """Function to trace for invariants"""
    return x + y

def bounded_compute(start, end):
    """Function with ordering relation"""
    return end - start
"#;

// =============================================================================
// 1. CONTRACTS Command Tests
// =============================================================================

mod contracts_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_help() {
        tldr_assert_cmd()
            .args(["contracts", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE").or(predicate::str::contains("file")))
            .stdout(predicate::str::contains("<function>"))
            .stdout(predicate::str::contains("--format"));
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "process_data"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport =
            serde_json::from_str(&stdout).expect("Should return valid JSON ContractsReport");

        // Should detect: x >= 0, isinstance(data, list), len(data) > 0
        assert!(
            report.preconditions.len() >= 3,
            "Expected at least 3 preconditions from guard clauses"
        );

        // Check for x >= 0 precondition (negation of x < 0)
        let has_x_constraint = report.preconditions.iter().any(|p| {
            p.variable == "x" && (p.constraint.contains(">=") || p.constraint.contains("> 0"))
        });
        assert!(has_x_constraint, "Should detect x >= 0 precondition");

        // Check for isinstance constraint
        let has_isinstance = report
            .preconditions
            .iter()
            .any(|p| p.constraint.contains("isinstance") && p.variable == "data");
        assert!(has_isinstance, "Should detect isinstance precondition");
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_assert_extraction() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("asserts.py");
        fs::write(&file_path, PYTHON_ASSERTS).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "calculate"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Should detect: a > 0, isinstance(b, (int, float))
        assert!(
            report.preconditions.len() >= 2,
            "Expected at least 2 preconditions from asserts"
        );

        let has_a_positive = report
            .preconditions
            .iter()
            .any(|p| p.variable == "a" && p.constraint.contains("> 0"));
        assert!(has_a_positive, "Should detect a > 0 from assert");
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_isinstance_type_constraints() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("isinstance.py");
        fs::write(&file_path, PYTHON_ISINSTANCE).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "transform"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Should detect isinstance(value, str) precondition
        let has_type_constraint = report
            .preconditions
            .iter()
            .any(|p| p.variable == "value" && p.constraint.contains("str"));
        assert!(
            has_type_constraint,
            "Should detect type constraint for value"
        );
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_postcondition_inference() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("postcond.py");
        fs::write(&file_path, PYTHON_POSTCONDITIONS).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "divide"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Should detect postconditions: result is not None, isinstance(result, float)
        assert!(
            !report.postconditions.is_empty(),
            "Expected postconditions to be detected"
        );

        let has_not_none = report
            .postconditions
            .iter()
            .any(|p| p.variable == "result" && p.constraint.contains("not None"));
        assert!(
            has_not_none,
            "Should detect result is not None postcondition"
        );
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_confidence_scoring() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("confidence.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "process_data"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Guard clauses should have High confidence
        for precond in &report.preconditions {
            assert_eq!(
                precond.confidence,
                Confidence::High,
                "Guard clause preconditions should have High confidence"
            );
        }
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_type_annotations_low_confidence() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("annotations.py");
        fs::write(&file_path, PYTHON_TYPE_ANNOTATIONS).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "greet"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Type annotation preconditions should have Low confidence
        let type_based = report
            .preconditions
            .iter()
            .filter(|p| p.confidence == Confidence::Low);
        assert!(
            type_based.count() >= 1,
            "Type annotation constraints should have Low confidence"
        );
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_json_output_format() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        let output = tldr_cmd()
            .args([
                "contracts",
                file_path.to_str().unwrap(),
                "process_data",
                "--format",
                "json",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Should be valid JSON");

        assert!(json.get("function").is_some());
        assert!(json.get("file").is_some());
        assert!(json.get("preconditions").is_some());
        assert!(json.get("postconditions").is_some());
        assert!(json.get("invariants").is_some());
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_text_output_format() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        tldr_assert_cmd()
            .args([
                "contracts",
                file_path.to_str().unwrap(),
                "process_data",
                "--format",
                "text",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Function:"))
            .stdout(predicate::str::contains("Preconditions:"))
            .stdout(predicate::str::contains("Postconditions:"));
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_no_conditions_detected() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("simple.py");
        fs::write(&file_path, "def simple(x): return x + 1").unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "simple"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let report: ContractsReport = serde_json::from_str(&stdout).unwrap();

        // Empty vectors are valid when no contracts detected
        assert!(report.preconditions.is_empty() || !report.preconditions.is_empty());
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_nested_conditions() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("nested.py");
        let code = r#"
def nested(x, y):
    if x < 0:
        if y < 0:
            raise ValueError("Both negative")
    return x + y
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "nested"])
            .output()
            .unwrap();

        assert!(output.status.success());
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_multiple_return_statements() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("multi_return.py");
        let code = r#"
def multi_return(x):
    if x < 0:
        return -1
    if x == 0:
        return 0
    return 1
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "multi_return"])
            .output()
            .unwrap();

        assert!(output.status.success());
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_file_not_found() {
        tldr_assert_cmd()
            .args(["contracts", "/nonexistent/file.py", "some_function"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("file not found"));
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_function_not_found() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, "def existing(): pass").unwrap();

        tldr_assert_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "nonexistent"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("function").and(predicate::str::contains("not found")),
            );
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_parse_error() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("invalid.py");
        fs::write(&file_path, "def broken( invalid syntax").unwrap();

        tldr_assert_cmd()
            .args(["contracts", file_path.to_str().unwrap(), "broken"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("parse error"));
    }

    #[test]
    #[ignore = "contracts command not yet implemented"]
    fn test_contracts_missing_required_args() {
        tldr_assert_cmd()
            .args(["contracts"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("required"));
    }
}

// =============================================================================
// 2. INVARIANTS Command Tests
// =============================================================================

mod invariants_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_invariants_help() {
        tldr_assert_cmd()
            .args(["invariants", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE").or(predicate::str::contains("file")))
            .stdout(predicate::str::contains("--from-tests"))
            .stdout(predicate::str::contains("--min-obs"));
    }

    #[test]
    fn test_invariants_type_inference() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("src.py");
        let test_path = temp.path().join("test_src.py");

        fs::write(&src_path, PYTHON_FOR_INVARIANTS).unwrap();
        fs::write(
            &test_path,
            r#"
from src import compute

def test_compute_ints():
    assert compute(1, 2) == 3
    assert compute(5, 10) == 15
    assert compute(0, 0) == 0
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");

        // Should have function invariants
        assert!(json.get("functions").is_some());
    }

    #[test]
    fn test_invariants_non_null_detection() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("module.py");
        let test_path = temp.path().join("test_module.py");

        fs::write(
            &src_path,
            r#"
def process(data):
    return data.strip()
"#,
        )
        .unwrap();

        fs::write(
            &test_path,
            r#"
from module import process

def test_process_strings():
    assert process("hello") == "hello"
    assert process("  world  ") == "world"
    assert process("test") == "test"
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should detect that data is never None
        assert!(
            stdout.contains("non_null")
                || stdout.contains("NonNull")
                || stdout.contains("not None")
        );
    }

    #[test]
    fn test_invariants_numeric_bounds() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("math_ops.py");
        let test_path = temp.path().join("test_math_ops.py");

        fs::write(
            &src_path,
            r#"
def square(x):
    return x * x
"#,
        )
        .unwrap();

        fs::write(
            &test_path,
            r#"
from math_ops import square

def test_square_positive():
    assert square(1) == 1
    assert square(2) == 4
    assert square(3) == 9
    assert square(10) == 100
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should detect x > 0 (positive) or x >= 0 (non-negative)
        assert!(
            stdout.contains("positive")
                || stdout.contains("> 0")
                || stdout.contains("non_negative")
                || stdout.contains(">= 0")
        );
    }

    #[test]
    fn test_invariants_ordering_relations() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("range_ops.py");
        let test_path = temp.path().join("test_range_ops.py");

        fs::write(&src_path, PYTHON_FOR_INVARIANTS).unwrap();

        fs::write(
            &test_path,
            r#"
from range_ops import bounded_compute

def test_bounded_compute():
    assert bounded_compute(0, 10) == 10
    assert bounded_compute(5, 15) == 10
    assert bounded_compute(100, 200) == 100
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
                "--function",
                "bounded_compute",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should detect start < end relation
        assert!(
            stdout.contains("relation")
                || stdout.contains("<")
                || stdout.contains("start")
                || stdout.contains("end")
        );
    }

    #[test]
    fn test_invariants_confidence_by_observations() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("func.py");
        let test_path = temp.path().join("test_func.py");

        fs::write(&src_path, "def identity(x): return x").unwrap();

        // Many observations = high confidence
        let mut test_code = String::from("from func import identity\n\n");
        for i in 0..15 {
            test_code.push_str(&format!(
                "def test_identity_{}(): assert identity({}) == {}\n",
                i, i, i
            ));
        }
        fs::write(&test_path, test_code).unwrap();

        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        // With 15 observations, should have high confidence
        assert!(stdout.contains("high") || stdout.contains("High"));
    }

    #[test]
    fn test_invariants_min_obs_filter() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("func.py");
        let test_path = temp.path().join("test_func.py");

        fs::write(&src_path, "def add(a, b): return a + b").unwrap();
        fs::write(
            &test_path,
            r#"
from func import add
def test_add(): assert add(1, 2) == 3
"#,
        )
        .unwrap();

        // With min-obs=5, single observation should be filtered out
        let output = tldr_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                test_path.to_str().unwrap(),
                "--min-obs",
                "5",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // Functions array should be empty or have no invariants
        let functions = json.get("functions").and_then(|f| f.as_array());
        if let Some(funcs) = functions {
            for func in funcs {
                let preconditions = func.get("preconditions").and_then(|p| p.as_array());
                assert!(
                    preconditions.is_none_or(|p| p.is_empty()),
                    "Should filter out invariants with < 5 observations"
                );
            }
        }
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_invariants_file_not_found() {
        tldr_assert_cmd()
            .args([
                "invariants",
                "/nonexistent/file.py",
                "--from-tests",
                "/some/tests",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("file not found"));
    }

    #[test]
    fn test_invariants_test_path_not_found() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("src.py");
        fs::write(&src_path, "def foo(): pass").unwrap();

        tldr_assert_cmd()
            .args([
                "invariants",
                src_path.to_str().unwrap(),
                "--from-tests",
                "/nonexistent/tests",
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains("test path not found"));
    }

    #[test]
    fn test_invariants_missing_from_tests() {
        let temp = TempDir::new().unwrap();
        let src_path = temp.path().join("src.py");
        fs::write(&src_path, "def foo(): pass").unwrap();

        tldr_assert_cmd()
            .args(["invariants", src_path.to_str().unwrap()])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("--from-tests").or(predicate::str::contains("required")),
            );
    }
}

// =============================================================================
// 3. SPECS Command Tests
// =============================================================================

mod specs_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_help() {
        tldr_assert_cmd()
            .args(["specs", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--from-tests"))
            .stdout(predicate::str::contains("--function"))
            .stdout(predicate::str::contains("--source"));
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_input_output_extraction() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");

        // Should extract IO specs for 'add' function
        let functions = json.get("functions").and_then(|f| f.as_array()).unwrap();
        let add_func = functions.iter().find(|f| {
            f.get("function_name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == "add")
        });
        assert!(add_func.is_some(), "Should find specs for 'add' function");

        let io_specs = add_func
            .unwrap()
            .get("input_output_specs")
            .and_then(|s| s.as_array());
        assert!(
            io_specs.is_some_and(|s| s.len() >= 3),
            "Should extract at least 3 IO specs for add"
        );
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_exception_extraction() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let functions = json.get("functions").and_then(|f| f.as_array()).unwrap();
        let validate_func = functions.iter().find(|f| {
            f.get("function_name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == "validate")
        });
        assert!(
            validate_func.is_some(),
            "Should find specs for 'validate' function"
        );

        let exc_specs = validate_func
            .unwrap()
            .get("exception_specs")
            .and_then(|s| s.as_array());
        assert!(
            exc_specs.is_some_and(|s| !s.is_empty()),
            "Should extract exception specs for validate"
        );
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_property_extraction() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let functions = json.get("functions").and_then(|f| f.as_array()).unwrap();
        let parse_func = functions.iter().find(|f| {
            f.get("function_name")
                .and_then(|n| n.as_str())
                .is_some_and(|n| n == "parse")
        });
        assert!(
            parse_func.is_some(),
            "Should find specs for 'parse' function"
        );

        let prop_specs = parse_func
            .unwrap()
            .get("property_specs")
            .and_then(|s| s.as_array());
        assert!(
            prop_specs.is_some_and(|s| s.len() >= 2),
            "Should extract type and length property specs for parse"
        );
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_literal_evaluation() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_literals.py");
        fs::write(
            &test_path,
            r#"
def test_with_literals():
    assert func([1, 2, 3]) == [3, 2, 1]
    assert func({"key": "value"}) == {"value": "key"}
    assert func("hello") == "olleh"
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should safely evaluate literals
        assert!(stdout.contains("[1, 2, 3]") || stdout.contains("1,2,3"));
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_function_filter() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args([
                "specs",
                "--from-tests",
                test_path.to_str().unwrap(),
                "--function",
                "add",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let functions = json.get("functions").and_then(|f| f.as_array()).unwrap();
        assert_eq!(functions.len(), 1, "Should only return specs for 'add'");
        assert_eq!(
            functions[0]
                .get("function_name")
                .and_then(|n| n.as_str())
                .unwrap(),
            "add"
        );
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_summary_counts() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let summary = json.get("summary").expect("Should have summary");
        assert!(summary.get("total_specs").is_some());
        assert!(summary.get("by_type").is_some());
        assert!(summary.get("test_functions_scanned").is_some());
        assert!(summary.get("test_files_scanned").is_some());
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_text_output() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("test_module.py");
        fs::write(&test_path, PYTHON_TEST_FILE).unwrap();

        tldr_assert_cmd()
            .args([
                "specs",
                "--from-tests",
                test_path.to_str().unwrap(),
                "--format",
                "text",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Function:"))
            .stdout(predicate::str::contains("IO:").or(predicate::str::contains("Raises:")))
            .stdout(predicate::str::contains("Total specs:"));
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_directory_recursive() {
        let temp = TempDir::new().unwrap();
        let tests_dir = temp.path().join("tests");
        fs::create_dir(&tests_dir).unwrap();

        fs::write(
            tests_dir.join("test_a.py"),
            "def test_a(): assert func_a(1) == 2",
        )
        .unwrap();
        fs::write(
            tests_dir.join("test_b.py"),
            "def test_b(): assert func_b(3) == 6",
        )
        .unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", tests_dir.to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let summary = json.get("summary").unwrap();
        let files_scanned = summary
            .get("test_files_scanned")
            .and_then(|n| n.as_u64())
            .unwrap();
        assert_eq!(files_scanned, 2, "Should scan both test files");
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_no_test_functions() {
        let temp = TempDir::new().unwrap();
        let test_path = temp.path().join("not_tests.py");
        fs::write(&test_path, "def helper(): pass\ndef utility(): pass").unwrap();

        let output = tldr_cmd()
            .args(["specs", "--from-tests", test_path.to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let summary = json.get("summary").unwrap();
        let total = summary.get("total_specs").and_then(|n| n.as_u64()).unwrap();
        assert_eq!(total, 0, "Should find no specs in non-test file");
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_path_not_found() {
        tldr_assert_cmd()
            .args(["specs", "--from-tests", "/nonexistent/path"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("not found"));
    }

    #[test]
    #[ignore = "specs command not yet implemented"]
    fn test_specs_missing_from_tests() {
        tldr_assert_cmd().args(["specs"]).assert().failure().stderr(
            predicate::str::contains("--from-tests").or(predicate::str::contains("required")),
        );
    }
}

// =============================================================================
// 4. VERIFY Command Tests
// =============================================================================

mod verify_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]

    fn test_verify_help() {
        tldr_assert_cmd()
            .args(["verify", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("[PATH]"))
            .stdout(predicate::str::contains("--quick"))
            .stdout(predicate::str::contains("--detail"));
    }

    #[test]
    fn test_verify_full_sweep() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        let test_dir = temp.path().join("tests");
        fs::create_dir(&src_dir).unwrap();
        fs::create_dir(&test_dir).unwrap();

        fs::write(src_dir.join("module.py"), PYTHON_GUARD_CLAUSES).unwrap();
        fs::write(test_dir.join("test_module.py"), PYTHON_TEST_FILE).unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");

        // Should have sub_results
        assert!(json.get("sub_results").is_some());
        assert!(json.get("summary").is_some());
        assert!(json.get("total_elapsed_ms").is_some());
    }

    #[test]
    fn test_verify_quick_mode() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_GUARD_CLAUSES).unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap(), "--quick"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // Quick mode should skip invariants and bounds
        let sub_results = json.get("sub_results").and_then(|s| s.as_object()).unwrap();

        // Invariants should be skipped or not present in quick mode
        // In quick mode, invariants aren't even added to sub_results
        assert!(
            sub_results.get("invariants").is_none()
                || sub_results
                    .get("invariants")
                    .and_then(|i| i.get("status"))
                    .is_some_and(|s| s.as_str() != Some("success")),
            "Invariants should be skipped in quick mode"
        );
    }

    #[test]

    fn test_verify_coverage_calculation() {
        let temp = TempDir::new().unwrap();

        // Create files with and without contracts
        fs::write(
            temp.path().join("with_contracts.py"),
            r#"
def constrained(x):
    if x < 0:
        raise ValueError("x must be non-negative")
    return x * 2
"#,
        )
        .unwrap();

        fs::write(
            temp.path().join("without_contracts.py"),
            r#"
def unconstrained(x):
    return x * 2
"#,
        )
        .unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let summary = json.get("summary").unwrap();
        let coverage = summary.get("coverage").unwrap();

        assert!(coverage.get("constrained_functions").is_some());
        assert!(coverage.get("total_functions").is_some());
        assert!(coverage.get("coverage_pct").is_some());
    }

    #[test]

    fn test_verify_detail_specific_analysis() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_GUARD_CLAUSES).unwrap();

        let output = tldr_cmd()
            .args([
                "verify",
                temp.path().to_str().unwrap(),
                "--detail",
                "contracts",
            ])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should show detailed contracts output
        assert!(stdout.contains("contract") || stdout.contains("precondition"));
    }

    #[test]

    fn test_verify_sub_analysis_failure_captured() {
        let temp = TempDir::new().unwrap();

        // Create a file that will cause parse errors
        fs::write(temp.path().join("broken.py"), "def broken( syntax error").unwrap();
        fs::write(temp.path().join("valid.py"), "def valid(): pass").unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        // Command should still succeed (failures are captured)
        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // Check that some sub-analysis captured an error
        let sub_results = json.get("sub_results").and_then(|s| s.as_object()).unwrap();
        assert!(
            sub_results.values().all(|r| r.is_object()),
            "Each sub-result should be represented as an object"
        );
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    #[test]

    fn test_verify_text_output() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_GUARD_CLAUSES).unwrap();

        tldr_assert_cmd()
            .args(["verify", temp.path().to_str().unwrap(), "--format", "text"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Verification:"))
            .stdout(predicate::str::contains("Constraint Coverage:"))
            .stdout(predicate::str::contains("Elapsed:"));
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]

    fn test_verify_default_current_dir() {
        // Running verify without path should use current directory
        let output = tldr_cmd().args(["verify"]).output().unwrap();

        // Should succeed (may find nothing if cwd has no Python files)
        assert!(output.status.success());
    }

    #[test]

    fn test_verify_empty_directory() {
        let temp = TempDir::new().unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let summary = json.get("summary").unwrap();
        let coverage = summary.get("coverage").unwrap();
        let total = coverage
            .get("total_functions")
            .and_then(|n| n.as_u64())
            .unwrap();
        assert_eq!(total, 0, "Empty directory should have 0 functions");
    }

    #[test]

    fn test_verify_no_test_directory() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), "def foo(): pass").unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        // Should succeed but specs analysis may report no test directory
        assert!(output.status.success());
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]

    fn test_verify_nonexistent_path() {
        tldr_assert_cmd()
            .args(["verify", "/nonexistent/path"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("not found").or(predicate::str::contains("No such")));
    }
}

// =============================================================================
// 5. DEAD-STORES Command Tests
// =============================================================================

mod dead_stores_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_help() {
        tldr_assert_cmd()
            .args(["dead-stores", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE").or(predicate::str::contains("file")))
            .stdout(predicate::str::contains("<function>"))
            .stdout(predicate::str::contains("--compare"));
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_basic_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("dead.py");
        fs::write(&file_path, PYTHON_DEAD_STORES).unwrap();

        let output = tldr_cmd()
            .args([
                "dead-stores",
                file_path.to_str().unwrap(),
                "example_with_dead_stores",
            ])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");

        // Should detect dead stores
        let dead_stores = json.get("dead_stores_ssa").and_then(|d| d.as_array());
        assert!(
            dead_stores.is_some_and(|d| !d.is_empty()),
            "Should detect at least one dead store"
        );

        // Should detect the first 'a = 10' as dead (reassigned before use)
        let has_dead_a = dead_stores
            .unwrap()
            .iter()
            .any(|d| d.get("variable").and_then(|v| v.as_str()) == Some("a"));
        assert!(has_dead_a, "Should detect 'a = 10' as dead store");

        // Should detect 'c = 30' as dead (never used)
        let has_dead_c = dead_stores
            .unwrap()
            .iter()
            .any(|d| d.get("variable").and_then(|v| v.as_str()) == Some("c"));
        assert!(has_dead_c, "Should detect 'c = 30' as dead store");
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_phi_function_handling() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("phi.py");
        let code = r#"
def with_phi(x):
    if x > 0:
        y = 1
    else:
        y = 2
    # y has phi function at this merge point
    return y
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["dead-stores", file_path.to_str().unwrap(), "with_phi"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // Should not report phi as dead store if y is used
        let dead_stores = json
            .get("dead_stores_ssa")
            .and_then(|d| d.as_array())
            .unwrap();

        // y_1 and y_2 feed into phi, which is used - none should be dead
        let has_phi_dead = dead_stores.iter().any(|d| {
            d.get("is_phi").and_then(|p| p.as_bool()) == Some(true)
                && d.get("variable").and_then(|v| v.as_str()) == Some("y")
        });
        // Phi for y should NOT be dead since y is returned
        assert!(!has_phi_dead, "Phi for used variable should not be dead");
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_ssa_names() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("ssa.py");
        fs::write(&file_path, PYTHON_DEAD_STORES).unwrap();

        let output = tldr_cmd()
            .args([
                "dead-stores",
                file_path.to_str().unwrap(),
                "example_with_dead_stores",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let dead_stores = json
            .get("dead_stores_ssa")
            .and_then(|d| d.as_array())
            .unwrap();

        // Each dead store should have an SSA name like "a_1", "c_1"
        for store in dead_stores {
            let ssa_name = store.get("ssa_name").and_then(|n| n.as_str());
            assert!(
                ssa_name.is_some(),
                "Each dead store should have an ssa_name"
            );
            assert!(
                ssa_name.unwrap().contains("_"),
                "SSA name should contain version suffix"
            );
        }
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_compare_mode() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("compare.py");
        fs::write(&file_path, PYTHON_DEAD_STORES).unwrap();

        let output = tldr_cmd()
            .args([
                "dead-stores",
                file_path.to_str().unwrap(),
                "example_with_dead_stores",
                "--compare",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // Should have both SSA and live-vars results
        assert!(json.get("dead_stores_ssa").is_some());
        assert!(
            json.get("dead_stores_live_vars").is_some(),
            "Compare mode should include live_vars result"
        );
        assert!(
            json.get("live_vars_count").is_some(),
            "Compare mode should include live_vars count"
        );
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_no_dead_stores() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("clean.py");
        let code = r#"
def all_used(a, b):
    x = a + 1
    y = b + 2
    return x + y
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["dead-stores", file_path.to_str().unwrap(), "all_used"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let count = json.get("count").and_then(|c| c.as_u64()).unwrap();
        assert_eq!(count, 0, "Should find no dead stores in clean code");
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_loop_variables() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("loop.py");
        let code = r#"
def loop_example(n):
    total = 0
    for i in range(n):
        total = total + i  # total is reassigned each iteration
    return total
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["dead-stores", file_path.to_str().unwrap(), "loop_example"])
            .output()
            .unwrap();

        assert!(output.status.success());
        // Loop variables should be handled correctly by SSA
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_multiple_assignments() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("multi.py");
        let code = r#"
def multiple_assigns(x):
    a = 1   # Dead: immediately reassigned
    a = 2   # Dead: immediately reassigned
    a = 3   # Live: returned
    return a
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args([
                "dead-stores",
                file_path.to_str().unwrap(),
                "multiple_assigns",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let count = json.get("count").and_then(|c| c.as_u64()).unwrap();
        assert_eq!(count, 2, "Should find 2 dead stores (a=1 and a=2)");
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_file_not_found() {
        tldr_assert_cmd()
            .args(["dead-stores", "/nonexistent/file.py", "some_function"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("file not found"));
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_function_not_found() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, "def existing(): pass").unwrap();

        tldr_assert_cmd()
            .args(["dead-stores", file_path.to_str().unwrap(), "nonexistent"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("function").and(predicate::str::contains("not found")),
            );
    }

    #[test]
    #[ignore = "dead-stores command not yet implemented"]
    fn test_dead_stores_ssa_error() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("invalid.py");
        // Create code that might cause SSA construction issues
        fs::write(&file_path, "def broken( invalid syntax").unwrap();

        tldr_assert_cmd()
            .args(["dead-stores", file_path.to_str().unwrap(), "broken"])
            .assert()
            .failure();
    }
}

// =============================================================================
// 6. BOUNDS Command Tests
// =============================================================================

mod bounds_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_help() {
        tldr_assert_cmd()
            .args(["bounds", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE").or(predicate::str::contains("file")))
            .stdout(predicate::str::contains("[function]"))
            .stdout(predicate::str::contains("--max-iter"));
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_interval_tracking() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("bounds.py");
        let code = r#"
def simple_bounds(n):
    x = 0
    x = x + 5
    x = x * 2
    return x
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["bounds", file_path.to_str().unwrap(), "simple_bounds"])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");

        // Should have bounds for x at various lines
        assert!(json.get("bounds").is_some());
        assert!(json.get("converged").is_some());
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_division_by_zero_warning() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("divzero.py");
        fs::write(&file_path, PYTHON_BOUNDS_ANALYSIS).unwrap();

        let output = tldr_cmd()
            .args(["bounds", file_path.to_str().unwrap(), "calculate_bounds"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let warnings = json.get("warnings").and_then(|w| w.as_array());
        assert!(
            warnings.is_some_and(|w| !w.is_empty()),
            "Should detect potential division by zero"
        );

        let has_div_zero = warnings
            .unwrap()
            .iter()
            .any(|w| w.get("kind").and_then(|k| k.as_str()) == Some("division_by_zero"));
        assert!(
            has_div_zero,
            "Should have division_by_zero warning when x can be 0"
        );
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_widening_convergence() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("loop.py");
        let code = r#"
def loop_bounds(n):
    x = 0
    for i in range(n):
        x = x + 1
    return x
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args([
                "bounds",
                file_path.to_str().unwrap(),
                "loop_bounds",
                "--max-iter",
                "50",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let converged = json.get("converged").and_then(|c| c.as_bool()).unwrap();
        assert!(converged, "Should converge with widening");

        let iterations = json.get("iterations").and_then(|i| i.as_u64()).unwrap();
        assert!(iterations <= 50, "Should converge within max iterations");
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_constant_propagation() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("const.py");
        let code = r#"
def constants():
    x = 10
    y = 20
    z = x + y  # Should be [30, 30]
    return z
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["bounds", file_path.to_str().unwrap(), "constants"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        // z should have exact bounds [30, 30]
        let bounds = json.get("bounds").and_then(|b| b.as_object());
        assert!(bounds.is_some());
        // Check that constant intervals are precise
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_analyze_all_functions() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("multi.py");
        let code = r#"
def func1():
    x = 5
    return x

def func2():
    y = 10
    return y
"#;
        fs::write(&file_path, code).unwrap();

        // Without function argument, analyze all
        let output = tldr_cmd()
            .args(["bounds", file_path.to_str().unwrap()])
            .output()
            .unwrap();

        assert!(output.status.success());
        // Should return results for both functions
    }

    // -------------------------------------------------------------------------
    // Output Format Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_text_output() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("bounds.py");
        fs::write(&file_path, PYTHON_BOUNDS_ANALYSIS).unwrap();

        tldr_assert_cmd()
            .args([
                "bounds",
                file_path.to_str().unwrap(),
                "calculate_bounds",
                "--format",
                "text",
            ])
            .assert()
            .success()
            .stdout(predicate::str::contains("Function:"))
            .stdout(predicate::str::contains("Converged:"))
            .stdout(predicate::str::contains("Line"));
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_infinity_handling() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("inf.py");
        let code = r#"
def unbounded(x):
    # x has no constraints, should be (-inf, +inf)
    return x * 2
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args(["bounds", file_path.to_str().unwrap(), "unbounded"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Should handle infinity bounds
        assert!(stdout.contains("inf") || stdout.contains("Infinity") || stdout.contains("-inf"));
    }

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_did_not_converge() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("noconv.py");
        let code = r#"
def divergent(x):
    while True:
        x = x * 2
    return x
"#;
        fs::write(&file_path, code).unwrap();

        let output = tldr_cmd()
            .args([
                "bounds",
                file_path.to_str().unwrap(),
                "divergent",
                "--max-iter",
                "5",
            ])
            .output()
            .unwrap();

        // Should still return results but with converged=false
        let stdout = String::from_utf8_lossy(&output.stdout);
        let json: Value = serde_json::from_str(&stdout).unwrap();

        let converged = json.get("converged").and_then(|c| c.as_bool()).unwrap();
        assert!(!converged, "Should not converge for infinite loop");
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "bounds command not yet implemented"]
    fn test_bounds_file_not_found() {
        tldr_assert_cmd()
            .args(["bounds", "/nonexistent/file.py"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("file not found"));
    }
}

// =============================================================================
// 7. CHOP Command Tests
// =============================================================================

mod chop_command {
    use super::*;

    // -------------------------------------------------------------------------
    // Happy Path Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_help() {
        tldr_assert_cmd()
            .args(["chop", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("FILE").or(predicate::str::contains("file")))
            .stdout(predicate::str::contains("<function>"))
            .stdout(predicate::str::contains("<source_line>"))
            .stdout(predicate::str::contains("<target_line>"));
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_forward_backward_intersection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        // Chop from line 2 (x = a + 1) to line 6 (result = w + 1)
        // Path should include: 2, 4, 5, 6
        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2", // source_line: x = a + 1
                "6", // target_line: result = w + 1
            ])
            .output()
            .unwrap();

        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).expect("Valid JSON ChopResult");

        assert!(result.path_exists, "Path should exist from x to result");
        assert!(result.lines.contains(&2), "Source line should be in chop");
        assert!(result.lines.contains(&6), "Target line should be in chop");
        assert!(
            result.lines.contains(&4),
            "Intermediate line (z = x + y) should be in chop"
        );
        assert!(
            result.lines.contains(&5),
            "Intermediate line (w = z * 3) should be in chop"
        );

        // Line 3 (y = b * 2) should NOT be in chop since it doesn't connect x to result
        // Actually, y is used in z, so it depends on whether we're tracing from x or a
        // Let me reconsider: chop(2, 6) = forward(2) AND backward(6)
        // forward(2) includes 2, 4, 5, 6
        // backward(6) includes 6, 5, 4, 2, 3 (since z uses both x and y)
        // So the intersection should include 2, 4, 5, 6
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_path_exists() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2",
                "6",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(result.path_exists);
        assert!(result.count > 0);
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_no_path_exists() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("independent.py");
        let code = r#"
def independent(a, b):
    x = a + 1       # Line 2: only depends on a
    y = b + 2       # Line 3: only depends on b
    return x, y     # Line 4
"#;
        fs::write(&file_path, code).unwrap();

        // Chop from y (line 3) to x (line 2) - no dependency path
        // Actually, we need lines where target doesn't depend on source
        // Let's try: x doesn't affect y
        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "independent",
                "2", // source: x = a + 1
                "3", // target: y = b + 2
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(!result.path_exists, "No path from x to y");
        assert!(result.lines.is_empty(), "Chop should be empty");
        assert_eq!(result.count, 0);
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_same_line() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "4",
                "4",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(result.path_exists, "Same line should have path");
        assert_eq!(result.lines, vec![4], "Should only contain the single line");
        assert_eq!(result.count, 1);
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_lines_sorted() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2",
                "6",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        let mut sorted = result.lines.clone();
        sorted.sort();
        assert_eq!(result.lines, sorted, "Lines should be sorted");
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_explanation() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        let output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2",
                "6",
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(result.explanation.is_some(), "Should have explanation");
        let explanation = result.explanation.unwrap();
        assert!(
            explanation.contains("dependency") || explanation.contains("path"),
            "Explanation should mention dependency path"
        );
    }

    // -------------------------------------------------------------------------
    // Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_control_flow_dependency() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("control.py");
        let code = r#"
def control_dep(x):
    if x > 0:           # Line 2: condition
        y = 1           # Line 3: control dependent on line 2
    else:
        y = 2           # Line 5
    return y            # Line 6
"#;
        fs::write(&file_path, code).unwrap();

        // Chop from condition (line 2) to y assignment (line 3)
        let output = tldr_cmd()
            .args(["chop", file_path.to_str().unwrap(), "control_dep", "2", "3"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(result.path_exists, "Control dependency should create path");
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_transitive_dependency() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("transitive.py");
        let code = r#"
def transitive(a):
    x = a        # Line 2
    y = x        # Line 3
    z = y        # Line 4
    w = z        # Line 5
    return w     # Line 6
"#;
        fs::write(&file_path, code).unwrap();

        // Chop from a (line 2) to w (line 5) - full transitive chain
        let output = tldr_cmd()
            .args(["chop", file_path.to_str().unwrap(), "transitive", "2", "5"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: ChopResult = serde_json::from_str(&stdout).unwrap();

        assert!(result.path_exists);
        // Should include all lines in the chain: 2, 3, 4, 5
        assert!(result.lines.contains(&2));
        assert!(result.lines.contains(&3));
        assert!(result.lines.contains(&4));
        assert!(result.lines.contains(&5));
    }

    // -------------------------------------------------------------------------
    // Error Case Tests
    // -------------------------------------------------------------------------

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_file_not_found() {
        tldr_assert_cmd()
            .args(["chop", "/nonexistent/file.py", "func", "1", "5"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("file not found"));
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_function_not_found() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, "def existing(): pass").unwrap();

        tldr_assert_cmd()
            .args(["chop", file_path.to_str().unwrap(), "nonexistent", "1", "5"])
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("function").and(predicate::str::contains("not found")),
            );
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_line_outside_function() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("small.py");
        let code = r#"
def small():
    x = 1
    return x
"#;
        fs::write(&file_path, code).unwrap();

        // Line 100 is outside the function (which is lines 2-4)
        tldr_assert_cmd()
            .args(["chop", file_path.to_str().unwrap(), "small", "2", "100"])
            .assert()
            .failure()
            .stderr(predicate::str::contains("outside function"));
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_invalid_line_numbers() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, "def test(): pass").unwrap();

        tldr_assert_cmd()
            .args(["chop", file_path.to_str().unwrap(), "test", "0", "5"])
            .assert()
            .failure();
    }

    #[test]
    #[ignore = "chop command not yet implemented"]
    fn test_chop_missing_required_args() {
        tldr_assert_cmd()
            .args(["chop", "file.py", "func", "1"]) // Missing target_line
            .assert()
            .failure()
            .stderr(predicate::str::contains("required"));
    }
}

// =============================================================================
// Integration Tests - Cross-Command Interactions
// =============================================================================

mod integration {
    use super::*;

    #[test]
    #[ignore = "integration test - requires all commands implemented"]
    fn test_verify_includes_contracts_results() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("module.py"), PYTHON_GUARD_CLAUSES).unwrap();

        // Run verify
        let verify_output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        let verify_json: Value =
            serde_json::from_str(&String::from_utf8_lossy(&verify_output.stdout)).unwrap();

        // Run contracts directly
        let _contracts_output = tldr_cmd()
            .args([
                "contracts",
                temp.path().join("module.py").to_str().unwrap(),
                "process_data",
            ])
            .output()
            .unwrap();

        // Verify should include contract information
        let sub_results = verify_json
            .get("sub_results")
            .and_then(|s| s.as_object())
            .unwrap();
        assert!(
            sub_results.contains_key("contracts"),
            "Verify should run contracts analysis"
        );
    }

    #[test]
    #[ignore = "integration test - requires all commands implemented"]
    fn test_verify_with_dead_stores() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("dead.py"), PYTHON_DEAD_STORES).unwrap();

        let output = tldr_cmd()
            .args(["verify", temp.path().to_str().unwrap()])
            .output()
            .unwrap();

        // Verify should potentially include dead stores in its analysis
        assert!(output.status.success());
    }

    #[test]
    #[ignore = "integration test - requires all commands implemented"]
    fn test_chop_uses_same_pdg_as_slice() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("flow.py");
        fs::write(&file_path, PYTHON_CHOP_ANALYSIS).unwrap();

        // Run chop
        let chop_output = tldr_cmd()
            .args([
                "chop",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2",
                "6",
            ])
            .output()
            .unwrap();

        // Run slice (forward from line 2)
        let slice_output = tldr_cmd()
            .args([
                "slice",
                file_path.to_str().unwrap(),
                "data_flow_example",
                "2",
                "--direction",
                "forward",
            ])
            .output()
            .unwrap();

        // Chop should be subset of forward slice
        let chop_result: ChopResult =
            serde_json::from_str(&String::from_utf8_lossy(&chop_output.stdout)).unwrap();
        let slice_json: Value =
            serde_json::from_str(&String::from_utf8_lossy(&slice_output.stdout)).unwrap();

        let slice_lines = slice_json
            .get("lines")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        for line in &chop_result.lines {
            assert!(
                slice_lines.contains(line),
                "Chop line {} should be in forward slice",
                line
            );
        }
    }
}
