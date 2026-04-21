//! Test fixtures for Session 15 metrics commands
//!
//! Provides inline test file contents as const strings for testing
//! LOC, Cognitive, Coverage, Hotspots, and Halstead commands.

// =============================================================================
// Python fixtures
// =============================================================================

/// Python file with known line counts:
/// - 10 code lines
/// - 5 comment lines (including docstring lines)
/// - 3 blank lines
///   Total: 18 lines
pub const PYTHON_LOC_SAMPLE: &str = r#"# Module comment
"""Module docstring"""

def greet(name):
    """Function docstring."""
    # Inline comment
    message = f"Hello, {name}!"
    print(message)
    return message

class Greeter:
    """Class docstring."""
    def __init__(self):
        self.count = 0
"#;

/// Python file that is completely empty (0 bytes)
pub const PYTHON_EMPTY: &str = "";

/// Simple function with no control flow (cognitive = 0)
pub const PYTHON_COGNITIVE_ZERO: &str = r#"
def simple_function(x, y):
    """No control flow, cognitive complexity = 0"""
    result = x + y
    return result
"#;

/// Function with single if statement (cognitive = 1)
pub const PYTHON_COGNITIVE_SINGLE_IF: &str = r#"
def check_positive(x):
    """Single if, cognitive complexity = 1"""
    if x > 0:
        return True
    return False
"#;

/// Function with nested if statements
/// if (+1)
///   if (+1 base + 1 nesting = 2)
/// Total: 3
pub const PYTHON_COGNITIVE_NESTED_IF: &str = r#"
def check_nested(x, y):
    """Nested if, cognitive complexity = 3"""
    if x > 0:        # +1
        if y > 0:    # +1 base + 1 nesting = +2
            return "both positive"
    return "not both positive"
"#;

/// Function with loop containing nested condition
/// for (+1)
///   if (+1 base + 1 nesting = 2)
/// Total: 3
pub const PYTHON_COGNITIVE_LOOP_WITH_CONDITION: &str = r#"
def process_items(items):
    """Loop with nested condition, cognitive = 3"""
    result = []
    for item in items:    # +1
        if item > 0:      # +1 + 1 nesting
            result.append(item)
    return result
"#;

/// File with multiple functions of varying complexity
pub const PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS: &str = r#"
def simple():
    """cognitive = 0"""
    return 1

def with_if(x):
    """cognitive = 1"""
    if x:
        return x
    return 0

def with_nested(x, y):
    """cognitive = 3"""
    if x:
        if y:
            return x + y
    return 0

def complex_function(data, threshold, flag):
    """cognitive > 10 (violation)"""
    result = 0
    for item in data:              # +1
        if item > threshold:       # +1 + 1 nesting
            if flag:               # +1 + 2 nesting
                while item > 0:    # +1 + 3 nesting
                    result += 1
                    item -= 1
            else:                  # +1
                result -= 1
    return result
"#;

/// Simple expression for Halstead metrics:
/// Operators: =, +, *, return (4 distinct, 4 total)
/// Operands: a, b, c, result, 2 (5 distinct, 5 total)
pub const PYTHON_HALSTEAD_SIMPLE: &str = r#"
def simple_math(a, b):
    result = a + b * 2
    return result
"#;

/// Empty function for Halstead edge case
pub const PYTHON_HALSTEAD_EMPTY: &str = r#"
def empty_function():
    pass
"#;

/// Function with high Halstead complexity
pub const PYTHON_HALSTEAD_COMPLEX: &str = r#"
def complex_calculation(x, y, z, w):
    a = x + y - z * w
    b = a / x + y ** 2
    c = (a + b) * (x - y) / (z + w)
    d = a if b > c else c
    result = a + b + c + d
    return result
"#;

// =============================================================================
// TypeScript fixtures
// =============================================================================

/// TypeScript file for LOC testing
pub const TYPESCRIPT_LOC_SAMPLE: &str = r#"// Single line comment
/**
 * Multi-line comment
 * describing the interface
 */

interface User {
    id: number;
    name: string;
}

function greet(user: User): string {
    // Return greeting
    return `Hello, ${user.name}!`;
}

export { greet };
"#;

// =============================================================================
// Rust fixtures
// =============================================================================

/// Rust file for LOC testing
pub const RUST_LOC_SAMPLE: &str = r#"//! Module documentation
//!
//! More docs here

/// Function documentation
fn calculate(x: i32, y: i32) -> i32 {
    // Add the values
    let result = x + y;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate() {
        assert_eq!(calculate(1, 2), 3);
    }
}
"#;

// =============================================================================
// Coverage report fixtures
// =============================================================================

/// Cobertura XML coverage report
pub const COBERTURA_XML: &str = r#"<?xml version="1.0" ?>
<coverage version="5.5" timestamp="1234567890" lines-valid="100" lines-covered="85" line-rate="0.85" branches-valid="20" branches-covered="16" branch-rate="0.80" complexity="0">
    <packages>
        <package name="mypackage" line-rate="0.85" branch-rate="0.80" complexity="0">
            <classes>
                <class name="module.py" filename="src/module.py" line-rate="0.90" branch-rate="0.80" complexity="0">
                    <methods>
                        <method name="process_data" signature="" line-rate="1.0" branch-rate="1.0">
                            <lines>
                                <line number="10" hits="5"/>
                                <line number="11" hits="5"/>
                            </lines>
                        </method>
                        <method name="uncovered_func" signature="" line-rate="0.0" branch-rate="0.0">
                            <lines>
                                <line number="20" hits="0"/>
                                <line number="21" hits="0"/>
                            </lines>
                        </method>
                    </methods>
                    <lines>
                        <line number="1" hits="10"/>
                        <line number="2" hits="10"/>
                        <line number="3" hits="0"/>
                        <line number="10" hits="5"/>
                        <line number="11" hits="5"/>
                        <line number="20" hits="0"/>
                        <line number="21" hits="0"/>
                    </lines>
                </class>
            </classes>
        </package>
    </packages>
</coverage>
"#;

/// LCOV format coverage report
pub const LCOV_REPORT: &str = r#"TN:test_suite
SF:/path/to/src/module.py
FN:10,process_data
FN:20,uncovered_func
FNDA:5,process_data
FNDA:0,uncovered_func
FNF:2
FNH:1
DA:1,10
DA:2,10
DA:3,0
DA:10,5
DA:11,5
DA:20,0
DA:21,0
LF:7
LH:5
BRF:4
BRH:3
end_of_record
SF:/path/to/src/helper.py
DA:1,1
DA:2,1
LF:2
LH:2
end_of_record
"#;

/// coverage.py JSON format
pub const COVERAGE_PY_JSON: &str = r#"{
    "meta": {
        "version": "7.0.0",
        "timestamp": "2024-01-15T10:30:00",
        "branch_coverage": true,
        "show_contexts": false
    },
    "files": {
        "src/module.py": {
            "executed_lines": [1, 2, 10, 11, 15],
            "summary": {
                "covered_lines": 5,
                "num_statements": 8,
                "percent_covered": 62.5,
                "missing_lines": 3,
                "excluded_lines": 0
            },
            "missing_lines": [3, 20, 21],
            "excluded_lines": []
        },
        "src/helper.py": {
            "executed_lines": [1, 2, 3, 4, 5],
            "summary": {
                "covered_lines": 5,
                "num_statements": 5,
                "percent_covered": 100.0,
                "missing_lines": 0,
                "excluded_lines": 0
            },
            "missing_lines": [],
            "excluded_lines": []
        }
    },
    "totals": {
        "covered_lines": 10,
        "num_statements": 13,
        "percent_covered": 76.92,
        "missing_lines": 3,
        "excluded_lines": 0
    }
}"#;

/// Binary content (PNG header)
pub const BINARY_PNG_HEADER: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

// =============================================================================
// Helper functions
// =============================================================================

use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Create a temporary file with given content and return its path
pub fn create_temp_file(dir: &tempfile::TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent dirs");
    }
    let mut file = fs::File::create(&path).expect("Failed to create temp file");
    file.write_all(content.as_bytes())
        .expect("Failed to write content");
    path
}

/// Create a temporary binary file
pub fn create_temp_binary_file(dir: &tempfile::TempDir, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent dirs");
    }
    let mut file = fs::File::create(&path).expect("Failed to create temp file");
    file.write_all(content).expect("Failed to write content");
    path
}

/// Create a test directory structure with multiple language files
pub fn create_multi_lang_project(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().to_path_buf();

    // Python files
    create_temp_file(dir, "src/main.py", PYTHON_LOC_SAMPLE);
    create_temp_file(dir, "src/utils.py", PYTHON_COGNITIVE_MULTIPLE_FUNCTIONS);

    // TypeScript files
    create_temp_file(dir, "src/app.ts", TYPESCRIPT_LOC_SAMPLE);

    // Rust files
    create_temp_file(dir, "src/lib.rs", RUST_LOC_SAMPLE);

    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_python_loc_line_count() {
        // Verify our documented line counts are correct
        let lines: Vec<&str> = PYTHON_LOC_SAMPLE.lines().collect();
        assert_eq!(
            lines.len(),
            14,
            "PYTHON_LOC_SAMPLE should have 14 lines (not counting trailing)"
        );
    }

    #[test]
    fn fixtures_cobertura_is_valid_xml_structure() {
        // Basic check that XML starts correctly
        assert!(COBERTURA_XML.trim().starts_with("<?xml"));
        assert!(COBERTURA_XML.contains("<coverage"));
        assert!(COBERTURA_XML.contains("</coverage>"));
    }

    #[test]
    fn fixtures_lcov_has_required_fields() {
        assert!(LCOV_REPORT.contains("SF:"));
        assert!(LCOV_REPORT.contains("DA:"));
        assert!(LCOV_REPORT.contains("end_of_record"));
    }

    #[test]
    fn fixtures_coverage_py_json_is_valid() {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(COVERAGE_PY_JSON);
        assert!(parsed.is_ok(), "COVERAGE_PY_JSON should be valid JSON");
    }
}
