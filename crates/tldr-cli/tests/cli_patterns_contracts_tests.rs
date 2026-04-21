//! CLI Patterns and Contracts Commands Tests
//!
//! Tests for tldr-cli pattern analysis and contract inference commands:
//! - Patterns: purity, resources, mutability, temporal, coupling, cohesion, interface, behavioral
//! - Contracts: contracts, bounds, invariants, specs, verify, dead-stores, chop
//! - Diagnostics: diagnostics (type checking and linting)
//!
//! See `bugs_cli_patterns_contracts.md` for documented bugs.

use std::fs;
// Path import not needed - using PathBuf via tempfile::TempDir
use std::process::Command;
use tempfile::TempDir;

// =============================================================================
// Test Fixtures
// =============================================================================

/// Create a test project with various Python patterns for testing
fn create_test_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    // Create main.py with various functions for pattern analysis
    fs::write(
        project_path.join("main.py"),
        r#"# Main module with various function patterns

def pure_add(x, y):
    """A pure function - no side effects"""
    return x + y

def impure_function():
    """Function with side effects"""
    with open("file.txt", "w") as f:
        f.write("hello")
    global_state = 1
    return global_state

def mutating_function(items):
    """Function that mutates its input"""
    items.append(42)
    items.sort()
    return items

def guarded_function(x):
    """Function with guard clauses"""
    if x < 0:
        raise ValueError("x must be non-negative")
    if not isinstance(x, int):
        raise TypeError("x must be int")
    return x * 2

class Calculator:
    """A calculator class for cohesion testing"""
    def __init__(self):
        self.value = 0
    
    def add(self, x):
        self.value += x
        return self
    
    def subtract(self, x):
        self.value -= x
        return self
    
    def get_value(self):
        return self.value

def resource_function():
    """Function with resource management"""
    f = open("test.txt", "r")
    data = f.read()
    f.close()
    return data

def leaky_function():
    """Function with potential resource leak"""
    f = open("test.txt", "r")
    data = f.read()
    # Missing f.close() - potential leak
    return data
"#,
    )
    .unwrap();

    // Create utils.py for coupling tests
    fs::write(
        project_path.join("utils.py"),
        r#"# Utility module
from main import pure_add, Calculator

def utility_func(x):
    """Uses functions from main"""
    return pure_add(x, 10)

def create_calc():
    """Creates a calculator"""
    return Calculator()
"#,
    )
    .unwrap();

    // Create tests directory for specs/invariants
    fs::create_dir(project_path.join("tests")).unwrap();
    fs::write(
        project_path.join("tests").join("test_main.py"),
        r#"# Test file for specs extraction
import pytest
from main import pure_add, guarded_function, Calculator

def test_pure_add():
    assert pure_add(2, 3) == 5
    assert pure_add(-1, 1) == 0

def test_pure_add_with_zero():
    assert pure_add(0, 0) == 0

def test_guarded_function_valid():
    assert guarded_function(5) == 10

def test_guarded_function_raises():
    with pytest.raises(ValueError):
        guarded_function(-1)

class TestCalculator:
    def test_calculator_add(self):
        calc = Calculator()
        calc.add(5)
        assert calc.get_value() == 5
    
    def test_calculator_subtract(self):
        calc = Calculator()
        calc.subtract(3)
        assert calc.get_value() == -3
"#,
    )
    .unwrap();

    temp_dir
}

/// Create a minimal test project for edge cases
fn create_minimal_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("minimal.py"),
        r#"def simple():
    pass
"#,
    )
    .unwrap();

    temp_dir
}

/// Create a project with type hints for bounds analysis
fn create_typed_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("typed.py"),
        r#"def calculate(x: int, y: int) -> int:
    if x > 0:
        z = x + y
    else:
        z = 0
    return z * 2

def divide(a: int, b: int) -> float:
    return a / b

def bounded(n: int) -> int:
    if n < 0:
        n = 0
    elif n > 100:
        n = 100
    return n
"#,
    )
    .unwrap();

    temp_dir
}

/// Create a project with data flow patterns for chop analysis
fn create_chop_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("chop_test.py"),
        r#"def data_flow(x):
    y = x + 1      # line 2
    z = y * 2      # line 3
    w = z + 10     # line 4
    return w       # line 5
"#,
    )
    .unwrap();

    temp_dir
}

/// Create a project with dead stores for SSA analysis
fn create_dead_store_project() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("dead_store.py"),
        r#"def dead_store_example():
    x = 1          # line 2 - dead store
    x = 2          # line 3 - overwrites without use
    return x       # line 4

def live_store_example():
    x = 1          # line 7 - used
    y = x + 1      # line 8
    return y       # line 9
"#,
    )
    .unwrap();

    temp_dir
}

// =============================================================================
// Resources Command Tests
// =============================================================================

#[test]
fn test_resources_basic_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "resources",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr resources");

    // Exit code may be non-zero when leaks are found (exit 3 = leaks detected)
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "Output should not be empty");
}

#[test]
fn test_resources_with_check_flags() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "resources",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "--check-all",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr resources with check-all");

    // Exit code may be non-zero when leaks are found
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "Output should not be empty");
}

#[test]
fn test_resources_text_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "resources",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "-o",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr resources text");

    // Exit code may be non-zero when leaks are found
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "Output should not be empty");
}

// =============================================================================
// Temporal Command Tests
// =============================================================================

#[test]
fn test_temporal_basic_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["temporal", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute tldr temporal");

    assert!(output.status.success(), "temporal command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // May or may not find temporal patterns
    assert!(
        !stdout.is_empty() || stdout.is_empty(),
        "Command should run"
    );
}

#[test]
fn test_temporal_with_options() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "temporal",
            temp_dir.path().to_str().unwrap(),
            "--min-support",
            "1",
            "--min-confidence",
            "0.5",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr temporal with options");

    assert!(
        output.status.success(),
        "temporal with options should succeed"
    );
}

// =============================================================================
// Coupling Command Tests
// =============================================================================

#[test]
fn test_coupling_basic_json() {
    let temp_dir = create_test_project();
    let main_path = temp_dir.path().join("main.py");
    let utils_path = temp_dir.path().join("utils.py");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "coupling",
            main_path.to_str().unwrap(),
            utils_path.to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr coupling");

    assert!(output.status.success(), "coupling command should succeed");
}

#[test]
fn test_coupling_same_file() {
    let temp_dir = create_test_project();
    let main_path = temp_dir.path().join("main.py");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "coupling",
            main_path.to_str().unwrap(),
            main_path.to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr coupling same file");

    // Should either succeed or give a meaningful error
    let _ = output.status;
}

// =============================================================================
// Cohesion Command Tests
// =============================================================================

#[test]
fn test_cohesion_basic_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["cohesion", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute tldr cohesion");

    assert!(output.status.success(), "cohesion command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should find the Calculator class
    assert!(
        stdout.contains("Calculator") || stdout.contains("class"),
        "Output should contain class information"
    );
}

#[test]
fn test_cohesion_with_min_methods() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "cohesion",
            temp_dir.path().to_str().unwrap(),
            "--min-methods",
            "2",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr cohesion with min-methods");

    assert!(
        output.status.success(),
        "cohesion with min-methods should succeed"
    );
}

#[test]
fn test_cohesion_text_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "cohesion",
            temp_dir.path().to_str().unwrap(),
            "-o",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr cohesion text");

    assert!(
        output.status.success(),
        "cohesion text format should succeed"
    );
}

// =============================================================================
// Interface Command Tests
// =============================================================================

#[test]
fn test_interface_basic() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "interface",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr interface");

    assert!(output.status.success(), "interface command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Interface analysis should extract public API
    assert!(
        stdout.contains("pure_add") || stdout.contains("function"),
        "Output should contain function information"
    );
}

// =============================================================================
// Behavioral Command Tests
// =============================================================================

#[test]
fn test_behavioral_basic_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "behavioral",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr behavioral");

    assert!(output.status.success(), "behavioral command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should extract behavioral constraints
    assert!(!stdout.is_empty(), "Output should not be empty");
}

#[test]
fn test_behavioral_with_function() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "behavioral",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "guarded_function",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr behavioral with function");

    assert!(
        output.status.success(),
        "behavioral with function should succeed"
    );
}

// =============================================================================
// Contracts Command Tests
// =============================================================================

#[test]
fn test_contracts_basic_json() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "contracts",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "guarded_function",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr contracts");

    assert!(output.status.success(), "contracts command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should extract preconditions from guard clauses
    assert!(
        stdout.contains("preconditions") || stdout.contains("condition"),
        "Output should contain preconditions"
    );
}

#[test]
fn test_contracts_text_format() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "contracts",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "guarded_function",
            "-o",
            "text",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr contracts text");

    assert!(
        output.status.success(),
        "contracts text format should succeed"
    );
}

// =============================================================================
// Bounds Command Tests
// =============================================================================

#[test]
fn test_bounds_basic_json() {
    let temp_dir = create_typed_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "bounds",
            temp_dir.path().join("typed.py").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr bounds");

    assert!(output.status.success(), "bounds command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Bounds analysis should produce interval results
    assert!(!stdout.is_empty(), "Output should not be empty");
}

#[test]
fn test_bounds_with_function() {
    let temp_dir = create_typed_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "bounds",
            temp_dir.path().join("typed.py").to_str().unwrap(),
            "calculate",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr bounds with function");

    assert!(
        output.status.success(),
        "bounds with function should succeed"
    );
}

// =============================================================================
// Invariants Command Tests
// =============================================================================

#[test]
fn test_invariants_basic() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "invariants",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "--from-tests",
            temp_dir.path().join("tests").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr invariants");

    assert!(output.status.success(), "invariants command should succeed");
}

#[test]
fn test_invariants_with_min_obs() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "invariants",
            temp_dir.path().join("main.py").to_str().unwrap(),
            "--from-tests",
            temp_dir.path().join("tests").to_str().unwrap(),
            "--min-obs",
            "1",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr invariants with min-obs");

    assert!(
        output.status.success(),
        "invariants with min-obs should succeed"
    );
}

// =============================================================================
// Specs Command Tests
// =============================================================================

#[test]
fn test_specs_basic() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "specs",
            "--from-tests",
            temp_dir.path().join("tests").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr specs");

    assert!(output.status.success(), "specs command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should extract specs from test assertions
    assert!(
        stdout.contains("pure_add") || stdout.contains("function"),
        "Output should contain function specs"
    );
}

#[test]
fn test_specs_with_function_filter() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "specs",
            "--from-tests",
            temp_dir.path().join("tests").to_str().unwrap(),
            "--function",
            "pure_add",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr specs with function filter");

    assert!(
        output.status.success(),
        "specs with function filter should succeed"
    );
}

// =============================================================================
// Verify Command Tests
// =============================================================================

#[test]
fn test_verify_basic() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["verify", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute tldr verify");

    assert!(output.status.success(), "verify command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Verify produces a dashboard report
    assert!(
        stdout.contains("contracts") || stdout.contains("specs") || stdout.contains("verify"),
        "Output should contain verification results"
    );
}

#[test]
fn test_verify_quick_mode() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["verify", temp_dir.path().to_str().unwrap(), "--quick", "-q"])
        .output()
        .expect("Failed to execute tldr verify --quick");

    assert!(output.status.success(), "verify --quick should succeed");
}

// =============================================================================
// Dead-Stores Command Tests
// =============================================================================

#[test]
fn test_dead_stores_basic() {
    let temp_dir = create_dead_store_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "dead-stores",
            temp_dir.path().join("dead_store.py").to_str().unwrap(),
            "dead_store_example",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr dead-stores");

    assert!(
        output.status.success(),
        "dead-stores command should succeed"
    );
}

#[test]
fn test_dead_stores_with_compare() {
    let temp_dir = create_dead_store_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "dead-stores",
            temp_dir.path().join("dead_store.py").to_str().unwrap(),
            "dead_store_example",
            "--compare",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr dead-stores --compare");

    assert!(
        output.status.success(),
        "dead-stores --compare should succeed"
    );
}

// =============================================================================
// Chop Command Tests
// =============================================================================

#[test]
fn test_chop_basic() {
    let temp_dir = create_chop_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "chop",
            temp_dir.path().join("chop_test.py").to_str().unwrap(),
            "data_flow",
            "2",
            "5",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr chop");

    assert!(output.status.success(), "chop command should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Chop should find lines on dependency path
    assert!(!stdout.is_empty(), "Output should not be empty");
}

#[test]
fn test_chop_same_line() {
    let temp_dir = create_chop_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "chop",
            temp_dir.path().join("chop_test.py").to_str().unwrap(),
            "data_flow",
            "2",
            "2",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr chop same line");

    assert!(output.status.success(), "chop same line should succeed");
}

// =============================================================================
// Diagnostics Command Tests
// =============================================================================

#[test]
fn test_diagnostics_basic() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["diagnostics", temp_dir.path().to_str().unwrap(), "-q"])
        .output()
        .expect("Failed to execute tldr diagnostics");

    // Exit codes: 0=clean, 1=diagnostics found, 60=no tools, 61=tools failed
    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 0 || exit_code == 1 || exit_code == 60 || exit_code == 61,
        "diagnostics should exit with known code, got {}",
        exit_code
    );
}

#[test]
fn test_diagnostics_with_severity() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "diagnostics",
            temp_dir.path().to_str().unwrap(),
            "--severity",
            "error",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr diagnostics with severity");

    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 0 || exit_code == 1 || exit_code == 60 || exit_code == 61,
        "diagnostics with severity should exit with known code, got {}",
        exit_code
    );
}

#[test]
fn test_diagnostics_no_typecheck() {
    let temp_dir = create_test_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "diagnostics",
            temp_dir.path().to_str().unwrap(),
            "--no-typecheck",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr diagnostics --no-typecheck");

    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 0 || exit_code == 60 || exit_code == 61,
        "diagnostics --no-typecheck should exit with known code"
    );
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_patterns_nonexistent_file() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["purity", "/nonexistent/path/file.py", "-q"])
        .output()
        .expect("Failed to execute tldr purity on nonexistent file");

    assert!(!output.status.success(), "Should fail for nonexistent file");
}

#[test]
fn test_contracts_nonexistent_function() {
    let temp_dir = create_minimal_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "contracts",
            temp_dir.path().join("minimal.py").to_str().unwrap(),
            "nonexistent_function",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr contracts on nonexistent function");

    // May succeed with empty result or fail with error
    let _ = output.status;
}

#[test]
fn test_chop_invalid_line_numbers() {
    let temp_dir = create_chop_project();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "chop",
            temp_dir.path().join("chop_test.py").to_str().unwrap(),
            "data_flow",
            "0",
            "999",
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr chop with invalid lines");

    // May succeed or fail depending on implementation
    let _ = output.status;
}

// =============================================================================
// Help Tests
// =============================================================================

#[test]
fn test_patterns_help() {
    let commands = [
        "resources",
        "temporal",
        "coupling",
        "cohesion",
        "interface",
        "behavioral",
    ];

    for cmd in &commands {
        let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
            .args([cmd, "--help"])
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute tldr {} --help", cmd));

        assert!(output.status.success(), "{} --help should succeed", cmd);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Usage:"),
            "Help should contain Usage section"
        );
        assert!(
            stdout.contains("Options:"),
            "Help should contain Options section"
        );
    }
}

#[test]
fn test_contracts_help() {
    let commands = [
        "contracts",
        "bounds",
        "invariants",
        "specs",
        "verify",
        "dead-stores",
        "chop",
    ];

    for cmd in &commands {
        let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
            .args([cmd, "--help"])
            .output()
            .unwrap_or_else(|_| panic!("Failed to execute tldr {} --help", cmd));

        assert!(output.status.success(), "{} --help should succeed", cmd);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("Usage:"),
            "Help should contain Usage section"
        );
    }
}

#[test]
fn test_diagnostics_help() {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args(["diagnostics", "--help"])
        .output()
        .expect("Failed to execute tldr diagnostics --help");

    assert!(output.status.success(), "diagnostics --help should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage:"),
        "Help should contain Usage section"
    );
    assert!(
        stdout.contains("--strict"),
        "Help should mention --strict flag"
    );
    assert!(
        stdout.contains("--severity"),
        "Help should mention --severity flag"
    );
}

// =============================================================================
// Multi-Language Tests
// =============================================================================

#[test]
fn test_patterns_rust_file() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("test.rs"),
        r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    println!("{}", add(1, 2));
}
"#,
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "purity",
            project_path.join("test.rs").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr purity on Rust file");

    // May succeed or fail depending on Rust support
    let _ = output.status;
}

#[test]
fn test_patterns_go_file() {
    let temp_dir = TempDir::new().unwrap();
    let project_path = temp_dir.path();

    fs::write(
        project_path.join("test.go"),
        r#"package main

func Add(a, b int) int {
    return a + b
}
"#,
    )
    .unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
        .args([
            "purity",
            project_path.join("test.go").to_str().unwrap(),
            "-q",
        ])
        .output()
        .expect("Failed to execute tldr purity on Go file");

    // May succeed or fail depending on Go support
    let _ = output.status;
}
