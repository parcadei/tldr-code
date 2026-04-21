//! E2E benchmark tests for CLI-only commands across multiple languages.
//!
//! These commands live in `crates/tldr-cli/` and are not exposed as core library
//! functions. Each test runs the actual `tldr` binary via `std::process::Command`
//! and parses JSON output for structural validation.
//!
//! Commands covered:
//! - contracts: pre/postcondition inference from guard clauses
//! - specs: behavioral specs from test files
//! - invariants: invariant inference from test traces
//! - verify: verification dashboard
//! - interface: public API signature extraction
//! - diff: AST-aware structural diff
//! - debt: technical debt (SQALE method)
//! - health: code health dashboard
//! - coverage: parse coverage reports

use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

// =============================================================================
// Helpers
// =============================================================================

/// Run the `tldr` binary with the given arguments, returning (stdout, stderr, success).
fn run_tldr(args: &[&str]) -> (String, String, bool) {
    let bin = env!("CARGO_BIN_EXE_tldr");
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to run tldr binary");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

/// Run `tldr` and parse stdout as JSON. Panics with diagnostics on failure.
fn run_tldr_json(args: &[&str]) -> Value {
    let (stdout, stderr, success) = run_tldr(args);
    assert!(
        success,
        "tldr {} failed.\nstderr: {}\nstdout: {}",
        args.join(" "),
        stderr,
        stdout
    );
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Failed to parse JSON from `tldr {}`.\nError: {}\nstdout: {}",
            args.join(" "),
            e,
            stdout
        )
    })
}

/// Create a temp directory and write a file into it. Returns (TempDir, file_path_string).
fn write_fixture(filename: &str, content: &str) -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(filename);
    fs::write(&path, content).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// Create a temp directory with multiple files. Returns (TempDir, Vec<(filename, path_string)>).
fn write_fixtures(files: &[(&str, &str)]) -> (TempDir, Vec<(String, String)>) {
    let dir = TempDir::new().unwrap();
    let mut paths = Vec::new();
    for (name, content) in files {
        let path = dir.path().join(name);
        // Create subdirectories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        paths.push((name.to_string(), path.to_str().unwrap().to_string()));
    }
    (dir, paths)
}

// =============================================================================
// contracts command
// =============================================================================

#[cfg(test)]
mod contracts {
    use super::*;

    // ---- Python ----

    #[test]
    fn test_contracts_python_guard_clause() {
        let (_dir, path) = write_fixture(
            "guard.py",
            r#"def divide(a, b):
    if b == 0:
        raise ValueError("division by zero")
    return a / b
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "divide", "-f", "json", "-q"]);

        assert_eq!(json["function"], "divide");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_b_guard = preconds.iter().any(|p| {
            let constraint = p["constraint"].as_str().unwrap_or("");
            let variable = p["variable"].as_str().unwrap_or("");
            variable == "b" && constraint.contains("!= 0")
        });
        assert!(
            has_b_guard,
            "Expected precondition about b != 0, got: {:?}",
            preconds
        );

        let high_conf = preconds.iter().any(|p| {
            p["variable"].as_str() == Some("b") && p["confidence"].as_str() == Some("high")
        });
        assert!(high_conf, "b != 0 precondition should be high confidence");
    }

    #[test]
    fn test_contracts_python_isinstance_check() {
        let (_dir, path) = write_fixture(
            "typecheck.py",
            r#"def validate_age(age):
    if not isinstance(age, int):
        raise TypeError("age must be int")
    if age < 0:
        raise ValueError("age must be non-negative")
    if age > 150:
        raise ValueError("age too large")
    return age
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "validate_age", "-f", "json", "-q"]);

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        assert!(
            preconds.len() >= 3,
            "Expected at least 3 preconditions (isinstance + 2 range checks), got {}",
            preconds.len()
        );

        let has_isinstance = preconds.iter().any(|p| {
            let constraint = p["constraint"].as_str().unwrap_or("");
            constraint.contains("isinstance") || constraint.contains("int")
        });
        assert!(
            has_isinstance,
            "Expected isinstance precondition, got: {:?}",
            preconds
        );

        let has_range = preconds.iter().any(|p| {
            let constraint = p["constraint"].as_str().unwrap_or("");
            constraint.contains("0") || constraint.contains("non-negative")
        });
        assert!(
            has_range,
            "Expected range precondition for age >= 0, got: {:?}",
            preconds
        );
    }

    #[test]
    fn test_contracts_python_multiple_functions() {
        let (_dir, path) = write_fixture(
            "multi.py",
            r#"def func_a(x):
    if x is None:
        raise ValueError("x required")
    return x

def func_b(y):
    if y < 0:
        raise ValueError("y must be positive")
    return y
"#,
        );

        // Test func_a
        let json_a = run_tldr_json(&["contracts", &path, "func_a", "-f", "json", "-q"]);
        assert_eq!(json_a["function"], "func_a");
        let preconds_a = json_a["preconditions"].as_array().unwrap();
        let has_none_check = preconds_a.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains("None") || c.contains("not None") || c.contains("is not None")
        });
        assert!(
            has_none_check,
            "func_a should have None check precondition, got: {:?}",
            preconds_a
        );

        // Test func_b
        let json_b = run_tldr_json(&["contracts", &path, "func_b", "-f", "json", "-q"]);
        assert_eq!(json_b["function"], "func_b");
        let preconds_b = json_b["preconditions"].as_array().unwrap();
        let has_positive = preconds_b.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains(">= 0") || c.contains("> 0") || c.contains("positive")
        });
        assert!(
            has_positive,
            "func_b should have positive/range precondition, got: {:?}",
            preconds_b
        );
    }

    // ---- Rust ----

    #[test]
    fn test_contracts_rust_guard_clause() {
        let (_dir, path) = write_fixture(
            "guard.rs",
            r#"fn divide(a: f64, b: f64) -> Result<f64, String> {
    if b == 0.0 {
        return Err("division by zero".to_string());
    }
    Ok(a / b)
}
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "divide", "-f", "json", "-q"]);
        assert_eq!(json["function"], "divide");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_b_guard = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            let v = p["variable"].as_str().unwrap_or("");
            v == "b" && c.contains("!= 0")
        });
        assert!(
            has_b_guard,
            "Rust: expected b != 0.0 precondition, got: {:?}",
            preconds
        );

        // Rust should also extract type-based preconditions
        let has_type = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains("f64")
        });
        assert!(
            has_type,
            "Rust: expected type annotation precondition, got: {:?}",
            preconds
        );

        // Check postconditions include return type
        let postconds = json["postconditions"].as_array().expect("postconditions array");
        let has_return_type = postconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains("Result")
        });
        assert!(
            has_return_type,
            "Rust: expected Result return type postcondition, got: {:?}",
            postconds
        );
    }

    // ---- Go ----

    #[test]
    fn test_contracts_go_guard_clause() {
        let (_dir, path) = write_fixture(
            "guard.go",
            r#"package main

import "errors"

func divide(a, b float64) (float64, error) {
    if b == 0 {
        return 0, errors.New("division by zero")
    }
    return a / b, nil
}
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "divide", "-f", "json", "-q"]);
        assert_eq!(json["function"], "divide");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_b_guard = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            let v = p["variable"].as_str().unwrap_or("");
            v == "b" && c.contains("!= 0")
        });
        assert!(
            has_b_guard,
            "Go: expected b != 0 precondition, got: {:?}",
            preconds
        );
    }

    #[test]
    fn test_contracts_go_string_validation() {
        let (_dir, path) = write_fixture(
            "validate.go",
            r#"package main

import "errors"

func validateName(name string) error {
    if name == "" {
        return errors.New("name cannot be empty")
    }
    if len(name) > 100 {
        return errors.New("name too long")
    }
    return nil
}
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "validateName", "-f", "json", "-q"]);
        assert_eq!(json["function"], "validateName");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_empty_check = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains("!= \"\"") || c.contains("not empty") || c.contains("name")
        });
        assert!(
            has_empty_check,
            "Go: expected empty string check precondition, got: {:?}",
            preconds
        );
    }

    // ---- Java ----

    #[test]
    fn test_contracts_java_guard_clause() {
        let (_dir, path) = write_fixture(
            "Guard.java",
            r#"public class Guard {
    public static double divide(double a, double b) {
        if (b == 0) {
            throw new IllegalArgumentException("division by zero");
        }
        return a / b;
    }
}
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "divide", "-f", "json", "-q"]);
        assert_eq!(json["function"], "divide");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_b_guard = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            let v = p["variable"].as_str().unwrap_or("");
            v == "b" && c.contains("!= 0")
        });
        assert!(
            has_b_guard,
            "Java: expected b != 0 precondition, got: {:?}",
            preconds
        );
    }

    #[test]
    fn test_contracts_java_null_check() {
        let (_dir, path) = write_fixture(
            "Validator.java",
            r#"public class Validator {
    public static String validate(String input) {
        if (input == null) {
            throw new NullPointerException("input is null");
        }
        if (input.isEmpty()) {
            throw new IllegalArgumentException("input is empty");
        }
        return input.trim();
    }
}
"#,
        );
        let json = run_tldr_json(&["contracts", &path, "validate", "-f", "json", "-q"]);
        assert_eq!(json["function"], "validate");

        let preconds = json["preconditions"].as_array().expect("preconditions array");
        let has_null_check = preconds.iter().any(|p| {
            let c = p["constraint"].as_str().unwrap_or("");
            c.contains("null") || c.contains("!= null") || c.contains("not null")
        });
        assert!(
            has_null_check,
            "Java: expected null check precondition, got: {:?}",
            preconds
        );
    }

    // ---- Output format ----

    #[test]
    fn test_contracts_text_format() {
        let (_dir, path) = write_fixture(
            "guard.py",
            r#"def divide(a, b):
    if b == 0:
        raise ValueError("division by zero")
    return a / b
"#,
        );
        let (stdout, _stderr, success) =
            run_tldr(&["contracts", &path, "divide", "-f", "text", "-q"]);
        assert!(success, "contracts text format should succeed");
        // Text output should mention the function and preconditions
        assert!(
            stdout.contains("divide") || stdout.contains("precondition") || stdout.contains("b"),
            "Text output should mention function name or preconditions, got: {}",
            stdout
        );
    }

    // ---- Nonexistent function ----

    #[test]
    fn test_contracts_nonexistent_function() {
        let (_dir, path) = write_fixture(
            "simple.py",
            r#"def hello():
    return "world"
"#,
        );
        let (_stdout, stderr, success) = run_tldr(&[
            "contracts",
            &path,
            "nonexistent_function",
            "-f",
            "json",
            "-q",
        ]);
        // The CLI returns an error when the function is not found -- this is correct behavior
        assert!(
            !success,
            "contracts should fail for a nonexistent function"
        );
        assert!(
            stderr.contains("not found"),
            "Error message should mention 'not found', got: {}",
            stderr
        );
    }
}

// =============================================================================
// specs command
// =============================================================================

#[cfg(test)]
mod specs {
    use super::*;

    #[test]
    fn test_specs_python_basic() {
        let (_dir, path) = write_fixture(
            "test_math.py",
            r#"def test_add_positive():
    assert add(2, 3) == 5

def test_add_negative():
    assert add(-1, -1) == -2

def test_add_zero():
    assert add(0, 0) == 0
"#,
        );
        let json = run_tldr_json(&["specs", "--from-tests", &path, "-f", "json", "-q"]);

        // Check top-level structure
        assert!(json["functions"].is_array(), "specs should have functions array");
        assert!(json["summary"].is_object(), "specs should have summary");

        let functions = json["functions"].as_array().unwrap();
        let add_fn = functions.iter().find(|f| f["function_name"] == "add");
        assert!(add_fn.is_some(), "Should find specs for 'add' function");

        let add_fn = add_fn.unwrap();
        let io_specs = add_fn["input_output_specs"]
            .as_array()
            .expect("input_output_specs array");
        assert!(
            io_specs.len() >= 3,
            "Expected at least 3 input/output specs for add, got {}",
            io_specs.len()
        );

        // Verify specific spec
        let has_2_3_5 = io_specs.iter().any(|s| {
            let empty_arr = vec![];
            let inputs = s["inputs"].as_array().unwrap_or(&empty_arr);
            let output = s["output"].as_i64().unwrap_or(-1);
            inputs.len() == 2
                && inputs[0].as_i64() == Some(2)
                && inputs[1].as_i64() == Some(3)
                && output == 5
        });
        assert!(
            has_2_3_5,
            "Expected spec: add(2, 3) == 5, got: {:?}",
            io_specs
        );

        // Verify summary
        let summary = &json["summary"];
        assert!(
            summary["total_specs"].as_i64().unwrap_or(0) >= 3,
            "Expected at least 3 total specs"
        );
        assert!(
            summary["functions_found"].as_i64().unwrap_or(0) >= 1,
            "Expected at least 1 function found"
        );
    }

    #[test]
    fn test_specs_python_multiple_functions() {
        let (_dir, path) = write_fixture(
            "test_calc.py",
            r#"def test_add():
    assert add(2, 3) == 5

def test_multiply():
    assert multiply(3, 4) == 12

def test_subtract():
    assert subtract(10, 4) == 6
"#,
        );
        let json = run_tldr_json(&["specs", "--from-tests", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();
        assert!(
            functions.len() >= 3,
            "Expected specs for 3 functions, got {}",
            functions.len()
        );

        let names: Vec<&str> = functions
            .iter()
            .filter_map(|f| f["function_name"].as_str())
            .collect();
        assert!(names.contains(&"add"), "Should have specs for add");
        assert!(names.contains(&"multiply"), "Should have specs for multiply");
        assert!(
            names.contains(&"subtract"),
            "Should have specs for subtract"
        );
    }

    #[test]
    fn test_specs_python_exception_assertion() {
        let (_dir, path) = write_fixture(
            "test_errors.py",
            r#"import pytest

def test_divide_by_zero():
    with pytest.raises(ValueError):
        divide(1, 0)

def test_divide_normal():
    assert divide(10, 2) == 5
"#,
        );
        let json = run_tldr_json(&["specs", "--from-tests", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();
        let divide_fn = functions.iter().find(|f| f["function_name"] == "divide");
        assert!(
            divide_fn.is_some(),
            "Should find specs for 'divide' function"
        );

        let divide_fn = divide_fn.unwrap();
        // Should have either exception_specs or input_output_specs
        let empty_io = vec![];
        let empty_exc = vec![];
        let io_specs = divide_fn["input_output_specs"].as_array().unwrap_or(&empty_io);
        let exc_specs = divide_fn["exception_specs"].as_array().unwrap_or(&empty_exc);
        assert!(
            !io_specs.is_empty() || !exc_specs.is_empty(),
            "divide should have either IO or exception specs"
        );
    }

    #[test]
    fn test_specs_function_filter() {
        let (_dir, path) = write_fixture(
            "test_multi.py",
            r#"def test_add():
    assert add(1, 2) == 3

def test_multiply():
    assert multiply(3, 4) == 12
"#,
        );
        let json = run_tldr_json(&[
            "specs",
            "--from-tests",
            &path,
            "--function",
            "add",
            "-f",
            "json",
            "-q",
        ]);

        let functions = json["functions"].as_array().unwrap();
        // With --function filter, should only show specs for "add"
        let has_add = functions.iter().any(|f| f["function_name"] == "add");
        assert!(has_add, "Filtered specs should include 'add'");
    }

    #[test]
    fn test_specs_text_format() {
        let (_dir, path) = write_fixture(
            "test_basic.py",
            r#"def test_inc():
    assert inc(1) == 2
"#,
        );
        let (stdout, _stderr, success) =
            run_tldr(&["specs", "--from-tests", &path, "-f", "text", "-q"]);
        assert!(success, "specs text format should succeed");
        assert!(
            !stdout.is_empty(),
            "Text output should not be empty"
        );
    }
}

// =============================================================================
// invariants command
// =============================================================================

#[cfg(test)]
mod invariants {
    use super::*;

    #[test]
    fn test_invariants_python_basic() {
        let (_dir, paths) = write_fixtures(&[
            (
                "math_funcs.py",
                r#"def add(a, b):
    return a + b

def multiply(a, b):
    return a * b
"#,
            ),
            (
                "test_math.py",
                r#"def test_add_positive():
    assert add(2, 3) == 5

def test_add_negative():
    assert add(-1, -1) == -2

def test_add_zero():
    assert add(0, 0) == 0

def test_multiply():
    assert multiply(3, 4) == 12
"#,
            ),
        ]);

        let source_path = &paths[0].1;
        let test_path = &paths[1].1;

        let json = run_tldr_json(&[
            "invariants",
            source_path,
            "--from-tests",
            test_path,
            "-f",
            "json",
            "-q",
        ]);

        // Check top-level structure
        assert!(
            json["functions"].is_array(),
            "invariants should have functions array"
        );
        assert!(
            json["summary"].is_object(),
            "invariants should have summary"
        );

        let functions = json["functions"].as_array().unwrap();
        assert!(
            !functions.is_empty(),
            "Expected at least one function with invariants"
        );

        // Check that at least one function has preconditions or postconditions
        let has_conditions = functions.iter().any(|f| {
            let pre = f["preconditions"].as_array().map(|a| a.len()).unwrap_or(0);
            let post = f["postconditions"].as_array().map(|a| a.len()).unwrap_or(0);
            pre > 0 || post > 0
        });
        assert!(
            has_conditions,
            "Expected at least some inferred invariants"
        );

        // Check summary fields
        let summary = &json["summary"];
        assert!(
            summary["total_observations"].as_i64().unwrap_or(0) > 0,
            "Expected some observations"
        );
        assert!(
            summary["total_invariants"].as_i64().unwrap_or(0) > 0,
            "Expected some invariants"
        );
    }

    #[test]
    fn test_invariants_function_filter() {
        let (_dir, paths) = write_fixtures(&[
            (
                "funcs.py",
                r#"def add(a, b):
    return a + b

def multiply(a, b):
    return a * b
"#,
            ),
            (
                "test_funcs.py",
                r#"def test_add():
    assert add(1, 2) == 3

def test_multiply():
    assert multiply(2, 3) == 6
"#,
            ),
        ]);

        let json = run_tldr_json(&[
            "invariants",
            &paths[0].1,
            "--from-tests",
            &paths[1].1,
            "--function",
            "add",
            "-f",
            "json",
            "-q",
        ]);

        let functions = json["functions"].as_array().unwrap();
        let has_add = functions.iter().any(|f| f["function_name"] == "add");
        assert!(has_add, "Filtered invariants should include 'add'");
    }

    #[test]
    fn test_invariants_type_inference() {
        let (_dir, paths) = write_fixtures(&[
            (
                "typed.py",
                r#"def greet(name):
    return "hello " + name
"#,
            ),
            (
                "test_typed.py",
                r#"def test_greet():
    assert greet("world") == "hello world"

def test_greet_name():
    assert greet("Alice") == "hello Alice"
"#,
            ),
        ]);

        let json = run_tldr_json(&[
            "invariants",
            &paths[0].1,
            "--from-tests",
            &paths[1].1,
            "-f",
            "json",
            "-q",
        ]);

        let functions = json["functions"].as_array().unwrap();
        if let Some(greet_fn) = functions.iter().find(|f| f["function_name"] == "greet") {
            let empty_pre = vec![];
            let preconds = greet_fn["preconditions"].as_array().unwrap_or(&empty_pre);
            // Type invariants should mention str/string
            let has_type = preconds.iter().any(|p| {
                let kind = p["kind"].as_str().unwrap_or("");
                kind == "type"
            });
            assert!(
                has_type,
                "Expected type invariants for string parameter, got: {:?}",
                preconds
            );
        }
    }

    #[test]
    fn test_invariants_by_kind_summary() {
        let (_dir, paths) = write_fixtures(&[
            (
                "math.py",
                r#"def add(a, b):
    return a + b
"#,
            ),
            (
                "test_math.py",
                r#"def test_add():
    assert add(1, 2) == 3

def test_add_neg():
    assert add(-1, 1) == 0
"#,
            ),
        ]);

        let json = run_tldr_json(&[
            "invariants",
            &paths[0].1,
            "--from-tests",
            &paths[1].1,
            "-f",
            "json",
            "-q",
        ]);

        let by_kind = &json["summary"]["by_kind"];
        assert!(
            by_kind.is_object(),
            "summary should have by_kind breakdown"
        );
        // by_kind should have at least some non-zero values
        let total: i64 = by_kind
            .as_object()
            .map(|m| m.values().filter_map(|v| v.as_i64()).sum())
            .unwrap_or(0);
        assert!(
            total > 0,
            "by_kind total should be positive, got: {:?}",
            by_kind
        );
    }
}

// =============================================================================
// verify command
// =============================================================================

#[cfg(test)]
mod verify {
    use super::*;

    #[test]
    fn test_verify_python_project() {
        let (_dir, _paths) = write_fixtures(&[
            (
                "guard.py",
                r#"def divide(a, b):
    if b == 0:
        raise ValueError("division by zero")
    return a / b
"#,
            ),
            (
                "test_math.py",
                r#"def test_add():
    assert add(2, 3) == 5
"#,
            ),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["verify", dir_path, "-f", "json", "-q", "--quick"]);

        // Check top-level structure
        assert!(
            json["path"].is_string(),
            "verify should have path field"
        );
        assert!(
            json["sub_results"].is_object(),
            "verify should have sub_results"
        );
        assert!(
            json["summary"].is_object(),
            "verify should have summary"
        );

        // Check sub_results contain expected analyses
        let sub_results = json["sub_results"].as_object().unwrap();
        assert!(
            sub_results.contains_key("contracts"),
            "sub_results should have contracts"
        );
        assert!(
            sub_results.contains_key("specs"),
            "sub_results should have specs"
        );

        // Each sub_result should have status
        for (_name, result) in sub_results {
            assert!(
                result["status"].is_string(),
                "Each sub_result should have status"
            );
            assert!(
                result["name"].is_string(),
                "Each sub_result should have name"
            );
        }

        // Summary should have coverage info
        let summary = &json["summary"];
        assert!(
            summary["coverage"].is_object() || summary["contract_count"].is_number(),
            "Summary should have coverage or contract_count"
        );
    }

    #[test]
    fn test_verify_summary_counts() {
        let (_dir, _paths) = write_fixtures(&[
            (
                "validated.py",
                r#"def check_bounds(x):
    if x < 0:
        raise ValueError("negative")
    if x > 100:
        raise ValueError("too large")
    return x
"#,
            ),
            (
                "test_validated.py",
                r#"def test_check_bounds():
    assert check_bounds(50) == 50

def test_check_bounds_zero():
    assert check_bounds(0) == 0
"#,
            ),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["verify", dir_path, "-f", "json", "-q", "--quick"]);

        let summary = &json["summary"];
        // At minimum, contracts should find preconditions
        let contract_count = summary["contract_count"].as_i64().unwrap_or(0);
        assert!(
            contract_count > 0,
            "Expected at least some contracts found"
        );

        // Spec count should reflect the test file
        let spec_count = summary["spec_count"].as_i64().unwrap_or(0);
        assert!(
            spec_count > 0,
            "Expected at least some specs from test file"
        );
    }

    #[test]
    fn test_verify_elapsed_timing() {
        let (_dir, _paths) = write_fixtures(&[(
            "simple.py",
            r#"def hello():
    return "world"
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["verify", dir_path, "-f", "json", "-q", "--quick"]);

        assert!(
            json["total_elapsed_ms"].is_number(),
            "verify should report total_elapsed_ms"
        );
        assert!(
            json["files_analyzed"].is_number(),
            "verify should report files_analyzed"
        );
    }

    #[test]
    fn test_verify_text_format() {
        let (_dir, _paths) = write_fixtures(&[(
            "simple.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let (stdout, _stderr, success) =
            run_tldr(&["verify", dir_path, "-f", "text", "-q", "--quick"]);
        assert!(success, "verify text format should succeed");
        assert!(!stdout.is_empty(), "Text output should not be empty");
    }
}

// =============================================================================
// interface command
// =============================================================================

#[cfg(test)]
mod interface {
    use super::*;

    // ---- Python ----

    #[test]
    fn test_interface_python() {
        let (_dir, path) = write_fixture(
            "module.py",
            r#"\"\"\"A sample module.\"\"\"

PUBLIC_CONST = 42
_PRIVATE_CONST = 99

class PublicClass:
    def public_method(self):
        return 1
    def _private_method(self):
        return 2

class _PrivateClass:
    def method(self):
        return 3

def public_function(x: int) -> int:
    return x + 1

def _private_helper(x):
    return x * 2
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        // Should have functions and classes arrays
        assert!(json["functions"].is_array(), "Should have functions array");
        assert!(json["classes"].is_array(), "Should have classes array");

        // Public function should be listed
        let functions = json["functions"].as_array().unwrap();
        let has_public = functions.iter().any(|f| f["name"] == "public_function");
        assert!(
            has_public,
            "Should include public_function, got: {:?}",
            functions.iter().map(|f| &f["name"]).collect::<Vec<_>>()
        );

        // Private function should NOT be listed
        let has_private = functions
            .iter()
            .any(|f| f["name"] == "_private_helper");
        assert!(
            !has_private,
            "Should NOT include _private_helper in interface"
        );

        // PublicClass should be listed
        let classes = json["classes"].as_array().unwrap();
        let has_public_class = classes.iter().any(|c| c["name"] == "PublicClass");
        assert!(
            has_public_class,
            "Should include PublicClass, got: {:?}",
            classes.iter().map(|c| &c["name"]).collect::<Vec<_>>()
        );

        // _PrivateClass should NOT be listed
        let has_private_class = classes.iter().any(|c| c["name"] == "_PrivateClass");
        assert!(
            !has_private_class,
            "Should NOT include _PrivateClass in interface"
        );
    }

    #[test]
    fn test_interface_python_signatures() {
        let (_dir, path) = write_fixture(
            "typed.py",
            r#"def greet(name: str) -> str:
    return "hello " + name

def compute(x: int, y: int = 0) -> int:
    return x + y
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();

        // Check signature includes type annotations
        let greet_fn = functions.iter().find(|f| f["name"] == "greet");
        assert!(greet_fn.is_some(), "Should find 'greet' function");

        let sig = greet_fn.unwrap()["signature"].as_str().unwrap_or("");
        assert!(
            sig.contains("str"),
            "Signature should contain type annotation 'str', got: {}",
            sig
        );
    }

    // ---- Rust ----

    #[test]
    fn test_interface_rust() {
        let (_dir, path) = write_fixture(
            "lib.rs",
            r#"pub struct Config {
    pub name: String,
    secret: String,
}

impl Config {
    pub fn new(name: String) -> Self {
        Config { name, secret: String::new() }
    }

    fn internal_helper(&self) -> &str {
        &self.secret
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

pub fn public_api(x: i32) -> i32 {
    x + 1
}

fn private_helper(x: i32) -> i32 {
    x * 2
}
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();
        let has_public = functions.iter().any(|f| f["name"] == "public_api");
        assert!(
            has_public,
            "Rust: Should include pub fn public_api, got: {:?}",
            functions.iter().map(|f| &f["name"]).collect::<Vec<_>>()
        );

        // private_helper should NOT appear
        let has_private = functions
            .iter()
            .any(|f| f["name"] == "private_helper");
        assert!(
            !has_private,
            "Rust: Should NOT include fn private_helper in interface"
        );

        // Config struct should be listed as a class
        let classes = json["classes"].as_array().unwrap();
        let has_config = classes.iter().any(|c| c["name"] == "Config");
        assert!(
            has_config,
            "Rust: Should include pub struct Config"
        );
    }

    // ---- Go ----

    #[test]
    fn test_interface_go() {
        let (_dir, path) = write_fixture(
            "module.go",
            r#"package mypackage

type Config struct {
    Name   string
    secret string
}

func NewConfig(name string) *Config {
    return &Config{Name: name}
}

func (c *Config) GetName() string {
    return c.Name
}

func (c *Config) internalHelper() string {
    return c.secret
}

func PublicFunction(x int) int {
    return x + 1
}

func privateFunction(x int) int {
    return x * 2
}
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();

        // Uppercase = exported in Go
        let has_public = functions.iter().any(|f| f["name"] == "PublicFunction");
        assert!(
            has_public,
            "Go: Should include exported PublicFunction, got: {:?}",
            functions.iter().map(|f| &f["name"]).collect::<Vec<_>>()
        );

        let has_new_config = functions.iter().any(|f| f["name"] == "NewConfig");
        assert!(
            has_new_config,
            "Go: Should include exported NewConfig"
        );

        // privateFunction should not appear (lowercase = unexported in Go)
        let has_private = functions
            .iter()
            .any(|f| f["name"] == "privateFunction");
        assert!(
            !has_private,
            "Go: Should NOT include unexported privateFunction"
        );
    }

    // ---- TypeScript ----

    #[test]
    fn test_interface_typescript() {
        let (_dir, path) = write_fixture(
            "module.ts",
            r#"export class Calculator {
    public add(a: number, b: number): number {
        return a + b;
    }

    private multiply(a: number, b: number): number {
        return a * b;
    }
}

export function publicHelper(x: number): number {
    return x + 1;
}

function privateHelper(x: number): number {
    return x * 2;
}
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        // Should have file field
        assert!(
            json["file"].is_string(),
            "interface should have file field"
        );

        // Should have at least some functions or classes
        let empty_fns = vec![];
        let empty_cls = vec![];
        let functions = json["functions"].as_array().unwrap_or(&empty_fns);
        let classes = json["classes"].as_array().unwrap_or(&empty_cls);
        assert!(
            !functions.is_empty() || !classes.is_empty(),
            "TypeScript interface should find some public items"
        );
    }

    // ---- JavaScript ----

    #[test]
    fn test_interface_javascript() {
        let (_dir, path) = write_fixture(
            "module.js",
            r#"class Calculator {
    add(a, b) {
        return a + b;
    }

    _internalCalc(x) {
        return x * 2;
    }
}

function publicHelper(x) {
    return x + 1;
}

function _privateHelper(x) {
    return x * 2;
}

module.exports = { Calculator, publicHelper };
"#,
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();
        let empty_cls = vec![];
        let classes = json["classes"].as_array().unwrap_or(&empty_cls);

        // Should find something
        assert!(
            !functions.is_empty() || !classes.is_empty(),
            "JavaScript interface should find some items"
        );

        // publicHelper should be present
        let has_public = functions.iter().any(|f| f["name"] == "publicHelper");
        assert!(
            has_public,
            "JS: Should include publicHelper, got: {:?}",
            functions.iter().map(|f| &f["name"]).collect::<Vec<_>>()
        );
    }

    // ---- Lineno and signature fields ----

    #[test]
    fn test_interface_field_completeness() {
        let (_dir, path) = write_fixture(
            "complete.py",
            "def process(data: list, limit: int = 10) -> dict:\n    \"\"\"Process the data.\"\"\"\n    return {\"result\": data[:limit]}\n",
        );
        let json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        let functions = json["functions"].as_array().unwrap();
        assert!(!functions.is_empty(), "Should find the process function");

        let func = &functions[0];
        assert!(func["name"].is_string(), "Function should have name");
        assert!(
            func["signature"].is_string(),
            "Function should have signature"
        );
        assert!(func["lineno"].is_number(), "Function should have lineno");
    }
}

// =============================================================================
// diff command
// =============================================================================

#[cfg(test)]
mod diff {
    use super::*;

    // ---- Python ----

    #[test]
    fn test_diff_python_function_modified() {
        let (_dir, paths) = write_fixtures(&[
            (
                "v1.py",
                r#"def foo():
    return 1

def bar():
    return 2
"#,
            ),
            (
                "v2.py",
                r#"def foo():
    return 99

def bar():
    return 2

def baz():
    return 3
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);

        assert_eq!(json["identical"], false, "Files should not be identical");

        let changes = json["changes"].as_array().expect("changes array");
        assert!(
            changes.len() >= 2,
            "Expected at least 2 changes (update foo + insert baz), got {}",
            changes.len()
        );

        // foo should be updated
        let foo_update = changes
            .iter()
            .find(|c| c["name"] == "foo" && c["change_type"] == "update");
        assert!(
            foo_update.is_some(),
            "Expected foo to be updated, got: {:?}",
            changes
                .iter()
                .map(|c| format!("{}:{}", c["name"], c["change_type"]))
                .collect::<Vec<_>>()
        );

        // baz should be inserted
        let baz_insert = changes
            .iter()
            .find(|c| c["name"] == "baz" && c["change_type"] == "insert");
        assert!(
            baz_insert.is_some(),
            "Expected baz to be inserted"
        );

        // Summary should be present
        let summary = &json["summary"];
        assert!(summary["total_changes"].as_i64().unwrap_or(0) >= 2);
        assert_eq!(summary["updates"].as_i64().unwrap_or(0), 1);
        assert_eq!(summary["inserts"].as_i64().unwrap_or(0), 1);
    }

    #[test]
    fn test_diff_python_function_deleted() {
        let (_dir, paths) = write_fixtures(&[
            (
                "v1.py",
                r#"def foo():
    return 1

def bar():
    return 2
"#,
            ),
            (
                "v2.py",
                r#"def foo():
    return 1
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);
        assert_eq!(json["identical"], false);

        let changes = json["changes"].as_array().unwrap();
        let bar_delete = changes
            .iter()
            .find(|c| c["name"] == "bar" && c["change_type"] == "delete");
        assert!(
            bar_delete.is_some(),
            "Expected bar to be deleted, got: {:?}",
            changes
                .iter()
                .map(|c| format!("{}:{}", c["name"], c["change_type"]))
                .collect::<Vec<_>>()
        );

        assert!(
            json["summary"]["deletes"].as_i64().unwrap_or(0) >= 1,
            "Summary should show at least 1 delete"
        );
    }

    #[test]
    fn test_diff_python_identical() {
        let (_dir, paths) = write_fixtures(&[
            (
                "a.py",
                r#"def foo():
    return 1
"#,
            ),
            (
                "b.py",
                r#"def foo():
    return 1
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);
        assert_eq!(json["identical"], true, "Identical files should report identical=true");
        let changes = json["changes"].as_array().unwrap();
        assert!(changes.is_empty(), "Identical files should have no changes");
    }

    // ---- JavaScript ----

    #[test]
    fn test_diff_javascript() {
        let (_dir, paths) = write_fixtures(&[
            (
                "v1.js",
                r#"function greet(name) {
    return "hello " + name;
}
function farewell(name) {
    return "bye " + name;
}
"#,
            ),
            (
                "v2.js",
                r#"function greet(name) {
    return "hi " + name;
}
function farewell(name) {
    return "bye " + name;
}
function wave(name) {
    return "waves at " + name;
}
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);
        assert_eq!(json["identical"], false);

        let changes = json["changes"].as_array().unwrap();
        let greet_update = changes
            .iter()
            .find(|c| c["name"] == "greet" && c["change_type"] == "update");
        assert!(greet_update.is_some(), "JS: greet should be updated");

        let wave_insert = changes
            .iter()
            .find(|c| c["name"] == "wave" && c["change_type"] == "insert");
        assert!(wave_insert.is_some(), "JS: wave should be inserted");
    }

    // ---- Go ----

    #[test]
    fn test_diff_go() {
        let (_dir, paths) = write_fixtures(&[
            (
                "v1.go",
                r#"package main

func add(a, b int) int {
    return a + b
}

func sub(a, b int) int {
    return a - b
}
"#,
            ),
            (
                "v2.go",
                r#"package main

func add(a, b int) int {
    return a + b + 1
}

func sub(a, b int) int {
    return a - b
}

func mul(a, b int) int {
    return a * b
}
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);
        assert_eq!(json["identical"], false);

        let changes = json["changes"].as_array().unwrap();
        assert!(
            !changes.is_empty(),
            "Go diff should detect changes"
        );

        // add should be modified
        let add_change = changes
            .iter()
            .find(|c| c["name"] == "add");
        assert!(
            add_change.is_some(),
            "Go: add function should be detected as changed"
        );
    }

    // ---- Granularity ----

    #[test]
    fn test_diff_class_granularity() {
        let (_dir, paths) = write_fixtures(&[
            (
                "v1.py",
                r#"class Foo:
    def method_a(self):
        return 1

class Bar:
    def method_b(self):
        return 2
"#,
            ),
            (
                "v2.py",
                r#"class Foo:
    def method_a(self):
        return 99

class Bar:
    def method_b(self):
        return 2

class Baz:
    def method_c(self):
        return 3
"#,
            ),
        ]);

        let json = run_tldr_json(&[
            "diff",
            &paths[0].1,
            &paths[1].1,
            "-g",
            "class",
            "-f",
            "json",
            "-q",
        ]);
        assert_eq!(json["identical"], false);
        assert_eq!(json["granularity"], "class");

        let changes = json["changes"].as_array().unwrap();
        assert!(
            !changes.is_empty(),
            "Class-level diff should detect changes"
        );
    }

    // ---- Location fields ----

    #[test]
    fn test_diff_location_fields() {
        let (_dir, paths) = write_fixtures(&[
            (
                "a.py",
                r#"def foo():
    return 1
"#,
            ),
            (
                "b.py",
                r#"def foo():
    return 2
"#,
            ),
        ]);

        let json = run_tldr_json(&["diff", &paths[0].1, &paths[1].1, "-f", "json", "-q"]);
        let changes = json["changes"].as_array().unwrap();
        assert!(!changes.is_empty(), "Should have at least one change");

        let change = &changes[0];
        // Updated items should have old_location and new_location
        if change["change_type"] == "update" {
            assert!(
                change["old_location"].is_object(),
                "Update should have old_location"
            );
            assert!(
                change["new_location"].is_object(),
                "Update should have new_location"
            );
            assert!(
                change["old_location"]["line"].is_number(),
                "Location should have line"
            );
        }
    }
}

// =============================================================================
// debt command
// =============================================================================

#[cfg(test)]
mod debt {
    use super::*;

    #[test]
    fn test_debt_python_project() {
        let (_dir, _paths) = write_fixtures(&[
            (
                "main.py",
                r#"def very_long_function_with_lots_of_logic(a, b, c, d, e, f, g, h):
    x = a + b
    y = c + d
    z = e + f
    w = g + h
    if x > 0:
        if y > 0:
            if z > 0:
                if w > 0:
                    return x + y + z + w
                else:
                    return x + y + z
            else:
                return x + y
        else:
            return x
    else:
        return 0

def another_undocumented_function():
    pass
"#,
            ),
            (
                "utils.py",
                r#"def helper(x):
    return x + 1
"#,
            ),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["debt", dir_path, "-f", "json", "-q"]);

        // Should have issues array
        assert!(json["issues"].is_array(), "debt should have issues array");

        let issues = json["issues"].as_array().unwrap();
        assert!(
            !issues.is_empty(),
            "Expected at least some debt issues for undocumented code"
        );

        // Each issue should have required fields
        for issue in issues {
            assert!(issue["file"].is_string(), "Issue should have file");
            assert!(issue["rule"].is_string(), "Issue should have rule");
            assert!(issue["category"].is_string(), "Issue should have category");
            assert!(
                issue["debt_minutes"].is_number(),
                "Issue should have debt_minutes"
            );
        }

        // Should find missing docs
        let has_missing_docs = issues.iter().any(|i| {
            i["rule"].as_str().unwrap_or("") == "missing_docs"
        });
        assert!(
            has_missing_docs,
            "Should detect missing documentation as debt"
        );
    }

    #[test]
    fn test_debt_category_filter() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def undocumented():
    pass
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&[
            "debt",
            dir_path,
            "-c",
            "maintainability",
            "-f",
            "json",
            "-q",
        ]);

        let issues = json["issues"].as_array().unwrap();
        for issue in issues {
            let category = issue["category"].as_str().unwrap_or("");
            assert_eq!(
                category, "maintainability",
                "With category filter, all issues should be maintainability, got: {}",
                category
            );
        }
    }

    #[test]
    fn test_debt_summary_fields() {
        let (_dir, _paths) = write_fixtures(&[(
            "simple.py",
            r#"def foo():
    return 1

def bar():
    return 2
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["debt", dir_path, "-f", "json", "-q"]);

        // Should have summary or per_category or total_debt
        let has_summary = json["summary"].is_object()
            || json["total_debt_minutes"].is_number()
            || json["per_category"].is_object()
            || json["issues"].is_array();
        assert!(
            has_summary,
            "debt should have summary/total_debt or issues, got keys: {:?}",
            json.as_object().map(|m| m.keys().collect::<Vec<_>>())
        );
    }

    #[test]
    fn test_debt_text_format() {
        let (_dir, _paths) = write_fixtures(&[(
            "simple.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let (stdout, _stderr, success) =
            run_tldr(&["debt", dir_path, "-f", "text", "-q"]);
        assert!(success, "debt text format should succeed");
        assert!(!stdout.is_empty(), "Text output should not be empty");
    }

    #[test]
    fn test_debt_top_limit() {
        let (_dir, _paths) = write_fixtures(&[
            ("a.py", "def a(): pass\n"),
            ("b.py", "def b(): pass\n"),
            ("c.py", "def c(): pass\n"),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["debt", dir_path, "-k", "2", "-f", "json", "-q"]);

        // With --top 2, the per-file breakdown should be capped
        assert!(
            json["issues"].is_array(),
            "debt should still have issues with --top"
        );
    }
}

// =============================================================================
// health command
// =============================================================================

#[cfg(test)]
mod health {
    use super::*;

    #[test]
    fn test_health_python_project() {
        let (_dir, _paths) = write_fixtures(&[
            (
                "main.py",
                r#"class Calculator:
    def add(self, a, b):
        return a + b

    def subtract(self, a, b):
        return a - b

    def multiply(self, a, b):
        return a * b

def standalone():
    return 42
"#,
            ),
            (
                "utils.py",
                r#"def helper(x):
    if x > 0:
        return x
    elif x < 0:
        return -x
    else:
        return 0
"#,
            ),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["health", dir_path, "-f", "json", "-q", "--quick"]);

        // Top-level structure
        assert_eq!(json["wrapper"], "health");
        assert!(json["summary"].is_object(), "health should have summary");
        assert!(
            json["total_elapsed_ms"].is_number(),
            "health should have total_elapsed_ms"
        );

        // Summary metrics
        let summary = &json["summary"];
        assert!(
            summary["functions_analyzed"].is_number(),
            "Summary should have functions_analyzed"
        );
        assert!(
            summary["classes_analyzed"].is_number(),
            "Summary should have classes_analyzed"
        );
    }

    #[test]
    fn test_health_summary_metrics() {
        let (_dir, _paths) = write_fixtures(&[(
            "complex.py",
            r#"def complex_function(x, y, z):
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
            else:
                return x + y
        else:
            return x
    else:
        return 0

def simple_function():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["health", dir_path, "-f", "json", "-q", "--quick"]);

        let summary = &json["summary"];
        let fn_count = summary["functions_analyzed"].as_i64().unwrap_or(0);
        assert!(fn_count >= 2, "Should analyze at least 2 functions");

        // avg_cyclomatic should be present and reasonable
        let avg_cc = summary["avg_cyclomatic"].as_f64().unwrap_or(0.0);
        assert!(
            avg_cc > 0.0,
            "Average cyclomatic complexity should be > 0"
        );
    }

    #[test]
    fn test_health_details_section() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["health", dir_path, "-f", "json", "-q", "--quick"]);

        // Details section should be present
        assert!(
            json["details"].is_object(),
            "health should have details"
        );

        let details = json["details"].as_object().unwrap();
        // Should have complexity sub-analysis at minimum
        assert!(
            details.contains_key("complexity"),
            "details should have complexity analysis"
        );

        let complexity = &details["complexity"];
        assert!(
            complexity["success"].as_bool().unwrap_or(false),
            "Complexity analysis should succeed"
        );
    }

    #[test]
    fn test_health_quick_mode() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["health", dir_path, "-f", "json", "-q", "--quick"]);

        assert_eq!(
            json["quick_mode"], true,
            "Should report quick_mode=true"
        );
    }

    #[test]
    fn test_health_preset() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&[
            "health",
            dir_path,
            "--preset",
            "strict",
            "-f",
            "json",
            "-q",
            "--quick",
        ]);

        // Should succeed with strict preset
        assert!(
            json["summary"].is_object(),
            "health with strict preset should have summary"
        );
    }

    #[test]
    fn test_health_text_format() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let (stdout, _stderr, success) =
            run_tldr(&["health", dir_path, "-f", "text", "-q", "--quick"]);
        assert!(success, "health text format should succeed");
        assert!(!stdout.is_empty(), "Text output should not be empty");
    }

    #[test]
    fn test_health_summary_mode() {
        let (_dir, _paths) = write_fixtures(&[(
            "code.py",
            r#"def foo():
    return 1
"#,
        )]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&[
            "health",
            dir_path,
            "--summary",
            "-f",
            "json",
            "-q",
            "--quick",
        ]);

        assert!(
            json["summary"].is_object(),
            "Summary mode should still have summary"
        );
    }
}

// =============================================================================
// coverage command
// =============================================================================

#[cfg(test)]
mod coverage {
    use super::*;

    // ---- LCOV format ----

    #[test]
    fn test_coverage_lcov_basic() {
        let (_dir, path) = write_fixture(
            "coverage.lcov",
            r#"SF:main.py
DA:1,1
DA:2,1
DA:3,0
DA:4,1
DA:5,0
LF:5
LH:3
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "-f", "json", "-q"]);

        assert_eq!(json["format"], "lcov", "Should detect LCOV format");

        let summary = &json["summary"];
        assert!(
            summary["line_coverage"].is_number(),
            "Should have line_coverage"
        );
        assert!(
            summary["total_lines"].is_number(),
            "Should have total_lines"
        );
        assert!(
            summary["covered_lines"].is_number(),
            "Should have covered_lines"
        );

        let coverage = summary["line_coverage"].as_f64().unwrap();
        assert!(
            (coverage - 60.0).abs() < 0.1,
            "Coverage should be 60% (3/5), got {}",
            coverage
        );
    }

    #[test]
    fn test_coverage_lcov_multi_file() {
        let (_dir, path) = write_fixture(
            "multi.lcov",
            r#"SF:main.py
DA:1,1
DA:2,1
DA:3,0
LF:3
LH:2
end_of_record
SF:utils.py
DA:1,1
DA:2,1
DA:3,1
LF:3
LH:3
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "-f", "json", "-q"]);

        let summary = &json["summary"];
        let total = summary["total_lines"].as_i64().unwrap();
        let covered = summary["covered_lines"].as_i64().unwrap();
        assert_eq!(total, 6, "Total lines should be 6 (3+3)");
        assert_eq!(covered, 5, "Covered lines should be 5 (2+3)");

        // Coverage should be ~83.3%
        let pct = summary["line_coverage"].as_f64().unwrap();
        assert!(
            (pct - 83.33).abs() < 1.0,
            "Coverage should be ~83.3%, got {}",
            pct
        );
    }

    #[test]
    fn test_coverage_lcov_threshold_met() {
        let (_dir, path) = write_fixture(
            "full.lcov",
            r#"SF:main.py
DA:1,1
DA:2,1
DA:3,1
DA:4,1
DA:5,1
LF:5
LH:5
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "--threshold", "80", "-f", "json", "-q"]);

        let summary = &json["summary"];
        assert_eq!(
            summary["threshold_met"], true,
            "100% coverage should meet 80% threshold"
        );
    }

    #[test]
    fn test_coverage_lcov_threshold_not_met() {
        let (_dir, path) = write_fixture(
            "low.lcov",
            r#"SF:main.py
DA:1,1
DA:2,0
DA:3,0
DA:4,0
DA:5,0
LF:5
LH:1
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "--threshold", "80", "-f", "json", "-q"]);

        let summary = &json["summary"];
        assert_eq!(
            summary["threshold_met"], false,
            "20% coverage should NOT meet 80% threshold"
        );
    }

    // ---- Cobertura XML format ----

    #[test]
    fn test_coverage_cobertura_xml() {
        let (_dir, path) = write_fixture(
            "coverage.xml",
            r#"<?xml version="1.0" ?>
<coverage version="5.5" timestamp="1234567890" lines-valid="10" lines-covered="7" line-rate="0.7" branches-valid="0" branches-covered="0" branch-rate="0" complexity="0">
    <packages>
        <package name="." line-rate="0.7" branch-rate="0" complexity="0">
            <classes>
                <class name="main.py" filename="main.py" line-rate="0.7" branch-rate="0" complexity="0">
                    <lines>
                        <line number="1" hits="1"/>
                        <line number="2" hits="1"/>
                        <line number="3" hits="0"/>
                        <line number="4" hits="1"/>
                        <line number="5" hits="1"/>
                        <line number="6" hits="0"/>
                        <line number="7" hits="1"/>
                        <line number="8" hits="1"/>
                        <line number="9" hits="0"/>
                        <line number="10" hits="1"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "-f", "json", "-q"]);

        // Should detect cobertura format
        let format = json["format"].as_str().unwrap_or("");
        assert!(
            format == "cobertura" || format == "auto",
            "Should detect cobertura format, got: {}",
            format
        );

        let summary = &json["summary"];
        assert!(
            summary["line_coverage"].is_number(),
            "Cobertura should have line_coverage"
        );
        let pct = summary["line_coverage"].as_f64().unwrap();
        assert!(
            pct > 0.0 && pct <= 100.0,
            "Coverage should be between 0 and 100, got {}",
            pct
        );
    }

    // ---- coverage.py JSON format ----

    #[test]
    fn test_coverage_coveragepy_json() {
        let (_dir, path) = write_fixture(
            "coverage.json",
            r#"{
    "meta": {
        "version": "5.5",
        "timestamp": "2024-01-01T00:00:00",
        "branch_coverage": false,
        "show_contexts": false
    },
    "files": {
        "main.py": {
            "executed_lines": [1, 2, 4, 5],
            "summary": {
                "covered_lines": 4,
                "num_statements": 6,
                "percent_covered": 66.67,
                "missing_lines": 2,
                "excluded_lines": 0
            },
            "missing_lines": [3, 6],
            "excluded_lines": []
        }
    },
    "totals": {
        "covered_lines": 4,
        "num_statements": 6,
        "percent_covered": 66.67,
        "missing_lines": 2,
        "excluded_lines": 0
    }
}
"#,
        );

        let json = run_tldr_json(&[
            "coverage",
            &path,
            "-R",
            "coveragepy",
            "-f",
            "json",
            "-q",
        ]);

        let summary = &json["summary"];
        assert!(
            summary["line_coverage"].is_number(),
            "coverage.py should have line_coverage"
        );
    }

    // ---- Format auto-detection ----

    #[test]
    fn test_coverage_auto_detect_lcov() {
        let (_dir, path) = write_fixture(
            "report.info",
            r#"SF:main.py
DA:1,1
DA:2,0
LF:2
LH:1
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "-f", "json", "-q"]);
        assert_eq!(
            json["format"], "lcov",
            "Should auto-detect LCOV format from content"
        );
    }

    // ---- Text format ----

    #[test]
    fn test_coverage_text_format() {
        let (_dir, path) = write_fixture(
            "coverage.lcov",
            r#"SF:main.py
DA:1,1
DA:2,1
DA:3,0
LF:3
LH:2
end_of_record
"#,
        );

        let (stdout, _stderr, success) =
            run_tldr(&["coverage", &path, "-f", "text", "-q"]);
        assert!(success, "coverage text format should succeed");
        assert!(
            !stdout.is_empty(),
            "Text output should not be empty"
        );
    }

    // ---- Empty/edge cases ----

    #[test]
    fn test_coverage_lcov_empty_file() {
        let (_dir, path) = write_fixture(
            "empty.lcov",
            r#"SF:empty.py
LF:0
LH:0
end_of_record
"#,
        );

        // Should handle zero lines gracefully (no division by zero)
        let (stdout, stderr, success) =
            run_tldr(&["coverage", &path, "-f", "json", "-q"]);
        // Either succeeds with 0% or handles the edge case
        if success {
            let json: Value = serde_json::from_str(&stdout).unwrap();
            let summary = &json["summary"];
            let total = summary["total_lines"].as_i64().unwrap_or(0);
            assert_eq!(total, 0, "Empty file should have 0 total lines");
        } else {
            // It's acceptable for the tool to report an error on empty coverage
            assert!(
                !stderr.is_empty(),
                "Error case should have stderr output"
            );
        }
    }

    #[test]
    fn test_coverage_lcov_100_percent() {
        let (_dir, path) = write_fixture(
            "perfect.lcov",
            r#"SF:main.py
DA:1,1
DA:2,1
DA:3,1
LF:3
LH:3
end_of_record
"#,
        );

        let json = run_tldr_json(&["coverage", &path, "-f", "json", "-q"]);
        let pct = json["summary"]["line_coverage"].as_f64().unwrap();
        assert!(
            (pct - 100.0).abs() < 0.01,
            "Full coverage should be 100%, got {}",
            pct
        );
    }
}

// =============================================================================
// Cross-command integration: verify uses contracts + specs
// =============================================================================

#[cfg(test)]
mod cross_command {
    use super::*;

    #[test]
    fn test_verify_integrates_contracts_and_specs() {
        let (_dir, _paths) = write_fixtures(&[
            (
                "secure.py",
                r#"def safe_divide(a, b):
    if b == 0:
        raise ValueError("division by zero")
    if not isinstance(a, (int, float)):
        raise TypeError("a must be numeric")
    return a / b
"#,
            ),
            (
                "test_secure.py",
                r#"def test_safe_divide():
    assert safe_divide(10, 2) == 5

def test_safe_divide_float():
    assert safe_divide(7.5, 2.5) == 3.0
"#,
            ),
        ]);

        let dir_path = _dir.path().to_str().unwrap();
        let json = run_tldr_json(&["verify", dir_path, "-f", "json", "-q", "--quick"]);

        let sub = json["sub_results"].as_object().unwrap();

        // Contracts sub-result should find guard clauses
        if let Some(contracts) = sub.get("contracts") {
            let status = contracts["status"].as_str().unwrap_or("");
            assert!(
                status == "success" || status == "partial",
                "Contracts should succeed or partially succeed"
            );
            let items = contracts["items_found"].as_i64().unwrap_or(0);
            assert!(
                items > 0,
                "Contracts should find preconditions from guard clauses"
            );
        }

        // Specs sub-result should find test specs
        if let Some(specs) = sub.get("specs") {
            let status = specs["status"].as_str().unwrap_or("");
            assert!(
                status == "success" || status == "partial",
                "Specs should succeed"
            );
            let items = specs["items_found"].as_i64().unwrap_or(0);
            assert!(items > 0, "Specs should find test-derived specs");
        }
    }

    #[test]
    fn test_contracts_then_interface_consistency() {
        // A function found by contracts should also be found by interface
        let (_dir, path) = write_fixture(
            "api.py",
            r#"def validate_input(data):
    if data is None:
        raise ValueError("data is required")
    if not isinstance(data, dict):
        raise TypeError("data must be a dict")
    return data
"#,
        );

        let contracts_json =
            run_tldr_json(&["contracts", &path, "validate_input", "-f", "json", "-q"]);
        let interface_json = run_tldr_json(&["interface", &path, "-f", "json", "-q"]);

        // Contracts should find the function
        assert_eq!(contracts_json["function"], "validate_input");

        // Interface should also list it (it's public -- no underscore prefix)
        let functions = interface_json["functions"].as_array().unwrap();
        let in_interface = functions
            .iter()
            .any(|f| f["name"] == "validate_input");
        assert!(
            in_interface,
            "Function found by contracts should also appear in interface"
        );
    }
}
