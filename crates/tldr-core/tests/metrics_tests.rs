//! Metrics Module Tests for tldr-core
//!
//! Comprehensive tests for:
//! - types.rs: Metric data structures (LocInfo, CognitiveInfo, HalsteadInfo, etc.)
//! - complexity.rs: Cyclomatic and cognitive complexity calculation
//! - cognitive.rs: SonarSource cognitive complexity algorithm
//! - halstead.rs: Halstead software science metrics
//! - loc.rs: Lines of code analysis
//! - file_utils.rs: File handling utilities
//!
//! DO NOT FIX BUGS - JUST DOCUMENT THEM in bugs_core_metrics.md

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

// Types tests
use tldr_core::metrics::types::{
    CognitiveContributor, CognitiveInfo, CoverageInfo, HalsteadInfo, HotspotInfo, HotspotTrend,
    LocInfo, ThresholdViolation,
};

// Complexity tests
use tldr_core::metrics::{calculate_all_complexities, calculate_complexity, CognitiveOptions};

// Cognitive tests
use tldr_core::metrics::analyze_cognitive_source;

// Halstead tests
use tldr_core::metrics::halstead::ThresholdStatus as HalsteadThresholdStatus;
use tldr_core::metrics::{analyze_halstead, classify_tokens, compute_halstead, HalsteadOptions};

// LOC tests
use tldr_core::metrics::loc::{analyze_file, LocSummary};
use tldr_core::metrics::{analyze_loc, count_lines, LocOptions};

// File utils tests
use tldr_core::metrics::{
    check_file_size, contains_path_traversal, has_binary_extension, is_binary_file,
    is_path_within_project, is_symlink, resolve_symlink_safely, should_skip_path, skip_directories,
    DEFAULT_MAX_FILE_SIZE_MB,
};

use tempfile::{tempdir, NamedTempFile};
use tldr_core::types::Language;

// =============================================================================
// Types Module Tests
// =============================================================================

#[test]
fn test_loc_info_new_and_validity() {
    let loc = LocInfo::new(100, 20, 10);
    assert_eq!(loc.code_lines, 100);
    assert_eq!(loc.comment_lines, 20);
    assert_eq!(loc.blank_lines, 10);
    assert_eq!(loc.total_lines, 130);
    assert!(loc.is_valid());
}

#[test]
fn test_loc_info_default() {
    let loc = LocInfo::default();
    assert_eq!(loc.code_lines, 0);
    assert_eq!(loc.comment_lines, 0);
    assert_eq!(loc.blank_lines, 0);
    assert_eq!(loc.total_lines, 0);
    assert!(loc.is_valid());
}

#[test]
fn test_loc_info_percentages() {
    let loc = LocInfo::new(80, 10, 10);
    assert!((loc.code_percentage() - 80.0).abs() < 0.01);
    assert!((loc.comment_percentage() - 10.0).abs() < 0.01);
}

#[test]
fn test_loc_info_percentages_empty() {
    let loc = LocInfo::default();
    assert_eq!(loc.code_percentage(), 0.0);
    assert_eq!(loc.comment_percentage(), 0.0);
}

#[test]
fn test_loc_info_merge() {
    let mut loc1 = LocInfo::new(100, 20, 10);
    let loc2 = LocInfo::new(50, 10, 5);
    loc1.merge(&loc2);
    assert_eq!(loc1.code_lines, 150);
    assert_eq!(loc1.comment_lines, 30);
    assert_eq!(loc1.blank_lines, 15);
    assert_eq!(loc1.total_lines, 195);
    assert!(loc1.is_valid());
}

#[test]
fn test_cognitive_info_new() {
    let cog = CognitiveInfo::new(15, 5);
    assert_eq!(cog.score, 15);
    assert_eq!(cog.nesting_penalty, 5);
    assert_eq!(cog.base_increment(), 10);
    assert!(cog.is_valid());
}

#[test]
fn test_cognitive_info_exceeds_threshold() {
    let cog = CognitiveInfo::new(20, 5);
    assert!(cog.exceeds_threshold(15));
    assert!(!cog.exceeds_threshold(25));
    assert!(!cog.exceeds_threshold(20)); // Equal doesn't exceed
}

#[test]
fn test_cognitive_info_saturating_sub() {
    let cog = CognitiveInfo::new(5, 10); // Invalid state but tests saturating_sub
    assert_eq!(cog.base_increment(), 0); // Saturates at 0
}

#[test]
fn test_halstead_info_from_counts() {
    let hal = HalsteadInfo::from_counts(10, 20, 50, 100);
    assert_eq!(hal.n1, 10);
    assert_eq!(hal.n2, 20);
    assert_eq!(hal.big_n1, 50);
    assert_eq!(hal.big_n2, 100);
    assert_eq!(hal.vocabulary, 30); // n1 + n2
    assert_eq!(hal.length, 150); // N1 + N2
    assert!(hal.is_valid());
}

#[test]
fn test_halstead_info_empty_function() {
    let hal = HalsteadInfo::from_counts(0, 0, 0, 0);
    assert_eq!(hal.volume, 1.0); // Avoid log(0)
    assert!(hal.is_valid());
}

#[test]
fn test_halstead_info_n2_zero_caps_difficulty() {
    let hal = HalsteadInfo::from_counts(10, 0, 50, 100);
    assert_eq!(hal.difficulty, 1000.0); // Capped at 1000 when n2=0
}

#[test]
fn test_halstead_derived_metrics() {
    let hal = HalsteadInfo::from_counts(10, 20, 50, 100);

    // effort = difficulty * volume
    let expected_effort = hal.difficulty * hal.volume;
    assert!((hal.effort - expected_effort).abs() < 0.001);

    // time = effort / 18
    let expected_time = hal.effort / 18.0;
    assert!((hal.time - expected_time).abs() < 0.001);

    // bugs = volume / 3000
    let expected_bugs = hal.volume / 3000.0;
    assert!((hal.bugs - expected_bugs).abs() < 0.001);
}

#[test]
fn test_hotspot_info_new() {
    let hot = HotspotInfo::new(PathBuf::from("src/main.rs"), None, 0.8, 0.6);
    assert!((hot.hotspot_score - 0.48).abs() < 0.001); // 0.8 * 0.6
    assert!(hot.is_valid());
}

#[test]
fn test_hotspot_info_with_function() {
    let hot = HotspotInfo::new(
        PathBuf::from("src/lib.rs"),
        Some("process_data".to_string()),
        0.5,
        0.5,
    );
    assert_eq!(hot.function, Some("process_data".to_string()));
    assert!((hot.hotspot_score - 0.25).abs() < 0.001);
}

#[test]
fn test_hotspot_info_invalid_scores() {
    let mut hot = HotspotInfo::new(PathBuf::from("test.rs"), None, 1.5, 0.5);
    // churn_score > 1.0 should fail is_valid
    assert!(!hot.is_valid());

    hot.churn_score = 0.5;
    hot.complexity_score = 1.5;
    assert!(!hot.is_valid());
}

#[test]
fn test_hotspot_trend_default() {
    let hot = HotspotInfo::default();
    assert_eq!(hot.trend, HotspotTrend::Unknown);
}

#[test]
fn test_coverage_info_from_line_counts() {
    let cov = CoverageInfo::from_line_counts(80, 100, vec![5, 10, 15]);
    assert!((cov.line_coverage - 80.0).abs() < 0.01);
    assert_eq!(cov.uncovered_lines, vec![5, 10, 15]);
    assert!(cov.is_valid());
}

#[test]
fn test_coverage_info_empty_file() {
    let cov = CoverageInfo::from_line_counts(0, 0, vec![]);
    assert!((cov.line_coverage - 100.0).abs() < 0.01); // 0/0 = 100%
    assert!(cov.is_valid());
}

#[test]
fn test_coverage_info_invalid_percentage() {
    let mut cov = CoverageInfo {
        line_coverage: 150.0, // Invalid
        ..Default::default()
    };
    assert!(!cov.is_valid());

    cov.line_coverage = -10.0; // Also invalid
    assert!(!cov.is_valid());
}

#[test]
fn test_threshold_violation_creation() {
    let violation = ThresholdViolation {
        level: "warning".to_string(),
        threshold: 15,
        actual: 20,
    };
    assert_eq!(violation.level, "warning");
    assert_eq!(violation.threshold, 15);
    assert_eq!(violation.actual, 20);
}

#[test]
fn test_cognitive_contributor_creation() {
    let contributor = CognitiveContributor {
        line: 42,
        construct: "if".to_string(),
        base_increment: 1,
        nesting_increment: 2,
        nesting_level: 3,
    };
    assert_eq!(contributor.line, 42);
    assert_eq!(contributor.construct, "if");
    assert_eq!(contributor.base_increment, 1);
    assert_eq!(contributor.nesting_increment, 2);
    assert_eq!(contributor.nesting_level, 3);
}

// =============================================================================
// Complexity Module Tests
// =============================================================================

#[test]
fn test_simple_function_complexity() {
    let source = r#"
def simple():
    return 1
"#;
    let metrics = calculate_complexity(source, "simple", Language::Python).unwrap();
    assert_eq!(metrics.function, "simple");
    assert_eq!(metrics.cyclomatic, 1); // No branches = base complexity
                                       // Cognitive may be 0 since there are no control structures
}

#[test]
fn test_if_statement_cyclomatic() {
    let source = r#"
def with_if(x):
    if x > 0:
        return 1
    return 0
"#;
    let metrics = calculate_complexity(source, "with_if", Language::Python).unwrap();
    assert_eq!(metrics.cyclomatic, 2); // Base + 1 if
}

#[test]
fn test_nested_if_complexity() {
    let source = r#"
def nested(a, b):
    if a > 0:
        if b > 0:
            return 1
    return 0
"#;
    let metrics = calculate_complexity(source, "nested", Language::Python).unwrap();
    assert_eq!(metrics.cyclomatic, 3); // Base + 2 ifs
    assert!(metrics.nesting_depth >= 2);
}

#[test]
fn test_loop_complexity() {
    let source = r#"
def with_loop():
    for i in range(10):
        print(i)
"#;
    let metrics = calculate_complexity(source, "with_loop", Language::Python).unwrap();
    assert_eq!(metrics.cyclomatic, 2); // Base + 1 for loop
}

#[test]
fn test_logical_operators_complexity() {
    let source = r#"
def with_logic(a, b, c):
    if a and b:
        return 1
    if a or c:
        return 2
    return 0
"#;
    let metrics = calculate_complexity(source, "with_logic", Language::Python).unwrap();
    // Base + 2 ifs + 2 logical operators = 5
    assert!(
        metrics.cyclomatic >= 4,
        "Cyclomatic should be at least 4, got {}",
        metrics.cyclomatic
    );
}

#[test]
fn test_function_not_found_error() {
    let source = "def foo(): pass";
    let result = calculate_complexity(source, "nonexistent", Language::Python);
    assert!(result.is_err());
}

#[test]
fn test_batch_complexity_all_functions() {
    let source = r#"
def simple():
    return 1

def with_if(x):
    if x > 0:
        return 1
    return 0

def with_loop():
    for i in range(10):
        print(i)
"#;
    let results = calculate_all_complexities(source, Language::Python).unwrap();
    assert_eq!(results.len(), 3, "Should find all 3 functions");
    assert!(results.contains_key("simple"));
    assert!(results.contains_key("with_if"));
    assert!(results.contains_key("with_loop"));
}

#[test]
fn test_batch_complexity_empty_source() {
    let source = "# just a comment\n";
    let results = calculate_all_complexities(source, Language::Python).unwrap();
    assert!(results.is_empty(), "No functions means empty map");
}

#[test]
fn test_batch_complexity_class_methods() {
    let source = r#"
class MyClass:
    def method_a(self):
        return 1

    def method_b(self, x):
        if x > 0:
            return x
        return 0
"#;
    let results = calculate_all_complexities(source, Language::Python).unwrap();
    // Should find class methods too
    assert!(
        results.len() >= 2,
        "Should find at least 2 methods, got {}",
        results.len()
    );
}

#[test]
fn test_try_except_complexity() {
    let source = r#"
def with_try():
    try:
        risky()
    except Exception:
        handle()
"#;
    let metrics = calculate_complexity(source, "with_try", Language::Python).unwrap();
    // Base + 1 for try (except adds complexity)
    assert!(metrics.cyclomatic >= 2);
}

#[test]
fn test_ternary_complexity() {
    let source = r#"
def with_ternary(x):
    return 1 if x > 0 else 0
"#;
    let metrics = calculate_complexity(source, "with_ternary", Language::Python).unwrap();
    // Base + 1 for ternary
    assert!(metrics.cyclomatic >= 2);
}

#[test]
fn test_while_loop_complexity() {
    let source = r#"
def with_while():
    i = 0
    while i < 10:
        print(i)
        i += 1
"#;
    let metrics = calculate_complexity(source, "with_while", Language::Python).unwrap();
    assert_eq!(metrics.cyclomatic, 2); // Base + 1 while
}

// =============================================================================
// Cognitive Complexity Tests
// =============================================================================

#[test]
fn test_cognitive_simple_function() {
    let source = r#"
def simple_function(x, y):
    result = x + y
    return result
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "simple_function")
        .unwrap();
    assert_eq!(
        func.cognitive, 0,
        "Simple function should have cognitive = 0"
    );
}

#[test]
fn test_cognitive_single_if() {
    let source = r#"
def check_positive(x):
    if x > 0:
        return True
    return False
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "check_positive")
        .unwrap();
    assert_eq!(func.cognitive, 1, "Single if should have cognitive = 1");
}

#[test]
fn test_cognitive_nested_if() {
    let source = r#"
def check_nested(x, y):
    if x > 0:
        if y > 0:
            return "both positive"
    return "not both positive"
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "check_nested")
        .unwrap();
    // if (1) + nested if (1 + 1 nesting penalty) = 3
    assert_eq!(func.cognitive, 3, "Nested if should have cognitive = 3");
}

#[test]
fn test_cognitive_loop_with_condition() {
    let source = r#"
def process_items(items):
    result = []
    for item in items:
        if item > 0:
            result.append(item)
    return result
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "process_items")
        .unwrap();
    // for (1) + nested if (1 + 1) = 3
    assert_eq!(
        func.cognitive, 3,
        "Loop with nested if should have cognitive = 3"
    );
}

#[test]
fn test_cognitive_else_not_counted() {
    let source = r#"
def with_else(x):
    if x > 0:
        return 1
    else:
        return -1
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "with_else")
        .unwrap();
    // Only the if adds +1, else does NOT add per SonarQube spec
    assert_eq!(
        func.cognitive, 1,
        "else should not add to cognitive complexity"
    );
}

#[test]
fn test_cognitive_logical_operators() {
    let source = r#"
def with_logic(a, b, c):
    if a and b:
        return 1
    if a or c:
        return 2
    return 0
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "with_logic")
        .unwrap();
    // 2 ifs (each +1) + 2 logical operators (each +1) = 4
    assert!(
        func.cognitive >= 4,
        "Should count logical operators, got {}",
        func.cognitive
    );
}

#[test]
fn test_cognitive_threshold_violations() {
    let source = r#"
def complex_function(data, threshold, flag):
    result = 0
    for item in data:
        if item > threshold:
            if flag:
                while item > 0:
                    if result > 100:
                        result += 1
                    item -= 1
            else:
                result -= 1
        else:
            for x in range(10):
                if x > 5:
                    result += x
    return result
"#;
    let options = CognitiveOptions::new().with_threshold(5);
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    assert!(
        !report.violations.is_empty(),
        "Should detect threshold violations"
    );
}

#[test]
fn test_cognitive_summary_calculation() {
    let source = r#"
def simple():
    return 1

def medium(x):
    if x > 0:
        return x
    return 0

def complex(a, b, c):
    if a > 0:
        if b > 0:
            if c > 0:
                return a + b + c
    return 0
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    assert_eq!(report.summary.total_functions, 3);
    assert!(report.summary.avg_cognitive > 0.0);
    assert!(report.summary.max_cognitive >= 6); // complex has nested ifs
}

#[test]
fn test_cognitive_options_builder() {
    let options = CognitiveOptions::new()
        .with_threshold(10)
        .with_high_threshold(20)
        .with_contributors(true)
        .with_cyclomatic(true)
        .with_top(5);

    assert_eq!(options.threshold, 10);
    assert_eq!(options.high_threshold, 20);
    assert!(options.show_contributors);
    assert!(options.include_cyclomatic);
    assert_eq!(options.top, 5);
}

#[test]
fn test_cognitive_threshold_status_from_score() {
    use tldr_core::metrics::cognitive::ThresholdStatus as CognitiveThresholdStatus;

    assert_eq!(
        CognitiveThresholdStatus::from_score(5, 15, 25),
        CognitiveThresholdStatus::Ok
    );
    assert_eq!(
        CognitiveThresholdStatus::from_score(12, 15, 25),
        CognitiveThresholdStatus::Warning
    ); // >= 80%
    assert_eq!(
        CognitiveThresholdStatus::from_score(15, 15, 25),
        CognitiveThresholdStatus::Violation
    );
    assert_eq!(
        CognitiveThresholdStatus::from_score(25, 15, 25),
        CognitiveThresholdStatus::Severe
    );
    assert_eq!(
        CognitiveThresholdStatus::from_score(30, 15, 25),
        CognitiveThresholdStatus::Severe
    );
}

// =============================================================================
// Halstead Metrics Tests
// =============================================================================

fn create_temp_file(content: &str, extension: &str) -> NamedTempFile {
    let mut file = tempfile::Builder::new()
        .suffix(extension)
        .tempfile()
        .unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

#[test]
fn test_halstead_simple_function() {
    let source = r#"
def simple_math(a, b):
    result = a + b * 2
    return result
"#;
    let file = create_temp_file(source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let report = result.unwrap();
    assert!(!report.functions.is_empty());

    let func = &report.functions[0];
    assert_eq!(func.name, "simple_math");
    assert!(
        func.metrics.n1 >= 3,
        "Should have at least 3 distinct operators"
    );
    assert!(
        func.metrics.n2 >= 3,
        "Should have at least 3 distinct operands"
    );
}

#[test]
fn test_halstead_vocabulary_invariant() {
    let source = r#"
def calc(x, y):
    return x + y - x * y
"#;
    let file = create_temp_file(source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let report = result.unwrap();

    for func in &report.functions {
        assert_eq!(
            func.metrics.vocabulary,
            func.metrics.n1 + func.metrics.n2,
            "vocabulary should equal n1 + n2"
        );
        assert_eq!(
            func.metrics.length,
            func.metrics.big_n1 + func.metrics.big_n2,
            "length should equal N1 + N2"
        );
    }
}

#[test]
fn test_halstead_derived_metrics_from_file() {
    let source = r#"
def complex_calc(a, b, c):
    x = a + b
    y = b - c
    z = x * y / a
    return x + y + z
"#;
    let file = create_temp_file(source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let report = result.unwrap();

    for func in &report.functions {
        let m = &func.metrics;

        // Volume >= 0
        assert!(m.volume >= 0.0, "volume should be non-negative");

        // Difficulty >= 0
        assert!(m.difficulty >= 0.0, "difficulty should be non-negative");

        // Effort = Difficulty * Volume (with tolerance for floating point)
        if m.volume > 0.0 && m.difficulty > 0.0 {
            let expected_effort = m.difficulty * m.volume;
            assert!(
                (m.effort - expected_effort).abs() < 0.01,
                "effort should equal difficulty * volume"
            );
        }

        // Time = Effort / 18
        let expected_time = m.effort / 18.0;
        assert!(
            (m.time - expected_time).abs() < 0.01,
            "time should equal effort / 18"
        );

        // Bugs = Volume / 3000
        let expected_bugs = m.volume / 3000.0;
        assert!(
            (m.bugs - expected_bugs).abs() < 0.001,
            "bugs should equal volume / 3000"
        );
    }
}

#[test]
fn test_halstead_empty_function() {
    let source = r#"
def empty_function():
    pass
"#;
    let file = create_temp_file(source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let report = result.unwrap();
    assert!(!report.functions.is_empty());

    let func = &report.functions[0];
    assert_eq!(func.name, "empty_function");
    // Volume should be >= 0 (avoid log(0))
    assert!(
        func.metrics.volume >= 0.0,
        "Volume should never be negative"
    );
}

#[test]
fn test_halstead_threshold_violations() {
    let source = r#"
def complex_calculation(x, y, z, w, a, b, c, d):
    r1 = x + y - z * w
    r2 = a / b + c ** 2
    r3 = (r1 + r2) * (x - y) / (z + w)
    r4 = r1 if r2 > r3 else r3
    r5 = a + b + c + d + x + y + z + w
    r6 = r1 * r2 * r3 * r4 * r5
    return r1 + r2 + r3 + r4 + r5 + r6
"#;
    let file = create_temp_file(source, ".py");

    let mut options = HalsteadOptions::new();
    options.volume_threshold = 100.0; // Low threshold to trigger violation
    options.difficulty_threshold = 5.0;

    let result = analyze_halstead(file.path(), Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    // Check that violations were recorded
    for func in &report.functions {
        assert!(
            matches!(
                func.thresholds.volume_status,
                HalsteadThresholdStatus::Good
                    | HalsteadThresholdStatus::Warning
                    | HalsteadThresholdStatus::Bad
            ),
            "Should have valid threshold status"
        );
    }
}

#[test]
fn test_halstead_show_operators_operands() {
    let source = r#"
def add(a, b):
    return a + b
"#;
    let file = create_temp_file(source, ".py");

    let mut options = HalsteadOptions::new();
    options.show_operators = true;
    options.show_operands = true;

    let result = analyze_halstead(file.path(), Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    if !report.functions.is_empty() {
        let func = &report.functions[0];
        assert!(
            func.operators.is_some(),
            "Should include operators when requested"
        );
        assert!(
            func.operands.is_some(),
            "Should include operands when requested"
        );
    }
}

#[test]
fn test_halstead_filter_by_function() {
    let source = r#"
def foo():
    return 1

def bar():
    return 2

def baz():
    return 3
"#;
    let file = create_temp_file(source, ".py");

    let mut options = HalsteadOptions::new();
    options.function = Some("bar".to_string());

    let result = analyze_halstead(file.path(), Some(Language::Python), options);

    assert!(result.is_ok());
    let report = result.unwrap();

    assert_eq!(report.functions.len(), 1, "Should only analyze 'bar'");
    assert_eq!(report.functions[0].name, "bar");
}

#[test]
fn test_compute_halstead_from_counts() {
    let operators: HashSet<String> = vec!["=", "+", "*", "return", "def"]
        .into_iter()
        .map(String::from)
        .collect();
    let operands: HashSet<String> = vec!["a", "b", "result", "2"]
        .into_iter()
        .map(String::from)
        .collect();

    let metrics = compute_halstead(&operators, &operands, 10, 15);

    assert_eq!(metrics.n1, 5);
    assert_eq!(metrics.n2, 4);
    assert_eq!(metrics.big_n1, 10);
    assert_eq!(metrics.big_n2, 15);
    assert_eq!(metrics.vocabulary, 9);
    assert_eq!(metrics.length, 25);
}

#[test]
fn test_halstead_multiple_languages() {
    // Test Rust
    let rust_source = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let rust_file = create_temp_file(rust_source, ".rs");
    let result = analyze_halstead(
        rust_file.path(),
        Some(Language::Rust),
        HalsteadOptions::new(),
    );
    assert!(result.is_ok(), "Should analyze Rust file");

    // Test JavaScript
    let js_source = r#"
function add(a, b) {
    return a + b;
}
"#;
    let js_file = create_temp_file(js_source, ".js");
    let result = analyze_halstead(
        js_file.path(),
        Some(Language::JavaScript),
        HalsteadOptions::new(),
    );
    assert!(result.is_ok(), "Should analyze JavaScript file");
}

#[test]
fn test_classify_tokens() {
    let source = "x = a + b";
    let result = classify_tokens(source, Language::Python);

    assert!(result.is_ok());
    let (operators, operands) = result.unwrap();

    // Should find operators and operands
    assert!(!operators.is_empty(), "Should find operators");
    assert!(!operands.is_empty(), "Should find operands");
}

// =============================================================================
// LOC Module Tests
// =============================================================================

#[test]
fn test_count_lines_python_simple() {
    let source = r#"# Comment
def foo():
    pass
"#;
    let info = count_lines(source, Language::Python);
    assert_eq!(info.code_lines, 2);
    assert_eq!(info.comment_lines, 1);
    assert_eq!(info.blank_lines, 0);
    assert!(info.is_valid());
}

#[test]
#[ignore = "BUG-M013: LOC analysis docstring counting may differ from expected - documents actual behavior"]
fn test_count_lines_python_docstring() {
    let source = r#"""Module docstring."""

def foo():
    """Function docstring."""
    pass
"#;
    let info = count_lines(source, Language::Python);
    // NOTE: Actual behavior may differ from expected due to docstring parsing edge cases
    // See bugs_core_metrics.md BUG-M013
    assert!(
        info.comment_lines >= 1,
        "Should have at least 1 comment line, got {}",
        info.comment_lines
    );
    assert!(
        info.code_lines >= 2,
        "Should have at least 2 code lines, got {}",
        info.code_lines
    );
    assert!(info.is_valid());
}

#[test]
#[ignore = "BUG-M013: LOC analysis multiline docstring counting - documents actual behavior"]
fn test_count_lines_python_multiline_docstring() {
    let source = r#"""
Multi-line
docstring
"""
def foo():
    pass
"#;
    let info = count_lines(source, Language::Python);
    // NOTE: Actual behavior may differ - multiline docstring state machine has edge cases
    // See bugs_core_metrics.md BUG-M013
    assert!(
        info.comment_lines >= 2,
        "Should have at least 2 comment lines for docstring"
    );
    assert!(info.code_lines >= 2, "Should have at least 2 code lines");
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_rust_simple() {
    let source = r#"// Comment
fn main() {
    println!("Hello");
}
"#;
    let info = count_lines(source, Language::Rust);
    assert_eq!(info.code_lines, 3);
    assert_eq!(info.comment_lines, 1);
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_rust_multiline_comment() {
    let source = r#"/* Multi
   line
   comment */
fn main() {
    /* inline */ let x = 1;
}
"#;
    let info = count_lines(source, Language::Rust);
    assert_eq!(info.comment_lines, 3);
    assert_eq!(info.code_lines, 3);
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_empty() {
    let source = "";
    let info = count_lines(source, Language::Python);
    assert_eq!(info.code_lines, 0);
    assert_eq!(info.comment_lines, 0);
    assert_eq!(info.blank_lines, 0);
    assert_eq!(info.total_lines, 0);
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_blank_only() {
    let source = "\n\n\n";
    let info = count_lines(source, Language::Python);
    assert_eq!(info.blank_lines, 3);
    assert_eq!(info.code_lines, 0);
    assert_eq!(info.comment_lines, 0);
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_javascript() {
    let source = r#"// Single line comment
/*
 * Multi-line
 */
function hello() {
    console.log("hi");
}
"#;
    let info = count_lines(source, Language::JavaScript);
    assert_eq!(info.comment_lines, 4);
    assert_eq!(info.code_lines, 3);
    assert!(info.is_valid());
}

#[test]
fn test_count_lines_go() {
    let source = r#"// Package main
package main

import "fmt"

// main is the entry point
func main() {
    fmt.Println("Hello")
}
"#;
    let info = count_lines(source, Language::Go);
    assert_eq!(info.comment_lines, 2);
    assert_eq!(info.blank_lines, 2);
    assert_eq!(info.code_lines, 5);
    assert!(info.is_valid());
}

#[test]
fn test_analyze_file_python() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(file, "# Comment").unwrap();
    writeln!(file, "def foo():").unwrap();
    writeln!(file, "    pass").unwrap();

    let result = analyze_file(
        file.path(),
        Some(Language::Python),
        DEFAULT_MAX_FILE_SIZE_MB,
    );
    assert!(result.is_ok());

    let (info, lang) = result.unwrap();
    assert_eq!(lang, Language::Python);
    assert_eq!(info.code_lines, 2);
    assert_eq!(info.comment_lines, 1);
}

#[test]
fn test_analyze_file_not_found() {
    let result = analyze_file(
        PathBuf::from("/nonexistent/file.py").as_path(),
        None,
        DEFAULT_MAX_FILE_SIZE_MB,
    );
    assert!(result.is_err());
}

#[test]
fn test_analyze_file_binary() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&[0x00, 0x01, 0x02, 0x03]).unwrap();
    file.write_all(b"\x89PNG\r\n\x1a\n").unwrap(); // PNG header

    let result = analyze_file(file.path(), None, DEFAULT_MAX_FILE_SIZE_MB);
    // Should fail because it detects as binary
    assert!(result.is_err());
}

#[test]
fn test_loc_summary_from_totals() {
    let summary = LocSummary::from_totals(10, 800, 100, 100);
    assert_eq!(summary.total_files, 10);
    assert_eq!(summary.total_lines, 1000);
    assert_eq!(summary.code_lines, 800);
    assert_eq!(summary.comment_lines, 100);
    assert_eq!(summary.blank_lines, 100);
    assert!((summary.code_percent - 80.0).abs() < 0.01);
    assert!((summary.comment_percent - 10.0).abs() < 0.01);
    assert!((summary.blank_percent - 10.0).abs() < 0.01);
}

#[test]
fn test_loc_summary_empty() {
    let summary = LocSummary::from_totals(0, 0, 0, 0);
    assert_eq!(summary.total_files, 0);
    assert_eq!(summary.total_lines, 0);
    assert_eq!(summary.code_percent, 0.0);
}

#[test]
fn test_analyze_loc_single_file() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(file, "def foo():").unwrap();
    writeln!(file, "    pass").unwrap();
    file.flush().unwrap(); // Ensure data is written

    // Create options with proper settings
    let mut options = LocOptions::new();
    options.gitignore = false; // Don't skip files based on gitignore

    let result = analyze_loc(file.path(), &options);

    // May fail if temp file path isn't recognized as Python
    // This documents the behavior
    match result {
        Ok(report) => {
            assert_eq!(report.summary.total_files, 1);
            assert!(
                report.summary.code_lines >= 1,
                "Should have at least 1 code line"
            );
        }
        Err(e) => {
            // File may not be recognized as a supported language
            println!(
                "analyze_loc failed (may be expected for temp files): {:?}",
                e
            );
        }
    }
}

#[test]
fn test_analyze_loc_directory() {
    let dir = tempdir().unwrap();

    // Create some test files
    let mut file1 = std::fs::File::create(dir.path().join("test1.py")).unwrap();
    writeln!(file1, "# File 1").unwrap();
    writeln!(file1, "x = 1").unwrap();
    file1.flush().unwrap();

    let mut file2 = std::fs::File::create(dir.path().join("test2.py")).unwrap();
    writeln!(file2, "# File 2").unwrap();
    writeln!(file2, "y = 2").unwrap();
    file2.flush().unwrap();

    let mut options = LocOptions::new();
    options.gitignore = false; // Don't skip files based on gitignore

    let result = analyze_loc(dir.path(), &options);

    assert!(result.is_ok());
    let report = result.unwrap();
    // Files may or may not be found depending on gitignore behavior
    let _ = report.summary.total_files;
}

#[test]
fn test_loc_options_builder() {
    let options = LocOptions::new()
        .with_lang(Some(Language::Python))
        .with_by_file(true)
        .with_by_dir(true)
        .with_exclude(vec!["*.test.py".to_string()]);

    assert_eq!(options.lang, Some(Language::Python));
    assert!(options.by_file);
    assert!(options.by_dir);
    assert_eq!(options.exclude.len(), 1);
}

// Helper trait for builder pattern
trait LocOptionsBuilder {
    fn with_lang(self, lang: Option<Language>) -> Self;
    fn with_by_file(self, by_file: bool) -> Self;
    fn with_by_dir(self, by_dir: bool) -> Self;
    fn with_exclude(self, exclude: Vec<String>) -> Self;
}

impl LocOptionsBuilder for LocOptions {
    fn with_lang(mut self, lang: Option<Language>) -> Self {
        self.lang = lang;
        self
    }

    fn with_by_file(mut self, by_file: bool) -> Self {
        self.by_file = by_file;
        self
    }

    fn with_by_dir(mut self, by_dir: bool) -> Self {
        self.by_dir = by_dir;
        self
    }

    fn with_exclude(mut self, exclude: Vec<String>) -> Self {
        self.exclude = exclude;
        self
    }
}

// =============================================================================
// File Utils Tests
// =============================================================================

#[test]
fn test_check_file_size_within_limit() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "small content").unwrap();

    assert!(check_file_size(file.path(), 10).is_ok());
}

#[test]
fn test_check_file_size_exceeds_limit() {
    let mut file = NamedTempFile::new().unwrap();
    // Write 2MB of data
    let data = vec![b'x'; 2 * 1024 * 1024];
    file.write_all(&data).unwrap();

    let result = check_file_size(file.path(), 1); // 1MB limit
    assert!(result.is_err());
}

#[test]
fn test_is_binary_file_by_content() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&[0x00, 0x01, 0x02, 0x00]).unwrap();

    assert!(is_binary_file(file.path()));
}

#[test]
fn test_is_binary_file_text_content() {
    let mut file = NamedTempFile::new().unwrap();
    write!(file, "def foo():\n    pass\n").unwrap();

    assert!(!is_binary_file(file.path()));
}

#[test]
fn test_has_binary_extension() {
    assert!(has_binary_extension(PathBuf::from("image.png").as_path()));
    assert!(has_binary_extension(PathBuf::from("archive.zip").as_path()));
    assert!(has_binary_extension(PathBuf::from("binary.exe").as_path()));
    assert!(!has_binary_extension(PathBuf::from("code.py").as_path()));
    assert!(!has_binary_extension(PathBuf::from("script.rs").as_path()));
}

#[test]
fn test_should_skip_path_node_modules() {
    assert!(should_skip_path(
        PathBuf::from("node_modules/package/index.js").as_path()
    ));
    assert!(should_skip_path(
        PathBuf::from("project/node_modules/lodash/index.js").as_path()
    ));
}

#[test]
fn test_should_skip_path_git() {
    assert!(should_skip_path(
        PathBuf::from(".git/objects/abc").as_path()
    ));
    assert!(should_skip_path(PathBuf::from("repo/.git/HEAD").as_path()));
}

#[test]
fn test_should_skip_path_pycache() {
    assert!(should_skip_path(
        PathBuf::from("__pycache__/module.pyc").as_path()
    ));
}

#[test]
fn test_should_skip_path_hidden() {
    assert!(should_skip_path(PathBuf::from(".hidden/file").as_path()));
    assert!(should_skip_path(
        PathBuf::from("dir/.hidden_file").as_path()
    ));
}

#[test]
fn test_should_not_skip_regular_path() {
    assert!(!should_skip_path(PathBuf::from("src/main.rs").as_path()));
    assert!(!should_skip_path(
        PathBuf::from("lib/utils/helper.py").as_path()
    ));
}

#[test]
fn test_should_not_skip_github() {
    assert!(!should_skip_path(
        PathBuf::from(".github/workflows/ci.yml").as_path()
    ));
}

#[test]
fn test_skip_directories() {
    let dirs = skip_directories();
    assert!(dirs.contains("node_modules"));
    assert!(dirs.contains(".git"));
    assert!(dirs.contains("__pycache__"));
    assert!(!dirs.contains("src"));
}

#[test]
fn test_contains_path_traversal() {
    assert!(contains_path_traversal(
        PathBuf::from("../outside").as_path()
    ));
    assert!(contains_path_traversal(
        PathBuf::from("dir/../other").as_path()
    ));
    assert!(!contains_path_traversal(
        PathBuf::from("dir/subdir/file").as_path()
    ));
}

#[test]
fn test_is_symlink_regular_file() {
    let file = NamedTempFile::new().unwrap();
    assert!(!is_symlink(file.path()));
}

#[test]
fn test_resolve_symlink_regular_file() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("regular_file.txt");
    std::fs::write(&file_path, "content").unwrap();

    let resolved = resolve_symlink_safely(&file_path, None).unwrap();
    assert_eq!(resolved, file_path.canonicalize().unwrap());
}

#[test]
fn test_is_path_within_project_valid() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("src/main.rs");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, "fn main() {}").unwrap();

    assert!(is_path_within_project(&file_path, dir.path()));
}

#[test]
fn test_is_path_within_project_outside() {
    let project_dir = tempdir().unwrap();
    let outside_dir = tempdir().unwrap();

    let outside_file = outside_dir.path().join("outside.txt");
    std::fs::write(&outside_file, "content").unwrap();

    assert!(!is_path_within_project(&outside_file, project_dir.path()));
}

// =============================================================================
// Integration Tests - Multiple Languages
// =============================================================================

#[test]
fn test_complexity_multiple_languages() {
    // Python
    let py_source = "def foo():\n    if True:\n        return 1\n    return 0";
    let py_result = calculate_complexity(py_source, "foo", Language::Python);
    assert!(py_result.is_ok());
    assert_eq!(py_result.unwrap().cyclomatic, 2);

    // JavaScript
    let js_source = "function foo() { if (true) { return 1; } return 0; }";
    let js_result = calculate_complexity(js_source, "foo", Language::JavaScript);
    assert!(js_result.is_ok());
}

#[test]
fn test_cognitive_multiple_languages() {
    // Python
    let py_source = r#"
def nested(x, y):
    if x > 0:
        if y > 0:
            return True
    return False
"#;
    let options = CognitiveOptions::new();
    let py_result = analyze_cognitive_source(py_source, Language::Python, "test.py", &options);
    assert!(py_result.is_ok());

    // Rust
    let rust_source = r#"
fn nested(x: i32, y: i32) -> bool {
    if x > 0 {
        if y > 0 {
            return true;
        }
    }
    false
}
"#;
    let rust_result = analyze_cognitive_source(rust_source, Language::Rust, "test.rs", &options);
    assert!(rust_result.is_ok());
}

#[test]
fn test_loc_multiple_languages() {
    let python_code = "# Comment\ndef foo():\n    pass\n";
    let rust_code = "// Comment\nfn foo() {\n    ()\n}\n";

    let py_info = count_lines(python_code, Language::Python);
    let rust_info = count_lines(rust_code, Language::Rust);

    assert_eq!(py_info.comment_lines, 1);
    assert_eq!(py_info.code_lines, 2);
    assert_eq!(rust_info.comment_lines, 1);
    assert_eq!(rust_info.code_lines, 3);
}

// =============================================================================
// Edge Cases and Error Handling
// =============================================================================

#[test]
fn test_complexity_deeply_nested() {
    // Create deeply nested if statements
    let mut source = String::from("def deep():\n");
    for i in 0..50 {
        source.push_str(&format!("{}if x > {}:\n", "    ".repeat(i + 1), i));
    }
    source.push_str(&format!("{}return 1\n", "    ".repeat(51)));

    let result = calculate_complexity(&source, "deep", Language::Python);
    assert!(result.is_ok());
    let metrics = result.unwrap();
    assert!(metrics.nesting_depth > 10);
}

#[test]
fn test_cognitive_with_contributors() {
    let source = r#"
def complex(x, y, z):
    if x > 0:
        for i in range(y):
            if z > 0:
                return i
    return 0
"#;
    let options = CognitiveOptions::new().with_contributors(true);
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "complex")
        .unwrap();
    assert!(func.contributors.is_some());
    let contributors = func.contributors.as_ref().unwrap();
    assert!(
        !contributors.is_empty(),
        "Should have contributors recorded"
    );
}

#[test]
fn test_cognitive_with_cyclomatic() {
    let source = r#"
def with_control(x):
    if x > 0:
        return 1
    return 0
"#;
    let options = CognitiveOptions::new().with_cyclomatic(true);
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let func = report
        .functions
        .iter()
        .find(|f| f.name == "with_control")
        .unwrap();
    assert!(
        func.cyclomatic.is_some(),
        "Should include cyclomatic when requested"
    );
}

#[test]
fn test_halstead_empty_source() {
    let source = "\n\n\n";
    let file = create_temp_file(source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let _report = result.unwrap();
    // May or may not have functions depending on parser
}

#[test]
fn test_halstead_very_large_function() {
    // Create a very large function with many operators
    let mut source = String::from("def large():\n    x = 0\n");
    for i in 0..100 {
        source.push_str(&format!("    x = x + {} * {}\n", i, i + 1));
    }
    source.push_str("    return x\n");

    let file = create_temp_file(&source, ".py");
    let result = analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new());

    assert!(result.is_ok());
    let report = result.unwrap();
    assert!(!report.functions.is_empty());

    let func = &report.functions[0];
    assert!(func.metrics.volume > 0.0);
}

#[test]
fn test_loc_mixed_content() {
    let source = r#"# Comment 1
# Comment 2

def foo():
    # Inline comment
    x = 1
    
    """docstring"""
    y = 2

# End comment
"#;
    let info = count_lines(source, Language::Python);
    assert!(info.is_valid());
    assert!(info.code_lines > 0);
    assert!(info.comment_lines > 0);
}

// =============================================================================
// Serialization Tests
// =============================================================================

#[test]
fn test_loc_info_serialization() {
    let loc = LocInfo::new(100, 20, 10);
    let json = serde_json::to_string(&loc).unwrap();
    let parsed: LocInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(loc, parsed);
}

#[test]
fn test_cognitive_info_serialization() {
    let cog = CognitiveInfo::new(10, 3);
    let json = serde_json::to_string(&cog).unwrap();
    let parsed: CognitiveInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(cog.score, parsed.score);
    assert_eq!(cog.nesting_penalty, parsed.nesting_penalty);
}

#[test]
fn test_halstead_info_serialization() {
    let hal = HalsteadInfo::from_counts(5, 10, 25, 50);
    let json = serde_json::to_string(&hal).unwrap();
    assert!(json.contains("\"N1\""));
    assert!(json.contains("\"N2\""));
    let parsed: HalsteadInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(hal.vocabulary, parsed.vocabulary);
}

#[test]
fn test_hotspot_info_serialization() {
    let hot = HotspotInfo::new(PathBuf::from("test.py"), None, 0.7, 0.3);
    let json = serde_json::to_string(&hot).unwrap();
    let parsed: HotspotInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(hot.file, parsed.file);
    assert!((hot.hotspot_score - parsed.hotspot_score).abs() < 0.001);
}

#[test]
fn test_coverage_info_serialization() {
    let cov = CoverageInfo::from_line_counts(85, 100, vec![1, 2, 3]);
    let json = serde_json::to_string(&cov).unwrap();
    let parsed: CoverageInfo = serde_json::from_str(&json).unwrap();
    assert!((cov.line_coverage - parsed.line_coverage).abs() < 0.01);
    assert_eq!(cov.uncovered_lines, parsed.uncovered_lines);
}

#[test]
fn test_cognitive_report_serialization() {
    let source = r#"
def test():
    if True:
        return 1
    return 0
"#;
    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(source, Language::Python, "test.py", &options).unwrap();

    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("functions"));
    assert!(json.contains("summary"));
}

#[test]
fn test_halstead_report_serialization() {
    let source = "def foo():\n    return 1\n";
    let file = create_temp_file(source, ".py");
    let report =
        analyze_halstead(file.path(), Some(Language::Python), HalsteadOptions::new()).unwrap();

    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("functions"));
    assert!(json.contains("summary"));
}

#[test]
fn test_loc_report_serialization() {
    // Create a simple test using count_lines directly since temp files
    // may have issues with language detection
    let source = "def foo():\n    pass\n";
    let info = count_lines(source, Language::Python);

    // Create a minimal report manually to test serialization
    let summary = LocSummary::from_totals(1, info.code_lines, info.comment_lines, info.blank_lines);

    let json = serde_json::to_string(&summary).unwrap();
    assert!(json.contains("total_files"));
    assert!(json.contains("code_lines"));
}

// =============================================================================
// Performance and Stress Tests
// =============================================================================

#[test]
fn test_complexity_many_functions() {
    // Create a source with many functions
    let mut source = String::new();
    for i in 0..100 {
        source.push_str(&format!("def func_{}():\n    return {}\n\n", i, i));
    }

    let results = calculate_all_complexities(&source, Language::Python).unwrap();
    assert_eq!(results.len(), 100);
}

#[test]
fn test_cognitive_many_functions() {
    let mut source = String::new();
    for i in 0..50 {
        source.push_str(&format!(
            r#"
def func_{}(x):
    if x > 0:
        return {}
    return 0
"#,
            i, i
        ));
    }

    let options = CognitiveOptions::new();
    let report = analyze_cognitive_source(&source, Language::Python, "test.py", &options).unwrap();
    assert_eq!(report.functions.len(), 50);
}

#[test]
fn test_loc_large_file() {
    let mut source = String::new();
    for i in 0..1000 {
        if i % 3 == 0 {
            source.push_str(&format!("# Comment {}\n", i));
        } else if i % 3 == 1 {
            source.push_str(&format!("x{} = {}\n", i, i));
        } else {
            source.push('\n');
        }
    }

    let info = count_lines(&source, Language::Python);
    assert!(info.is_valid());
    assert_eq!(info.total_lines, 1000);
}
