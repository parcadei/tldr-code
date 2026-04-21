//! Test for calculate_complexity API
//!
//! This API is used by the `complexity` CLI command

use std::io::Write;
use tempfile::NamedTempFile;
use tldr_core::metrics::calculate_complexity;
use tldr_core::types::Language;

#[test]
fn test_calculate_complexity_simple_function() {
    // Arrange: Create a simple Python file
    let mut temp_file = NamedTempFile::with_suffix(".py").unwrap();
    writeln!(
        temp_file,
        r#"
def simple():
    pass

def complex():
    if True:
        if True:
            if True:
                pass
"#
    )
    .unwrap();

    // Act: Call the API for simple function
    let result = calculate_complexity(
        temp_file.path().to_str().unwrap(),
        "simple",
        Language::Python,
    );

    // Assert: Should succeed
    assert!(
        result.is_ok(),
        "calculate_complexity should succeed for simple function"
    );

    let simple_metric = result.unwrap();
    println!("✅ simple() complexity: {}", simple_metric.cyclomatic);

    // Act: Call the API for complex function
    let result = calculate_complexity(
        temp_file.path().to_str().unwrap(),
        "complex",
        Language::Python,
    );

    assert!(
        result.is_ok(),
        "calculate_complexity should succeed for complex function"
    );
    let complex_metric = result.unwrap();
    println!("✅ complex() complexity: {}", complex_metric.cyclomatic);

    // Assert: Complex function should have higher complexity
    assert!(
        simple_metric.cyclomatic < complex_metric.cyclomatic,
        "Complex function should have higher complexity than simple function: {} < {}",
        simple_metric.cyclomatic,
        complex_metric.cyclomatic
    );

    println!("✅ calculate_complexity API works correctly!");
}

#[test]
fn test_calculate_complexity_not_found() {
    // Arrange: Create a Python file
    let mut temp_file = NamedTempFile::with_suffix(".py").unwrap();
    writeln!(
        temp_file,
        r#"
def existing():
    pass
"#
    )
    .unwrap();

    // Act: Call the API for non-existent function
    let result = calculate_complexity(
        temp_file.path().to_str().unwrap(),
        "nonexistent",
        Language::Python,
    );

    // Assert: Should return error
    assert!(result.is_err(), "Should error for non-existent function");
}

#[test]
fn test_calculate_complexity_rust() {
    // Arrange: Create a Rust file
    let mut temp_file = NamedTempFile::with_suffix(".rs").unwrap();
    writeln!(
        temp_file,
        r#"
fn main() {{
    println!("Hello");
}}
"#
    )
    .unwrap();

    // Act
    let result = calculate_complexity(temp_file.path().to_str().unwrap(), "main", Language::Rust);

    // Assert
    assert!(result.is_ok(), "Should work with Rust files");
    let metric = result.unwrap();
    println!("✅ Rust main() complexity: {}", metric.cyclomatic);
}
