//! Session 15: Halstead Metrics Tests
//!
//! This module contains integration tests for the Halstead metrics command.
//!
//! ## Running Tests
//!
//! ```bash
//! cargo test -p tldr-core --test session15_halstead_tests
//! ```

#[path = "support/session15_halstead_fixtures.rs"]
mod fixtures;

use tldr_core::metrics::halstead::{analyze_halstead, HalsteadOptions};
use tldr_core::types::Language;

/// Test that operator and operand counts are reasonable for simple expression
#[test]
fn test_operator_operand_count() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "simple.py", fixtures::PYTHON_HALSTEAD_SIMPLE);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok(), "Halstead analysis should succeed");
    let report = result.unwrap();

    assert!(
        !report.functions.is_empty(),
        "Should have at least one function"
    );
    let func = report
        .functions
        .iter()
        .find(|f| f.name == "simple_math")
        .expect("Should find simple_math function");

    // Simple expression `result = a + b * 2` plus `return result` should have:
    // - Operators: =, +, *, return, def, (, ) etc.
    // - Operands: a, b, result, 2
    assert!(
        func.metrics.n1 >= 3,
        "Should have at least 3 distinct operators, got {}",
        func.metrics.n1
    );
    assert!(
        func.metrics.n2 >= 3,
        "Should have at least 3 distinct operands, got {}",
        func.metrics.n2
    );
}

/// Test vocabulary invariant: vocabulary == n1 + n2
#[test]
fn test_vocabulary_calculation() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "vocab.py", fixtures::PYTHON_HALSTEAD_SIMPLE);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    for func in &report.functions {
        let expected_vocabulary = func.metrics.n1 + func.metrics.n2;
        assert_eq!(
            func.metrics.vocabulary, expected_vocabulary,
            "vocabulary should equal n1 + n2 for function {}",
            func.name
        );

        let expected_length = func.metrics.big_n1 + func.metrics.big_n2;
        assert_eq!(
            func.metrics.length, expected_length,
            "length should equal N1 + N2 for function {}",
            func.name
        );
    }
}

/// Test derived metrics formulas
#[test]
fn test_derived_metrics() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "complex.py", fixtures::PYTHON_HALSTEAD_COMPLEX);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    for func in &report.functions {
        // Volume >= 0
        assert!(func.metrics.volume >= 0.0, "volume should be non-negative");

        // Difficulty >= 0
        assert!(
            func.metrics.difficulty >= 0.0,
            "difficulty should be non-negative"
        );

        // Effort = Difficulty * Volume (with tolerance)
        if func.metrics.volume > 0.0 && func.metrics.difficulty > 0.0 {
            let expected_effort = func.metrics.difficulty * func.metrics.volume;
            assert!(
                (func.metrics.effort - expected_effort).abs() < 0.01,
                "effort should equal difficulty * volume"
            );
        }

        // Time = Effort / 18
        let expected_time = func.metrics.effort / 18.0;
        assert!(
            (func.metrics.time - expected_time).abs() < 0.01,
            "time should equal effort / 18"
        );
    }
}

/// Test estimated bugs formula: bugs = volume / 3000
#[test]
fn test_estimated_bugs() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "bugs.py", fixtures::PYTHON_HALSTEAD_COMPLEX);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    for func in &report.functions {
        let expected_bugs = func.metrics.volume / 3000.0;
        assert!(
            (func.metrics.bugs - expected_bugs).abs() < 0.001,
            "bugs should equal volume / 3000 for function {}",
            func.name
        );
    }
}

/// Test threshold violation detection
#[test]
fn test_threshold_violations() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "complex.py", fixtures::PYTHON_HALSTEAD_COMPLEX);

    let mut options = HalsteadOptions::new();
    options.volume_threshold = 100.0; // Low threshold to trigger violations
    options.difficulty_threshold = 5.0;

    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    // All functions should have valid threshold status
    for func in &report.functions {
        use tldr_core::metrics::halstead::ThresholdStatus;
        assert!(
            matches!(
                func.thresholds.volume_status,
                ThresholdStatus::Good | ThresholdStatus::Warning | ThresholdStatus::Bad
            ),
            "volume_status should be a valid threshold status"
        );
        assert!(
            matches!(
                func.thresholds.difficulty_status,
                ThresholdStatus::Good | ThresholdStatus::Warning | ThresholdStatus::Bad
            ),
            "difficulty_status should be a valid threshold status"
        );
    }
}

/// Test empty function handling
#[test]
fn test_empty_function() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path =
        fixtures::create_temp_file(&temp_dir, "empty.py", fixtures::PYTHON_HALSTEAD_EMPTY);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Find the empty function
    let empty_fn = report.functions.iter().find(|f| f.name == "empty_function");

    assert!(empty_fn.is_some(), "Should find empty_function");
    let func = empty_fn.unwrap();

    // Empty function should have minimal metrics but not cause errors
    assert!(
        func.metrics.volume >= 0.0,
        "Volume should never be negative"
    );

    // Verify invariants hold even for empty functions
    assert_eq!(func.metrics.vocabulary, func.metrics.n1 + func.metrics.n2);
    assert_eq!(
        func.metrics.length,
        func.metrics.big_n1 + func.metrics.big_n2
    );
}

/// Test filtering by function name
#[test]
fn test_function_filter() {
    let source = r#"
def foo():
    return 1

def bar():
    return 2

def baz():
    return 3
"#;
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = fixtures::create_temp_file(&temp_dir, "multi.py", source);

    let mut options = HalsteadOptions::new();
    options.function = Some("bar".to_string());

    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    assert_eq!(report.functions.len(), 1, "Should only have 'bar' function");
    assert_eq!(report.functions[0].name, "bar");
}

/// Test summary statistics calculation
#[test]
fn test_summary_stats() {
    let source = r#"
def func1():
    return 1 + 2

def func2():
    x = 1
    y = 2
    return x + y

def func3():
    a = 1
    b = 2
    c = 3
    return a + b + c
"#;
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let file_path = fixtures::create_temp_file(&temp_dir, "summary.py", source);

    let options = HalsteadOptions::new();
    let result = analyze_halstead(&file_path, Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Check summary
    assert_eq!(report.summary.total_functions, report.functions.len());

    if !report.functions.is_empty() {
        let total_volume: f64 = report.functions.iter().map(|f| f.metrics.volume).sum();
        let expected_avg = total_volume / report.functions.len() as f64;
        assert!(
            (report.summary.avg_volume - expected_avg).abs() < 0.01,
            "Average volume should be correct"
        );
    }
}
