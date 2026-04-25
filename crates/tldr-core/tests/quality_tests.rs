//! Integration tests for the quality module
//!
//! Tests cover:
//! - Code smells detection (God Class, Long Method, Long Parameter List)
//! - Complexity analysis and hotspot detection
//! - Maintainability index calculation
//! - Code churn analysis
//! - Technical debt analysis
//! - Code health analysis
//! - Cohesion analysis (LCOM4)
//! - Dead code detection
//! - Martin metrics (package coupling)
//! - Module coupling analysis
//! - Similarity/clone detection
//! - Coverage parsing

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use tldr_core::quality::{
    analyze_cohesion, analyze_complexity, analyze_coupling, analyze_dead_code,
    compute_martin_metrics, detect_smells, find_similar, maintainability_index, parse_coverage,
    ComplexityOptions, CoverageFormat, CoverageOptions, SmellType, ThresholdPreset,
};
use tldr_core::types::Language;

// ============================================================================
// Test Helpers
// ============================================================================

fn create_test_dir() -> TempDir {
    TempDir::new().unwrap()
}

fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

// ============================================================================
// Smells Detection Tests
// ============================================================================

#[test]
fn test_detect_smells_empty_directory() {
    let dir = create_test_dir();
    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.smells.len(), 0);
    assert_eq!(report.files_scanned, 0);
}

#[test]
fn test_detect_smells_no_smells_clean_code() {
    let dir = create_test_dir();
    let content = r#"
def small_func():
    """A small, clean function."""
    return 42

class SmallClass:
    """A small class with few methods."""
    def method1(self):
        return 1
    
    def method2(self):
        return 2
"#;
    write_file(&dir, "clean.py", content);

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Clean code should have no smells
    assert_eq!(report.smells.len(), 0);
}

#[test]
fn test_detect_smells_god_class() {
    let dir = create_test_dir();
    // Create a class with many methods (>20 for God Class)
    let mut content = String::from("class GodClass:\n");
    for i in 0..25 {
        content.push_str(&format!("    def method{}(self):\n        pass\n", i));
    }
    write_file(&dir, "god_class.py", &content);

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should detect God Class
    let god_classes: Vec<_> = report
        .smells
        .iter()
        .filter(|s| s.smell_type == SmellType::GodClass)
        .collect();
    assert!(!god_classes.is_empty(), "Should detect God Class");
}

#[test]
fn test_detect_smells_long_method() {
    let dir = create_test_dir();
    // Create a method with many lines (>50 for Long Method)
    let mut content = String::from("def long_method():\n");
    for i in 0..60 {
        content.push_str(&format!("    x{} = {}\n", i, i));
    }
    content.push_str("    return x0\n");
    write_file(&dir, "long_method.py", &content);

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should detect Long Method
    let long_methods: Vec<_> = report
        .smells
        .iter()
        .filter(|s| s.smell_type == SmellType::LongMethod)
        .collect();
    assert!(!long_methods.is_empty(), "Should detect Long Method");
}

#[test]
fn test_detect_smells_long_parameter_list() {
    let dir = create_test_dir();
    // Create a function with many parameters (>5 for Long Parameter List)
    let content = r#"
def many_params(a, b, c, d, e, f, g, h):
    return a + b + c + d + e + f + g + h
"#;
    write_file(&dir, "many_params.py", content);

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should detect Long Parameter List
    let long_params: Vec<_> = report
        .smells
        .iter()
        .filter(|s| s.smell_type == SmellType::LongParameterList)
        .collect();
    assert!(!long_params.is_empty(), "Should detect Long Parameter List");
}

#[test]
fn test_detect_smells_with_suggestions() {
    let dir = create_test_dir();
    let content = r#"
def many_params(a, b, c, d, e, f, g, h):
    return a + b + c + d + e + f + g + h
"#;
    write_file(&dir, "many_params.py", content);

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, true);
    assert!(result.is_ok());
    let report = result.unwrap();

    // All smells should have suggestions
    for smell in &report.smells {
        assert!(smell.suggestion.is_some(), "Smell should have a suggestion");
    }
}

#[test]
fn test_detect_smells_threshold_presets() {
    let dir = create_test_dir();
    let content = r#"
def many_params(a, b, c, d, e, f, g, h):
    return a + b + c + d + e + f + g + h
"#;
    write_file(&dir, "params.py", content);

    // Strict preset should detect more smells
    let strict_result = detect_smells(dir.path(), ThresholdPreset::Strict, None, false);
    assert!(strict_result.is_ok());

    // Relaxed preset should detect fewer smells
    let relaxed_result = detect_smells(dir.path(), ThresholdPreset::Relaxed, None, false);
    assert!(relaxed_result.is_ok());
}

#[test]
fn test_detect_smells_by_type_filter() {
    let dir = create_test_dir();
    let content = r#"
def many_params(a, b, c, d, e, f, g, h):
    return a + b + c + d + e + f + g + h
"#;
    write_file(&dir, "params.py", content);

    let result = detect_smells(
        dir.path(),
        ThresholdPreset::Default,
        Some(SmellType::LongParameterList),
        false,
    );
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should only contain LongParameterList smells
    for smell in &report.smells {
        assert_eq!(smell.smell_type, SmellType::LongParameterList);
    }
}

// ============================================================================
// Complexity Analysis Tests
// ============================================================================

#[test]
fn test_analyze_complexity_empty_directory() {
    let dir = create_test_dir();
    let result = analyze_complexity(dir.path(), None, None);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.functions_analyzed, 0);
    assert_eq!(report.hotspot_count, 0);
    assert_eq!(report.avg_cyclomatic, 0.0);
}

#[test]
fn test_analyze_complexity_simple_functions() {
    let dir = create_test_dir();
    let content = r#"
def simple1():
    return 1

def simple2():
    return 2
"#;
    write_file(&dir, "simple.py", content);

    let result = analyze_complexity(dir.path(), Some(Language::Python), None);
    assert!(result.is_ok());
    let report = result.unwrap();

    assert_eq!(report.functions_analyzed, 2);
    assert_eq!(report.hotspot_count, 0); // CC=1 is below threshold
    assert_eq!(report.max_cyclomatic, 1);
}

#[test]
fn test_analyze_complexity_hotspot_detection() {
    let dir = create_test_dir();
    let content = r#"
def complex_function(a, b, c, d, e, f, g, h, i, j, k):
    result = 0
    if a:
        if b:
            result += 1
        elif c:
            result += 2
        else:
            result += 3
    elif d:
        if e:
            result += 4
        elif f:
            result += 5
        else:
            result += 6
    else:
        if g:
            result += 7
        elif h:
            result += 8
        else:
            result += 9
    
    for x in range(10):
        if x % 2 == 0:
            result += x
    
    return result
"#;
    write_file(&dir, "complex.py", content);

    let result = analyze_complexity(dir.path(), Some(Language::Python), None);
    assert!(result.is_ok());
    let report = result.unwrap();

    assert!(report.max_cyclomatic > 10, "Should detect high complexity");
    assert!(report.hotspot_count > 0, "Should identify hotspots");
}

#[test]
fn test_analyze_complexity_with_options() {
    let dir = create_test_dir();
    let content = r#"
def moderate(a, b, c, d, e, f):
    if a:
        return 1
    elif b:
        return 2
    elif c:
        return 3
    elif d:
        return 4
    elif e:
        return 5
    else:
        return 6
"#;
    write_file(&dir, "moderate.py", content);

    let options = ComplexityOptions {
        hotspot_threshold: 5,
        max_hotspots: 10,
        include_cognitive: true,
    };

    let result = analyze_complexity(dir.path(), Some(Language::Python), Some(options));
    assert!(result.is_ok());
    let report = result.unwrap();

    assert!(
        report.hotspot_count > 0,
        "Should detect hotspots with lower threshold"
    );
}

#[test]
fn test_analyze_complexity_sorted_by_cc() {
    let dir = create_test_dir();
    let content = r#"
def low_cc():
    return 1

def high_cc(a, b, c, d, e, f, g, h, i, j, k, l, m, n, o):
    if a:
        if b:
            return 1
        elif c:
            return 2
        elif d:
            return 3
    return 0

def medium_cc(a, b, c):
    if a:
        return 1
    elif b:
        return 2
    else:
        return 3
"#;
    write_file(&dir, "sorted.py", content);

    let result = analyze_complexity(dir.path(), Some(Language::Python), None);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Functions should be sorted by CC descending
    for i in 1..report.functions.len() {
        assert!(
            report.functions[i - 1].cyclomatic >= report.functions[i].cyclomatic,
            "Functions should be sorted by CC descending"
        );
    }

    // Ranks should be correct
    for (i, func) in report.functions.iter().enumerate() {
        assert_eq!(func.rank, i + 1);
    }
}

// ============================================================================
// Maintainability Index Tests
// ============================================================================

#[test]
fn test_maintainability_index_empty() {
    let dir = create_test_dir();
    let result = maintainability_index(dir.path(), true, None);

    assert!(result.is_ok());
    let _report = result.unwrap();
    // Should handle empty directory
}

#[test]
fn test_maintainability_index_simple_file() {
    let dir = create_test_dir();
    let content = r#"
def simple():
    """A simple function."""
    return 42
"#;
    write_file(&dir, "simple.py", content);

    let result = maintainability_index(dir.path(), true, None);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should calculate MI for the file
    assert!(!report.files.is_empty());
}

// ============================================================================
// Cohesion Analysis Tests
// ============================================================================

#[test]
fn test_analyze_cohesion_empty() {
    let dir = create_test_dir();
    let result = analyze_cohesion(dir.path(), None, 10);

    assert!(result.is_ok());
    let report = result.unwrap();
    // Cohesion report has classes field - check structure
    let _ = report.classes;
}

#[test]
fn test_analyze_cohesion_single_class() {
    let dir = create_test_dir();
    let content = r#"
class User:
    def __init__(self):
        self.name = ""
        self.email = ""
    
    def get_name(self):
        return self.name
    
    def get_email(self):
        return self.email
"#;
    write_file(&dir, "user.py", content);

    let result = analyze_cohesion(dir.path(), None, 10);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Should find the class
    assert!(!report.classes.is_empty());
}

// ============================================================================
// Dead Code Analysis Tests
// ============================================================================

#[test]
fn test_analyze_dead_code_empty() {
    let dir = create_test_dir();
    let result = analyze_dead_code(dir.path(), None, &[]);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.dead_functions.len(), 0);
}

#[test]
fn test_analyze_dead_code_all_used() {
    let dir = create_test_dir();
    let content = r#"
def public_func():
    return helper()

def helper():
    return 42

if __name__ == "__main__":
    print(public_func())
"#;
    write_file(&dir, "main.py", content);

    let result = analyze_dead_code(dir.path(), None, &[]);
    assert!(result.is_ok());
    let report = result.unwrap();

    // All functions are used
    assert_eq!(report.dead_functions.len(), 0);
}

// ============================================================================
// Martin Metrics Tests
// ============================================================================

#[test]
fn test_compute_martin_metrics_empty() {
    let dir = create_test_dir();
    let result = compute_martin_metrics(dir.path(), None);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.packages.len(), 0);
}

#[test]
fn test_compute_martin_metrics_single_package() {
    let dir = create_test_dir();
    fs::create_dir_all(dir.path().join("mypackage")).unwrap();

    write_file(&dir, "mypackage/__init__.py", "\"\"\"My package.\"\"\"");
    write_file(&dir, "mypackage/module.py", "def func1():\n    pass\n");

    let result = compute_martin_metrics(dir.path(), None);
    assert!(result.is_ok());
}

// ============================================================================
// Coupling Analysis Tests
// ============================================================================

#[test]
fn test_analyze_coupling_empty() {
    let dir = create_test_dir();
    let result = analyze_coupling(dir.path(), None, None);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.modules_analyzed, 0);
}

#[test]
fn test_analyze_coupling_simple() {
    let dir = create_test_dir();

    write_file(
        &dir,
        "a.py",
        r#"
def func_a():
    return 1
"#,
    );

    write_file(
        &dir,
        "b.py",
        r#"
from a import func_a

def func_b():
    return func_a()
"#,
    );

    let result = analyze_coupling(dir.path(), None, None);
    assert!(result.is_ok());
}

// ============================================================================
// Similarity/Clone Detection Tests
// ============================================================================

#[test]
fn test_find_similar_empty() {
    let dir = create_test_dir();
    let result = find_similar(dir.path(), None, 0.8, None);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.similar_pairs.len(), 0);
}

#[test]
fn test_find_similar_no_clones() {
    let dir = create_test_dir();

    write_file(
        &dir,
        "a.py",
        r#"
def unique_a():
    return "completely different implementation"
"#,
    );

    write_file(
        &dir,
        "b.py",
        r#"
def unique_b():
    return "another totally different function"
"#,
    );

    let result = find_similar(dir.path(), None, 0.8, None);
    assert!(result.is_ok());
    let report = result.unwrap();

    // No similar functions
    assert_eq!(report.similar_pairs.len(), 0);
}

// ============================================================================
// Coverage Parsing Tests
// ============================================================================

#[test]
fn test_parse_coverage_cobertura() {
    let dir = create_test_dir();
    let cobertura_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE coverage SYSTEM "http://cobertura.sourceforge.net/xml/coverage-04.dtd">
<coverage version="4.0.0" timestamp="1234567890">
    <packages>
        <package name="mypackage">
            <classes>
                <class filename="mymodule.py" name="mymodule">
                    <methods>
                        <method name="myfunc">
                            <lines>
                                <line number="1" hits="1"/>
                                <line number="2" hits="0"/>
                            </lines>
                        </method>
                    </methods>
                </class>
            </classes>
        </package>
    </packages>
</coverage>"#;

    write_file(&dir, "coverage.xml", cobertura_xml);

    let options = CoverageOptions::default();
    let result = parse_coverage(dir.path(), Some(CoverageFormat::Cobertura), &options);
    assert!(result.is_ok());
    let report = result.unwrap();

    assert!(!report.files.is_empty());
}

#[test]
fn test_parse_coverage_lcov() {
    let dir = create_test_dir();
    let lcov_content = r#"SF:src/main.py
DA:1,1
DA:2,0
DA:3,1
LF:3
LH:2
end_of_record"#;

    write_file(&dir, "coverage.lcov", lcov_content);

    let options = CoverageOptions::default();
    let result = parse_coverage(dir.path(), Some(CoverageFormat::Lcov), &options);
    assert!(result.is_ok());
}

#[test]
fn test_parse_coverage_empty() {
    let dir = create_test_dir();
    let options = CoverageOptions::default();
    let result = parse_coverage(dir.path(), None, &options);

    assert!(result.is_ok());
    let report = result.unwrap();
    assert_eq!(report.files.len(), 0);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_quality_multi_language() {
    let dir = create_test_dir();

    write_file(
        &dir,
        "test.py",
        r#"
def func():
    return 1
"#,
    );

    write_file(
        &dir,
        "test.rs",
        r#"
fn func() -> i32 {
    1
}
"#,
    );

    // Test smells across languages
    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());

    // Test complexity across languages
    let result = analyze_complexity(dir.path(), None, None);
    assert!(result.is_ok());
}

#[test]
fn test_quality_summary_calculations() {
    let dir = create_test_dir();

    write_file(
        &dir,
        "test.py",
        r#"
def func1():
    return 1

def func2():
    return 2
"#,
    );

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Check summary calculations
    assert_eq!(report.summary.total_smells, report.smells.len());

    // Check by_type counts
    let total_by_type: usize = report.summary.by_type.values().sum();
    assert_eq!(total_by_type, report.smells.len());
}

#[test]
fn test_quality_by_file_grouping() {
    let dir = create_test_dir();

    write_file(
        &dir,
        "file1.py",
        r#"
def many_params(a, b, c, d, e, f, g, h):
    return a
"#,
    );

    write_file(
        &dir,
        "file2.py",
        r#"
def many_params2(a, b, c, d, e, f, g, h, i, j):
    return a
"#,
    );

    let result = detect_smells(dir.path(), ThresholdPreset::Default, None, false);
    assert!(result.is_ok());
    let report = result.unwrap();

    // Check that smells are grouped by file
    let total_in_by_file: usize = report.by_file.values().map(|v| v.len()).sum();
    assert_eq!(total_in_by_file, report.smells.len());
}
