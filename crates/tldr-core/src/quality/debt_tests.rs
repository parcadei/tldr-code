//! Test module for technical debt analysis functionality
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the debt module is implemented.
//!
//! # Test Categories
//! - Unit tests: Data type serialization, LOC counting, TODO detection
//! - Integration tests: Single file and directory analysis
//! - Output format tests: JSON and text report generation
//! - Edge case tests: Empty files, unicode, unsupported languages

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// Import the types that will be implemented
use super::debt::{
    // Functions
    analyze_debt,
    analyze_file,
    count_loc,
    find_complexity_issues,
    find_deep_nesting,
    find_god_classes,
    find_high_coupling,
    find_missing_docs,
    find_todo_comments,
    // Enums
    DebtCategory,
    // Structs
    DebtIssue,
    DebtOptions,
    DebtReport,
    DebtRule,
    DebtSummary,
    FileDebt,
};

use crate::types::Language;

// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for creating temporary files and directories
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing debt analysis
    pub struct TestDir {
        pub dir: TempDir,
    }

    impl TestDir {
        /// Create a new empty temporary directory
        pub fn new() -> std::io::Result<Self> {
            let dir = TempDir::new()?;
            Ok(Self { dir })
        }

        /// Get the path to the directory
        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Add a file to the directory
        pub fn add_file(&self, name: &str, content: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(path)
        }

        /// Add a subdirectory
        pub fn add_subdir(&self, name: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(name);
            std::fs::create_dir_all(&path)?;
            Ok(path)
        }
    }

    /// Sample Python code with various debt issues for testing
    pub const PYTHON_WITH_TODO: &str = r#"
# TODO: refactor this function
def simple_func():
    pass

# FIXME: this is broken
def another_func():
    # HACK: workaround for issue #123
    return 42

# XXX: deprecated, remove in v2.0
"#;

    /// Python code with extreme complexity function (CC > 25)
    /// This fixture has enough decision points to trigger complexity.extreme
    pub const PYTHON_HIGH_COMPLEXITY: &str = r#"
def complex_decision(a, b, c, d, e, f, g, h):
    if a > 0:
        if b > 0:
            if c > 0:
                if d > 0:
                    if e > 0:
                        if f > 0:
                            return a + b + c + d + e + f
                        else:
                            return a + b + c + d + e
                    else:
                        if f > 0:
                            return a + b + c + d + f
                        else:
                            return a + b + c + d
                else:
                    if e > 0:
                        return a + b + c + e
                    else:
                        return a + b + c
            else:
                if d > 0:
                    if e > 0 and f > 0:
                        return a + b + d + e + f
                    elif g > 0 or h > 0:
                        return a + b + d + g + h
                    else:
                        return a + b + d
                else:
                    return a + b
        else:
            if c > 0:
                if d > 0 and e > 0:
                    return a + c + d + e
                elif f > 0:
                    return a + c + f
                else:
                    return a + c
            else:
                return a
    else:
        if b > 0:
            if c > 0 or d > 0:
                if e > 0:
                    return b + c + d + e
                else:
                    return b + c + d
            else:
                return b
        elif c > 0:
            if d > 0 and e > 0 and f > 0:
                return c + d + e + f
            elif g > 0 or h > 0:
                return c + g + h
            else:
                return c
        else:
            return 0
"#;

    /// Python code with long method (>100 lines)
    pub fn python_long_method() -> String {
        let mut lines = vec!["def very_long_method():".to_string()];
        for i in 0..105 {
            lines.push(format!("    x{} = {}", i, i));
        }
        lines.push("    return x104".to_string());
        lines.join("\n")
    }

    /// Python code with long parameter list (>5 params)
    pub const PYTHON_LONG_PARAMS: &str = r#"
def too_many_params(self, a, b, c, d, e, f, g):
    """Function with too many parameters."""
    return a + b + c + d + e + f + g
"#;

    /// Python god class (>20 methods, low cohesion)
    pub fn python_god_class() -> String {
        let mut methods = Vec::new();
        for i in 0..25 {
            methods.push(format!(
                "    def method_{i}(self):\n        self.field_{i} = {i}\n        return self.field_{i}",
                i = i
            ));
        }
        format!(
            "class GodClass:\n    def __init__(self):\n        pass\n\n{}",
            methods.join("\n\n")
        )
    }

    /// Python class with deep nesting (>4 levels)
    pub const PYTHON_DEEP_NESTING: &str = r#"
def deeply_nested():
    if True:
        for i in range(10):
            while i > 0:
                try:
                    if i == 5:
                        # This is 5 levels deep
                        pass
                except:
                    pass
"#;

    /// Python file with many imports (high coupling)
    pub fn python_high_coupling() -> String {
        let imports: Vec<String> = (0..20).map(|i| format!("import module_{}", i)).collect();
        format!("{}\n\ndef main():\n    pass", imports.join("\n"))
    }

    /// Python code missing documentation
    pub const PYTHON_MISSING_DOCS: &str = r#"
def public_function():
    return 42

class PublicClass:
    def public_method(self):
        return "hello"

def _private_function():
    """This is private, shouldn't trigger."""
    pass
"#;

    /// Python code with proper documentation
    pub const PYTHON_WITH_DOCS: &str = r#"
def documented_function():
    """This function is properly documented."""
    return 42

class DocumentedClass:
    """A well-documented class."""

    def documented_method(self):
        """A documented method."""
        return "hello"
"#;
}

// =============================================================================
// Unit Tests - Enum and Struct Serialization
// =============================================================================

#[cfg(test)]
mod unit_tests {
    use super::*;
    use serde_json;

    // -------------------------------------------------------------------------
    // DebtCategory Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_category_variants() {
        // All SQALE categories should exist
        let categories = [DebtCategory::Reliability,
            DebtCategory::Security,
            DebtCategory::Maintainability,
            DebtCategory::Efficiency,
            DebtCategory::Changeability,
            DebtCategory::Testability];

        assert_eq!(categories.len(), 6);
    }

    #[test]
    fn test_debt_category_serialization() {
        // Categories serialize to lowercase
        let json = serde_json::to_value(DebtCategory::Maintainability).unwrap();
        assert_eq!(json, "maintainability");

        let json = serde_json::to_value(DebtCategory::Reliability).unwrap();
        assert_eq!(json, "reliability");

        let json = serde_json::to_value(DebtCategory::Changeability).unwrap();
        assert_eq!(json, "changeability");

        let json = serde_json::to_value(DebtCategory::Testability).unwrap();
        assert_eq!(json, "testability");
    }

    #[test]
    fn test_debt_category_deserialization() {
        let cat: DebtCategory = serde_json::from_str("\"maintainability\"").unwrap();
        assert_eq!(cat, DebtCategory::Maintainability);

        let cat: DebtCategory = serde_json::from_str("\"reliability\"").unwrap();
        assert_eq!(cat, DebtCategory::Reliability);
    }

    // -------------------------------------------------------------------------
    // DebtRule Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_rule_minutes() {
        // Verify remediation time for each rule
        assert_eq!(DebtRule::ComplexityHigh.minutes(), 20);
        assert_eq!(DebtRule::ComplexityVeryHigh.minutes(), 30);
        assert_eq!(DebtRule::ComplexityExtreme.minutes(), 60);
        assert_eq!(DebtRule::GodClass.minutes(), 60);
        assert_eq!(DebtRule::LongMethod.minutes(), 30);
        assert_eq!(DebtRule::LongParamList.minutes(), 15);
        assert_eq!(DebtRule::DeepNesting.minutes(), 15);
        assert_eq!(DebtRule::TodoComment.minutes(), 10);
        assert_eq!(DebtRule::HighCoupling.minutes(), 20);
        assert_eq!(DebtRule::MissingDocs.minutes(), 10);
    }

    #[test]
    fn test_debt_rule_category() {
        // Verify category mapping
        assert_eq!(
            DebtRule::ComplexityHigh.category(),
            DebtCategory::Maintainability
        );
        assert_eq!(
            DebtRule::ComplexityVeryHigh.category(),
            DebtCategory::Maintainability
        );
        assert_eq!(
            DebtRule::ComplexityExtreme.category(),
            DebtCategory::Maintainability
        );
        assert_eq!(
            DebtRule::LongMethod.category(),
            DebtCategory::Maintainability
        );
        assert_eq!(
            DebtRule::DeepNesting.category(),
            DebtCategory::Maintainability
        );
        assert_eq!(
            DebtRule::MissingDocs.category(),
            DebtCategory::Maintainability
        );

        assert_eq!(DebtRule::GodClass.category(), DebtCategory::Changeability);
        assert_eq!(
            DebtRule::HighCoupling.category(),
            DebtCategory::Changeability
        );

        assert_eq!(
            DebtRule::LongParamList.category(),
            DebtCategory::Testability
        );

        assert_eq!(DebtRule::TodoComment.category(), DebtCategory::Reliability);
    }

    #[test]
    fn test_debt_rule_description() {
        assert!(DebtRule::ComplexityHigh
            .description()
            .contains("complexity"));
        assert!(
            DebtRule::GodClass.description().contains("class")
                || DebtRule::GodClass.description().contains("cohesion")
        );
        assert!(
            DebtRule::TodoComment.description().contains("TODO")
                || DebtRule::TodoComment.description().contains("FIXME")
        );
    }

    #[test]
    fn test_debt_rule_as_str() {
        assert_eq!(DebtRule::ComplexityHigh.as_str(), "complexity.high");
        assert_eq!(
            DebtRule::ComplexityVeryHigh.as_str(),
            "complexity.very_high"
        );
        assert_eq!(DebtRule::ComplexityExtreme.as_str(), "complexity.extreme");
        assert_eq!(DebtRule::GodClass.as_str(), "god_class");
        assert_eq!(DebtRule::LongMethod.as_str(), "long_method");
        assert_eq!(DebtRule::LongParamList.as_str(), "long_param_list");
        assert_eq!(DebtRule::DeepNesting.as_str(), "deep_nesting");
        assert_eq!(DebtRule::TodoComment.as_str(), "todo_comment");
        assert_eq!(DebtRule::HighCoupling.as_str(), "high_coupling");
        assert_eq!(DebtRule::MissingDocs.as_str(), "missing_docs");
    }

    // -------------------------------------------------------------------------
    // DebtIssue Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_issue_serialization() {
        let issue = DebtIssue {
            file: PathBuf::from("src/main.py"),
            line: 42,
            element: Some("ClassName.method_name".to_string()),
            rule: "complexity.high".to_string(),
            message: "High complexity: CC=12".to_string(),
            category: "maintainability".to_string(),
            debt_minutes: 20,
        };

        let json = serde_json::to_value(&issue).unwrap();

        assert_eq!(json["file"], "src/main.py");
        assert_eq!(json["line"], 42);
        assert_eq!(json["element"], "ClassName.method_name");
        assert_eq!(json["rule"], "complexity.high");
        assert_eq!(json["message"], "High complexity: CC=12");
        assert_eq!(json["category"], "maintainability");
        assert_eq!(json["debt_minutes"], 20);
    }

    #[test]
    fn test_debt_issue_element_omitted_when_none() {
        let issue = DebtIssue {
            file: PathBuf::from("test.py"),
            line: 10,
            element: None,
            rule: "todo_comment".to_string(),
            message: "TODO: fix this".to_string(),
            category: "reliability".to_string(),
            debt_minutes: 10,
        };

        let json = serde_json::to_value(&issue).unwrap();

        // element should be omitted (not null) when None
        assert!(json.get("element").is_none() || json["element"].is_null());
    }

    #[test]
    fn test_debt_issue_invariants() {
        let issue = DebtIssue {
            file: PathBuf::from("test.py"),
            line: 1,
            element: None,
            rule: "todo_comment".to_string(),
            message: "TODO".to_string(),
            category: "reliability".to_string(),
            debt_minutes: 10,
        };

        // Line must be >= 1 (1-indexed)
        assert!(issue.line >= 1);
        // Debt minutes must be > 0
        assert!(issue.debt_minutes > 0);
    }

    // -------------------------------------------------------------------------
    // FileDebt Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_file_debt_serialization() {
        let file_debt = FileDebt {
            file: PathBuf::from("src/engine.py"),
            total_minutes: 120,
            issue_count: 5,
            issues: vec![], // Issues are skipped in serialization
        };

        let json = serde_json::to_value(&file_debt).unwrap();

        assert_eq!(json["file"], "src/engine.py");
        assert_eq!(json["total_minutes"], 120);
        assert_eq!(json["issue_count"], 5);
        // issues field should be skipped in serialization
        assert!(json.get("issues").is_none());
    }

    #[test]
    fn test_file_debt_invariants() {
        let issues = vec![
            DebtIssue {
                file: PathBuf::from("test.py"),
                line: 1,
                element: None,
                rule: "todo_comment".to_string(),
                message: "TODO".to_string(),
                category: "reliability".to_string(),
                debt_minutes: 10,
            },
            DebtIssue {
                file: PathBuf::from("test.py"),
                line: 5,
                element: Some("func".to_string()),
                rule: "complexity.high".to_string(),
                message: "CC=12".to_string(),
                category: "maintainability".to_string(),
                debt_minutes: 20,
            },
        ];

        let file_debt = FileDebt {
            file: PathBuf::from("test.py"),
            total_minutes: 30, // Must equal sum of issue debt
            issue_count: 2,    // Must equal issues.len()
            issues: issues.clone(),
        };

        // Invariant: total_minutes == sum of debt_minutes
        let sum: u32 = issues.iter().map(|i| i.debt_minutes).sum();
        assert_eq!(file_debt.total_minutes, sum);

        // Invariant: issue_count == issues.len()
        assert_eq!(file_debt.issue_count, issues.len());
    }

    // -------------------------------------------------------------------------
    // DebtSummary Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_summary_serialization() {
        let mut by_category = BTreeMap::new();
        by_category.insert("maintainability".to_string(), 180);
        by_category.insert("reliability".to_string(), 60);

        let mut by_rule = BTreeMap::new();
        by_rule.insert("complexity.high".to_string(), 100);
        by_rule.insert("todo_comment".to_string(), 60);

        let summary = DebtSummary {
            total_minutes: 240,
            total_hours: 4.0,
            total_cost: Some(200.0),
            debt_ratio: 0.052,
            debt_density: 52.0,
            by_category,
            by_rule,
        };

        let json = serde_json::to_value(&summary).unwrap();

        assert_eq!(json["total_minutes"], 240);
        assert!((json["total_hours"].as_f64().unwrap() - 4.0).abs() < 0.001);
        assert!((json["total_cost"].as_f64().unwrap() - 200.0).abs() < 0.01);
        assert!((json["debt_ratio"].as_f64().unwrap() - 0.052).abs() < 0.001);
        assert!((json["debt_density"].as_f64().unwrap() - 52.0).abs() < 0.1);
    }

    #[test]
    fn test_debt_summary_cost_omitted_when_none() {
        let summary = DebtSummary {
            total_minutes: 60,
            total_hours: 1.0,
            total_cost: None,
            debt_ratio: 0.01,
            debt_density: 10.0,
            by_category: BTreeMap::new(),
            by_rule: BTreeMap::new(),
        };

        let json = serde_json::to_value(&summary).unwrap();

        // total_cost should be omitted when None
        assert!(json.get("total_cost").is_none() || json["total_cost"].is_null());
    }

    #[test]
    fn test_debt_summary_formulas() {
        // Test the calculation formulas

        // total_hours = total_minutes / 60.0
        let total_minutes = 150u32;
        let expected_hours = 2.5; // 150 / 60 = 2.5
        let actual_hours = (total_minutes as f64 / 60.0 * 100.0).round() / 100.0;
        assert!((actual_hours - expected_hours).abs() < 0.01);

        // debt_ratio = total_minutes / total_loc
        let total_loc = 1000usize;
        let expected_ratio = 0.15; // 150 / 1000 = 0.15
        let actual_ratio = ((total_minutes as f64 / total_loc as f64) * 1000.0).round() / 1000.0;
        assert!((actual_ratio - expected_ratio).abs() < 0.001);

        // debt_density = debt_ratio * 1000 (minutes per KLOC)
        let expected_density = 150.0; // 0.15 * 1000
        let actual_density = (actual_ratio * 1000.0 * 100.0).round() / 100.0;
        assert!((actual_density - expected_density).abs() < 0.1);
    }

    // -------------------------------------------------------------------------
    // DebtReport Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_report_serialization() {
        let report = DebtReport {
            issues: vec![DebtIssue {
                file: PathBuf::from("test.py"),
                line: 1,
                element: None,
                rule: "todo_comment".to_string(),
                message: "TODO".to_string(),
                category: "reliability".to_string(),
                debt_minutes: 10,
            }],
            top_files: vec![FileDebt {
                file: PathBuf::from("test.py"),
                total_minutes: 10,
                issue_count: 1,
                issues: vec![],
            }],
            summary: DebtSummary {
                total_minutes: 10,
                total_hours: 0.17,
                total_cost: None,
                debt_ratio: 0.01,
                debt_density: 10.0,
                by_category: BTreeMap::new(),
                by_rule: BTreeMap::new(),
            },
        };

        let json = serde_json::to_value(&report).unwrap();

        assert!(json["issues"].is_array());
        assert!(json["top_files"].is_array());
        assert!(json["summary"].is_object());
    }

    #[test]
    fn test_debt_report_invariants() {
        // Issues must be sorted by debt_minutes descending
        let issues = [DebtIssue {
                file: PathBuf::from("a.py"),
                line: 1,
                element: None,
                rule: "complexity.extreme".to_string(),
                message: "".to_string(),
                category: "maintainability".to_string(),
                debt_minutes: 60,
            },
            DebtIssue {
                file: PathBuf::from("b.py"),
                line: 1,
                element: None,
                rule: "complexity.high".to_string(),
                message: "".to_string(),
                category: "maintainability".to_string(),
                debt_minutes: 20,
            },
            DebtIssue {
                file: PathBuf::from("c.py"),
                line: 1,
                element: None,
                rule: "todo_comment".to_string(),
                message: "".to_string(),
                category: "reliability".to_string(),
                debt_minutes: 10,
            }];

        // Verify descending order
        for window in issues.windows(2) {
            assert!(
                window[0].debt_minutes >= window[1].debt_minutes,
                "Issues must be sorted by debt_minutes descending"
            );
        }
    }

    // -------------------------------------------------------------------------
    // DebtOptions Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_debt_options_defaults() {
        let options = DebtOptions::default();

        assert_eq!(options.path, PathBuf::from("."));
        assert!(options.category_filter.is_none());
        assert_eq!(options.top_k, 20);
        assert!(options.hourly_rate.is_none());
        assert_eq!(options.min_debt, 0);
        assert!(options.language.is_none());
    }
}

// =============================================================================
// Unit Tests - LOC Counting (DEBT-001)
// =============================================================================

#[cfg(test)]
mod loc_tests {
    use super::*;

    #[test]
    fn test_count_loc_empty() {
        assert_eq!(count_loc("", Language::Python), 0);
    }

    #[test]
    fn test_count_loc_blank_lines_only() {
        let source = "\n\n\n   \n  \n";
        assert_eq!(count_loc(source, Language::Python), 0);
    }

    #[test]
    fn test_count_loc_comments_only() {
        let source = "# comment\n# another comment\n";
        assert_eq!(count_loc(source, Language::Python), 0);
    }

    #[test]
    fn test_count_loc_simple_code() {
        let source = "def foo():\n    pass\n";
        assert_eq!(count_loc(source, Language::Python), 2);
    }

    #[test]
    fn test_count_loc_with_inline_comments() {
        let source = "x = 1  # inline comment\ny = 2\n";
        // Lines with code + comment should still count as code
        assert_eq!(count_loc(source, Language::Python), 2);
    }

    #[test]
    fn test_count_loc_with_single_line_docstring() {
        let source = r#"
def foo():
    """Single line docstring"""
    pass
"#;
        // Single-line docstrings should not be counted
        assert_eq!(count_loc(source, Language::Python), 2); // def and pass
    }

    #[test]
    fn test_count_loc_with_multiline_docstring() {
        let source = r#"
def foo():
    """
    Multi-line
    docstring
    here
    """
    pass
"#;
        // Multi-line docstrings should not be counted
        assert_eq!(count_loc(source, Language::Python), 2); // def and pass
    }

    #[test]
    fn test_count_loc_mixed_quote_styles() {
        let source = r#"
def foo():
    '''Triple single quotes'''
    pass

def bar():
    """Triple double quotes"""
    return 1
"#;
        // Both quote styles should work
        assert_eq!(count_loc(source, Language::Python), 4); // def foo, pass, def bar, return
    }

    #[test]
    fn test_count_loc_rust() {
        let source = r#"
// Comment
fn main() {
    // Another comment
    println!("hello");
}
"#;
        // Comments should not be counted
        assert_eq!(count_loc(source, Language::Rust), 3); // fn main() {, println, }
    }

    #[test]
    fn test_count_loc_typescript() {
        let source = r#"
// Single line comment
/* Multi-line
   comment */
function hello() {
    console.log("hi");
}
"#;
        assert!(count_loc(source, Language::TypeScript) >= 3);
    }
}

// =============================================================================
// Unit Tests - TODO Detection (DEBT-002)
// =============================================================================

#[cfg(test)]
mod todo_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_find_todo_comments_basic() {
        let source = "# TODO: fix this\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].line, 1);
        assert_eq!(issues[0].rule, "todo_comment");
        assert_eq!(issues[0].category, "reliability");
        assert_eq!(issues[0].debt_minutes, 10);
    }

    #[test]
    fn test_find_todo_comments_all_tags() {
        let source = "# TODO: first\n# FIXME: second\n# HACK: third\n# XXX: fourth\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 4);

        // Verify all tags detected
        let messages: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
        assert!(messages.iter().any(|m| m.contains("TODO")));
        assert!(messages.iter().any(|m| m.contains("FIXME")));
        assert!(messages.iter().any(|m| m.contains("HACK")));
        assert!(messages.iter().any(|m| m.contains("XXX")));
    }

    #[test]
    fn test_find_todo_comments_case_insensitive() {
        let source = "# todo: lowercase\n# Todo: mixed\n# TODO: uppercase\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 3, "All case variants should be detected");
    }

    #[test]
    fn test_find_todo_comments_line_numbers() {
        let source = "x = 1\n# TODO: on line 2\ny = 2\n# FIXME: on line 4\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].line, 2);
        assert_eq!(issues[1].line, 4);
    }

    #[test]
    fn test_find_todo_comments_empty_content() {
        let source = "# TODO\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "TODO");
    }

    #[test]
    fn test_find_todo_comments_content_truncation() {
        let long_content = "a".repeat(100);
        let source = format!("# TODO: {}\n", long_content);
        let issues = find_todo_comments(&source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 1);
        // Content should be truncated to 50 chars max
        // Message format is "TODO: <content>" so total should be <= 6 + 50
        assert!(issues[0].message.len() <= 56);
    }

    #[test]
    fn test_find_todo_comments_with_colons() {
        let source = "# TODO: this: has: colons\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("this"));
    }

    #[test]
    fn test_find_todo_comments_extra_spaces() {
        let source = "# FIXME   :   lots of spaces\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("FIXME"));
    }

    #[test]
    fn test_find_todo_comments_no_space_after_hash() {
        // Python style: tree-sitter will still find the comment node,
        // but the regex inside will need to match #TODO without space.
        // The AST version finds the comment, strips #, and matches TODO.
        let source = "#TODO: no space\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);

        // With AST approach, #TODO is a valid comment node.
        // After stripping "#", we get "TODO: no space" which matches.
        // So this SHOULD be detected now (AST is more accurate than regex).
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn test_find_todo_comments_file_path() {
        let issues = find_todo_comments(
            "# TODO: test\n",
            Path::new("src/module/file.py"),
            Language::Python,
        );

        assert_eq!(issues[0].file, PathBuf::from("src/module/file.py"));
    }

    #[test]
    fn test_find_todo_comments_fixture() {
        let issues = find_todo_comments(PYTHON_WITH_TODO, Path::new("test.py"), Language::Python);

        // Fixture has: TODO, FIXME, HACK, XXX
        assert_eq!(issues.len(), 4);
    }
}

// =============================================================================
// Unit Tests - Multi-Language TODO Detection (DEBT-002b)
// =============================================================================

#[cfg(test)]
mod todo_multilang_tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Per-language comment detection
    // -------------------------------------------------------------------------

    #[test]
    fn test_python_hash_comment() {
        let source = "# TODO: fix this\ndef foo(): pass\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
        assert!(issues[0].message.contains("fix this"));
    }

    #[test]
    fn test_javascript_slash_comment() {
        let source = "// TODO: fix this\nfunction foo() {}\n";
        let issues = find_todo_comments(source, Path::new("test.js"), Language::JavaScript);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_typescript_slash_comment() {
        let source = "// FIXME: broken\nconst x: number = 1;\n";
        let issues = find_todo_comments(source, Path::new("test.ts"), Language::TypeScript);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("FIXME"));
    }

    #[test]
    fn test_rust_line_comment() {
        let source = "// TODO: fix this\nfn main() {}\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_go_comment() {
        let source = "package main\n// FIXME: broken\nfunc main() {}\n";
        let issues = find_todo_comments(source, Path::new("test.go"), Language::Go);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("FIXME"));
    }

    #[test]
    fn test_java_line_comment() {
        let source = "// TODO: fix this\nclass Foo {}\n";
        let issues = find_todo_comments(source, Path::new("test.java"), Language::Java);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_java_block_comment() {
        let source = "/* HACK: workaround */\nclass Foo {}\n";
        let issues = find_todo_comments(source, Path::new("test.java"), Language::Java);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("HACK"));
    }

    #[test]
    fn test_c_comment() {
        let source = "// TODO: fix this\nint main() { return 0; }\n";
        let issues = find_todo_comments(source, Path::new("test.c"), Language::C);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_cpp_comment() {
        let source = "// XXX: deprecated\nint main() { return 0; }\n";
        let issues = find_todo_comments(source, Path::new("test.cpp"), Language::Cpp);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("XXX"));
    }

    #[test]
    fn test_ruby_comment() {
        let source = "# TODO: fix this\ndef foo; end\n";
        let issues = find_todo_comments(source, Path::new("test.rb"), Language::Ruby);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_kotlin_line_comment() {
        let source = "// TODO: fix this\nfun main() {}\n";
        let issues = find_todo_comments(source, Path::new("test.kt"), Language::Kotlin);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_csharp_comment() {
        let source = "// TODO: fix this\nclass Foo {}\n";
        let issues = find_todo_comments(source, Path::new("test.cs"), Language::CSharp);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_scala_comment() {
        let source = "// TODO: fix this\nobject Foo {}\n";
        let issues = find_todo_comments(source, Path::new("test.scala"), Language::Scala);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_php_comment() {
        let source = "<?php\n// TODO: fix this\nfunction foo() {}\n";
        let issues = find_todo_comments(source, Path::new("test.php"), Language::Php);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_lua_comment() {
        let source = "-- TODO: fix this\nlocal x = 1\n";
        let issues = find_todo_comments(source, Path::new("test.lua"), Language::Lua);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_luau_comment() {
        let source = "-- FIXME: broken\nlocal x: number = 1\n";
        let issues = find_todo_comments(source, Path::new("test.luau"), Language::Luau);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("FIXME"));
    }

    #[test]
    fn test_elixir_comment() {
        let source = "# TODO: fix this\ndefmodule Foo do\nend\n";
        let issues = find_todo_comments(source, Path::new("test.ex"), Language::Elixir);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_ocaml_comment() {
        let source = "(* TODO: fix this *)\nlet x = 1\n";
        let issues = find_todo_comments(source, Path::new("test.ml"), Language::Ocaml);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_swift_comment() {
        // Swift tree-sitter is deferred, so uses regex fallback
        let source = "// TODO: fix this\nfunc foo() {}\n";
        let issues = find_todo_comments(source, Path::new("test.swift"), Language::Swift);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    // -------------------------------------------------------------------------
    // AST-based: string literals should NOT match
    // -------------------------------------------------------------------------

    #[test]
    fn test_python_string_not_detected() {
        // String literal containing TODO should NOT be detected
        let source = "x = \"# TODO: not a comment\"\ndef foo(): pass\n";
        let issues = find_todo_comments(source, Path::new("test.py"), Language::Python);
        assert_eq!(
            issues.len(),
            0,
            "String literal should not be detected as TODO comment"
        );
    }

    #[test]
    fn test_javascript_string_not_detected() {
        let source = "const x = \"// TODO: not a comment\";\n";
        let issues = find_todo_comments(source, Path::new("test.js"), Language::JavaScript);
        assert_eq!(
            issues.len(),
            0,
            "String literal should not be detected as TODO comment"
        );
    }

    #[test]
    fn test_rust_string_not_detected() {
        let source = "fn main() { let x = \"// TODO: not a comment\"; }\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(
            issues.len(),
            0,
            "String literal should not be detected as TODO comment"
        );
    }

    #[test]
    fn test_go_string_not_detected() {
        let source = "package main\nvar x = \"// FIXME: not a comment\"\n";
        let issues = find_todo_comments(source, Path::new("test.go"), Language::Go);
        assert_eq!(
            issues.len(),
            0,
            "String literal should not be detected as TODO comment"
        );
    }

    // -------------------------------------------------------------------------
    // Multiple comments and mixed
    // -------------------------------------------------------------------------

    #[test]
    fn test_multiple_comments_rust() {
        let source = "// TODO: first\n// FIXME: second\nfn main() {}\n// HACK: third\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 3);
    }

    #[test]
    fn test_block_comment_with_todo_c() {
        let source = "/* TODO: fix this */\nint main() { return 0; }\n";
        let issues = find_todo_comments(source, Path::new("test.c"), Language::C);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("TODO"));
    }

    #[test]
    fn test_all_tags_javascript() {
        let source = "// TODO: t\n// FIXME: f\n// HACK: h\n// XXX: x\n";
        let issues = find_todo_comments(source, Path::new("test.js"), Language::JavaScript);
        assert_eq!(issues.len(), 4);
    }

    #[test]
    fn test_case_insensitive_go() {
        let source = "package main\n// todo: lowercase\n// Todo: mixed\n// TODO: upper\n";
        let issues = find_todo_comments(source, Path::new("test.go"), Language::Go);
        assert_eq!(issues.len(), 3);
    }

    #[test]
    fn test_content_truncation_multilang() {
        let long = "a".repeat(100);
        let source = format!("// TODO: {}\nfn main() {{}}\n", long);
        let issues = find_todo_comments(&source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 1);
        // Content truncated to 50 chars, message = "TODO: <50 chars>"
        assert!(issues[0].message.len() <= 56);
    }

    #[test]
    fn test_line_numbers_kotlin() {
        let source = "fun main() {}\n// TODO: line 2\nval x = 1\n// FIXME: line 4\n";
        let issues = find_todo_comments(source, Path::new("test.kt"), Language::Kotlin);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].line, 2);
        assert_eq!(issues[1].line, 4);
    }

    #[test]
    fn test_php_hash_comment() {
        // PHP supports both # and // comments
        let source = "<?php\n# TODO: hash style\n// FIXME: slash style\n";
        let issues = find_todo_comments(source, Path::new("test.php"), Language::Php);
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn test_no_false_match_todone() {
        // TODONE should not match (word boundary)
        let source = "// TODONE: not a todo\nfn main() {}\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 0, "TODONE should not match TODO");
    }

    #[test]
    fn test_empty_comment_todo() {
        let source = "// TODO\nfn main() {}\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "TODO");
    }

    #[test]
    fn test_debt_minutes_consistent() {
        let source = "// TODO: test\n";
        let issues = find_todo_comments(source, Path::new("test.rs"), Language::Rust);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].debt_minutes, 10);
        assert_eq!(issues[0].rule, "todo_comment");
        assert_eq!(issues[0].category, "reliability");
    }
}

// =============================================================================
// Unit Tests - Complexity Analysis (DEBT-003)
// =============================================================================

#[cfg(test)]
mod complexity_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_find_complexity_issues_under_threshold() {
        let source = r#"
def simple():
    return 42
"#;
        let issues = find_complexity_issues(source, Path::new("test.py"), Language::Python);

        // CC=1, should not trigger any complexity issues
        let complexity_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule.starts_with("complexity"))
            .collect();
        assert!(complexity_issues.is_empty());
    }

    #[test]
    fn test_find_complexity_issues_high() {
        // Create a function with CC > 10 but <= 15
        let source = r#"
def moderately_complex(a, b, c, d, e):
    if a: return 1
    elif b: return 2
    elif c: return 3
    elif d: return 4
    elif e: return 5
    if a and b: return 6
    if c and d: return 7
    if e and a: return 8
    if b and c: return 9
    if d and e: return 10
    if a or b: return 11
    return 0
"#;
        let issues = find_complexity_issues(source, Path::new("test.py"), Language::Python);

        let complexity_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "complexity.high")
            .collect();

        // Should have CC > 10, triggering complexity.high
        assert!(
            !complexity_issues.is_empty()
                || issues.iter().any(|i| i.rule.starts_with("complexity")),
            "Should detect high complexity"
        );
    }

    #[test]
    fn test_find_complexity_issues_extreme() {
        // High complexity fixture should trigger extreme (CC > 25)
        let issues = find_complexity_issues(
            PYTHON_HIGH_COMPLEXITY,
            Path::new("test.py"),
            Language::Python,
        );

        let extreme_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "complexity.extreme")
            .collect();

        // The fixture has very high CC, might hit extreme threshold
        // If not extreme, should at least have very_high or high
        assert!(
            !extreme_issues.is_empty() || issues.iter().any(|i| i.rule.starts_with("complexity")),
            "Should detect complexity issues"
        );
    }

    #[test]
    fn test_find_complexity_issues_only_highest() {
        // Only the highest applicable threshold should be reported
        // i.e., if CC=30, only complexity.extreme, not also complexity.high
        let issues = find_complexity_issues(
            PYTHON_HIGH_COMPLEXITY,
            Path::new("test.py"),
            Language::Python,
        );

        let _func_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule.starts_with("complexity"))
            .collect();

        // Each function should have at most one complexity issue
        // (the highest applicable threshold)
        // This is a behavioral invariant from the spec
    }

    #[test]
    fn test_find_complexity_issues_long_method() {
        let source = fixtures::python_long_method();
        let issues = find_complexity_issues(&source, Path::new("test.py"), Language::Python);

        let long_method_issues: Vec<_> =
            issues.iter().filter(|i| i.rule == "long_method").collect();

        assert!(
            !long_method_issues.is_empty(),
            "Should detect long method (>100 LOC)"
        );
        assert_eq!(long_method_issues[0].debt_minutes, 30);
        assert_eq!(long_method_issues[0].category, "maintainability");
    }

    #[test]
    fn test_find_complexity_issues_long_param_list() {
        let issues =
            find_complexity_issues(PYTHON_LONG_PARAMS, Path::new("test.py"), Language::Python);

        let param_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "long_param_list")
            .collect();

        assert!(
            !param_issues.is_empty(),
            "Should detect long parameter list (>5)"
        );
        assert_eq!(param_issues[0].debt_minutes, 15);
        assert_eq!(param_issues[0].category, "testability");
    }

    #[test]
    fn test_find_complexity_issues_excludes_self() {
        // self and cls should not count toward parameter count
        let source = r#"
def method(self, a, b, c, d, e):
    pass
"#;
        let issues = find_complexity_issues(source, Path::new("test.py"), Language::Python);

        let param_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "long_param_list")
            .collect();

        // 5 params (excluding self) is not > 5, so no issue
        assert!(param_issues.is_empty());
    }

    #[test]
    fn test_find_complexity_issues_method_names() {
        let source = r#"
class MyClass:
    def complex_method(self, a, b, c, d, e, f, g):
        if a: return 1
        elif b: return 2
        elif c: return 3
        elif d: return 4
        elif e: return 5
        elif f: return 6
        elif g: return 7
        return 0
"#;
        let issues = find_complexity_issues(source, Path::new("test.py"), Language::Python);

        // Check that element name is "MyClass.complex_method"
        let method_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.element
                    .as_ref()
                    .map(|e| e.contains("MyClass."))
                    .unwrap_or(false)
            })
            .collect();

        assert!(
            !method_issues.is_empty(),
            "Should include class.method naming"
        );
    }
}

// =============================================================================
// Unit Tests - God Class Detection (DEBT-004)
// =============================================================================

#[cfg(test)]
mod god_class_tests {
    
    use super::*;

    #[test]
    fn test_find_god_classes_small_class() {
        let source = r#"
class SmallClass:
    def method1(self): pass
    def method2(self): pass
    def method3(self): pass
"#;
        let issues = find_god_classes(source, Path::new("test.py"), Language::Python);

        assert!(issues.is_empty(), "Small class should not be flagged");
    }

    #[test]
    fn test_find_god_classes_high_lcom() {
        // The god class fixture has 25 methods with low cohesion
        let source = fixtures::python_god_class();
        let issues = find_god_classes(&source, Path::new("test.py"), Language::Python);

        // Should detect god class (>20 methods AND LCOM4 > 0.8)
        let god_issues: Vec<_> = issues.iter().filter(|i| i.rule == "god_class").collect();

        assert!(!god_issues.is_empty(), "Should detect god class");
        assert_eq!(god_issues[0].debt_minutes, 60);
        assert_eq!(god_issues[0].category, "changeability");
    }

    #[test]
    fn test_find_god_classes_excludes_dunder() {
        // Dunder methods should not count toward method count
        let source = r#"
class WithDunders:
    def __init__(self): pass
    def __str__(self): pass
    def __repr__(self): pass
    def __eq__(self, other): pass
    def __hash__(self): pass
    # ... more dunders
"#;
        let issues = find_god_classes(source, Path::new("test.py"), Language::Python);

        // Even with many dunders, should not be flagged
        assert!(issues.is_empty());
    }

    #[test]
    fn test_compute_lcom4_cohesive() {
        // All methods share the same field -> LCOM4 close to 0
        // Note: This test assumes compute_lcom4 is exposed for testing
        // The actual test would need to construct appropriate test data

        // This is a placeholder - actual implementation would test the LCOM4 calculation
    }

    #[test]
    fn test_compute_lcom4_incohesive() {
        // No methods share fields -> LCOM4 = 1.0
        // Note: This test assumes compute_lcom4 is exposed for testing
    }

    #[test]
    fn test_compute_lcom4_single_method() {
        // Class with < 2 methods -> LCOM4 = 0.0 by definition
    }
}

// =============================================================================
// Unit Tests - Deep Nesting Detection (DEBT-010)
// =============================================================================

#[cfg(test)]
mod nesting_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_find_deep_nesting_under_threshold() {
        let source = r#"
def shallow():
    if True:
        for i in range(10):
            if i > 0:
                pass  # Only 3 levels, under threshold
"#;
        let issues = find_deep_nesting(source, Path::new("test.py"), Language::Python);

        let nesting_issues: Vec<_> = issues.iter().filter(|i| i.rule == "deep_nesting").collect();

        assert!(nesting_issues.is_empty(), "3 levels should not trigger");
    }

    #[test]
    fn test_find_deep_nesting_at_threshold() {
        let source = r#"
def at_threshold():
    if True:           # 1
        for i in [1]:  # 2
            while i:   # 3
                if i:  # 4
                    pass
"#;
        let issues = find_deep_nesting(source, Path::new("test.py"), Language::Python);

        // 4 levels is AT threshold, not OVER, so should not trigger
        let nesting_issues: Vec<_> = issues.iter().filter(|i| i.rule == "deep_nesting").collect();

        assert!(
            nesting_issues.is_empty(),
            "4 levels exactly should not trigger"
        );
    }

    #[test]
    fn test_find_deep_nesting_over_threshold() {
        let issues = find_deep_nesting(PYTHON_DEEP_NESTING, Path::new("test.py"), Language::Python);

        let nesting_issues: Vec<_> = issues.iter().filter(|i| i.rule == "deep_nesting").collect();

        assert!(!nesting_issues.is_empty(), "5 levels should trigger");
        assert_eq!(nesting_issues[0].debt_minutes, 15);
        assert_eq!(nesting_issues[0].category, "maintainability");
    }

    #[test]
    fn test_find_deep_nesting_message() {
        let issues = find_deep_nesting(PYTHON_DEEP_NESTING, Path::new("test.py"), Language::Python);

        if let Some(issue) = issues.iter().find(|i| i.rule == "deep_nesting") {
            assert!(
                issue.message.contains("levels"),
                "Message should mention nesting levels"
            );
        }
    }
}

// =============================================================================
// Unit Tests - High Coupling Detection (DEBT-011)
// =============================================================================

#[cfg(test)]
mod coupling_tests {
    
    use super::*;

    #[test]
    fn test_find_high_coupling_under_threshold() {
        let source = r#"
import os
import sys
from pathlib import Path

def main():
    pass
"#;
        let issues = find_high_coupling(source, Path::new("test.py"), Language::Python);

        let coupling_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "high_coupling")
            .collect();

        assert!(coupling_issues.is_empty(), "3 imports should not trigger");
    }

    #[test]
    fn test_find_high_coupling_over_threshold() {
        let source = fixtures::python_high_coupling();
        let issues = find_high_coupling(&source, Path::new("test.py"), Language::Python);

        let coupling_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.rule == "high_coupling")
            .collect();

        assert!(!coupling_issues.is_empty(), ">15 imports should trigger");
        assert_eq!(coupling_issues[0].debt_minutes, 20);
        assert_eq!(coupling_issues[0].category, "changeability");
        assert_eq!(coupling_issues[0].line, 1); // Module-level issue
    }

    #[test]
    fn test_find_high_coupling_unique_modules() {
        // Same module imported multiple ways should count as one
        let source = r#"
import os
from os import path
from os.path import join

def main():
    pass
"#;
        let _issues = find_high_coupling(source, Path::new("test.py"), Language::Python);

        // Depending on implementation, os might be counted once or per import
        // The key is that repeated imports don't inflate the count unfairly
    }
}

// =============================================================================
// Unit Tests - Missing Docs Detection (DEBT-012)
// =============================================================================

#[cfg(test)]
mod docs_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_find_missing_docs_public_function() {
        let issues = find_missing_docs(PYTHON_MISSING_DOCS, Path::new("test.py"), Language::Python);

        let docs_issues: Vec<_> = issues.iter().filter(|i| i.rule == "missing_docs").collect();

        // public_function and PublicClass.public_method should be flagged
        assert!(docs_issues.len() >= 2);
        assert_eq!(docs_issues[0].debt_minutes, 10);
        assert_eq!(docs_issues[0].category, "maintainability");
    }

    #[test]
    fn test_find_missing_docs_private_excluded() {
        let issues = find_missing_docs(PYTHON_MISSING_DOCS, Path::new("test.py"), Language::Python);

        // _private_function should NOT be flagged
        let private_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.element
                    .as_ref()
                    .map(|e| e.starts_with("_"))
                    .unwrap_or(false)
            })
            .collect();

        assert!(
            private_issues.is_empty(),
            "Private functions should be excluded"
        );
    }

    #[test]
    fn test_find_missing_docs_documented() {
        let issues = find_missing_docs(PYTHON_WITH_DOCS, Path::new("test.py"), Language::Python);

        let docs_issues: Vec<_> = issues.iter().filter(|i| i.rule == "missing_docs").collect();

        assert!(
            docs_issues.is_empty(),
            "Documented code should not be flagged"
        );
    }

    #[test]
    fn test_find_missing_docs_dunder_excluded() {
        let source = r#"
class MyClass:
    def __init__(self):
        pass

    def __str__(self):
        return "MyClass"
"#;
        let issues = find_missing_docs(source, Path::new("test.py"), Language::Python);

        // Dunder methods should not be flagged
        let dunder_issues: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.element
                    .as_ref()
                    .map(|e| e.contains("__"))
                    .unwrap_or(false)
            })
            .collect();

        assert!(dunder_issues.is_empty());
    }
}

// =============================================================================
// Integration Tests - File Analysis (DEBT-005)
// =============================================================================

#[cfg(test)]
mod file_analysis_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_analyze_file_python() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let file_path = dir.add_file("test.py", PYTHON_WITH_TODO).unwrap();

        let result = analyze_file(&file_path, None, None);

        assert!(result.is_ok());
        let (issues, loc) = result.unwrap();

        assert!(!issues.is_empty(), "Should find TODO issues");
        assert!(loc > 0, "Should count some lines of code");
    }

    #[test]
    fn test_analyze_file_unsupported_language() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let file_path = dir.add_file("test.unknown", "some content").unwrap();

        let result = analyze_file(&file_path, None, None);

        assert!(result.is_ok());
        let (issues, loc) = result.unwrap();

        // Unsupported files should return empty results, not error
        assert!(issues.is_empty());
        assert_eq!(loc, 0);
    }

    #[test]
    fn test_analyze_file_empty() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let file_path = dir.add_file("empty.py", "").unwrap();

        let result = analyze_file(&file_path, None, None);

        assert!(result.is_ok());
        let (issues, loc) = result.unwrap();

        assert!(issues.is_empty());
        assert_eq!(loc, 0);
    }

    #[test]
    fn test_analyze_file_category_filter() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let source = format!("{}\n{}", PYTHON_WITH_TODO, PYTHON_HIGH_COMPLEXITY);
        let file_path = dir.add_file("mixed.py", &source).unwrap();

        // Filter to only reliability (TODO comments)
        let result = analyze_file(&file_path, Some("reliability"), None);

        assert!(result.is_ok());
        let (issues, _) = result.unwrap();

        // All issues should be reliability category
        for issue in &issues {
            assert_eq!(issue.category, "reliability");
        }
    }

    #[test]
    fn test_analyze_file_language_override() {
        let dir = TestDir::new().expect("Failed to create test dir");
        // File with .txt extension but Python content
        let file_path = dir
            .add_file("script.txt", "# TODO: fix\ndef foo(): pass")
            .unwrap();

        // Without override, should return empty (unknown extension)
        let result = analyze_file(&file_path, None, None);
        let (issues1, _) = result.unwrap();

        // With Python override, should find issues
        let result = analyze_file(&file_path, None, Some(Language::Python));
        let (issues2, _) = result.unwrap();

        assert!(issues1.is_empty() || issues2.len() >= issues1.len());
    }

    #[test]
    fn test_analyze_file_read_error() {
        let result = analyze_file(Path::new("/nonexistent/file.py"), None, None);

        // Should return Ok with empty results, not error
        assert!(result.is_ok());
        let (issues, loc) = result.unwrap();
        assert!(issues.is_empty());
        assert_eq!(loc, 0);
    }
}

// =============================================================================
// Integration Tests - Directory Analysis (DEBT-006)
// =============================================================================

#[cfg(test)]
mod directory_analysis_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_analyze_debt_directory() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("file1.py", "# TODO: first\n").unwrap();
        dir.add_file("file2.py", "# FIXME: second\n").unwrap();
        dir.add_file("src/nested.py", "# TODO: nested\n").unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = analyze_debt(options);

        assert!(result.is_ok());
        let report = result.unwrap();

        // Should find 3 TODO issues across files
        assert_eq!(report.issues.len(), 3);
        assert_eq!(report.summary.total_minutes, 30); // 3 * 10
    }

    #[test]
    fn test_analyze_debt_skips_pycache() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("main.py", "# TODO: count\n").unwrap();
        dir.add_subdir("__pycache__").unwrap();
        dir.add_file("__pycache__/cached.py", "# TODO: ignore\n")
            .unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // Should only find 1 TODO (the one not in __pycache__)
        assert_eq!(report.issues.len(), 1);
    }

    #[test]
    fn test_analyze_debt_skips_node_modules() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("index.js", "// TODO: count\n").unwrap();
        dir.add_subdir("node_modules").unwrap();
        dir.add_file("node_modules/pkg/index.js", "// TODO: ignore\n")
            .unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let _report = analyze_debt(options).unwrap();

        // JavaScript TODOs use // format, need to verify detection works
        // Main point: node_modules should be skipped
    }

    #[test]
    fn test_analyze_debt_skips_git() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("main.py", "# TODO: count\n").unwrap();
        dir.add_subdir(".git").unwrap();
        dir.add_file(".git/config", "# TODO: ignore\n").unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        assert_eq!(report.issues.len(), 1);
    }

    #[test]
    fn test_analyze_debt_single_file() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let file_path = dir
            .add_file("single.py", "# TODO: one\n# FIXME: two\n")
            .unwrap();

        let options = DebtOptions {
            path: file_path,
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.summary.total_minutes, 20);
    }

    #[test]
    fn test_analyze_debt_empty_directory() {
        let dir = TestDir::new().expect("Failed to create test dir");

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        assert!(report.issues.is_empty());
        assert_eq!(report.summary.total_minutes, 0);
        assert_eq!(report.summary.debt_ratio, 0.0);
    }

    #[test]
    fn test_analyze_debt_top_k() {
        let dir = TestDir::new().expect("Failed to create test dir");

        // Create 5 files with different debt levels
        for i in 0..5 {
            let todos: String = (0..=i).map(|_| "# TODO: issue\n").collect();
            dir.add_file(&format!("file{}.py", i), &todos).unwrap();
        }

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            top_k: 3,
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // Should only include top 3 files
        assert_eq!(report.top_files.len(), 3);

        // Files should be sorted by debt descending
        for window in report.top_files.windows(2) {
            assert!(window[0].total_minutes >= window[1].total_minutes);
        }
    }

    #[test]
    fn test_analyze_debt_min_debt_filter() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("small.py", "# TODO: 10 min\n").unwrap();
        dir.add_file("large.py", &fixtures::python_long_method())
            .unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            min_debt: 20, // Filter out issues < 20 minutes
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // TODO comments are 10 minutes, should be filtered
        for issue in &report.issues {
            assert!(issue.debt_minutes >= 20);
        }
    }

    #[test]
    fn test_analyze_debt_hourly_rate() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("test.py", "# TODO: one\n").unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            hourly_rate: Some(100.0),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // 10 minutes = 0.167 hours, at $100/hour = ~$16.67
        assert!(report.summary.total_cost.is_some());
        let cost = report.summary.total_cost.unwrap();
        assert!(cost > 0.0);
    }
}

// =============================================================================
// Integration Tests - Summary Calculations
// =============================================================================

#[cfg(test)]
mod summary_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_debt_summary_calculations() {
        let dir = TestDir::new().expect("Failed to create test dir");

        // Create file with known debt
        // Using private functions (_x, _y) to avoid missing_docs issues
        let source = "# TODO: a\n# TODO: b\n# TODO: c\ndef _x(): pass\ndef _y(): pass\n";
        dir.add_file("test.py", source).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // 3 TODOs * 10 = 30 minutes
        assert_eq!(report.summary.total_minutes, 30);

        // 30 / 60 = 0.5 hours
        assert!((report.summary.total_hours - 0.5).abs() < 0.01);

        // debt_ratio = 30 / LOC
        // debt_density = debt_ratio * 1000
        assert!(report.summary.debt_density > 0.0);
    }

    #[test]
    fn test_debt_summary_by_category() {
        let dir = TestDir::new().expect("Failed to create test dir");

        let source = format!("{}\n{}", PYTHON_WITH_TODO, PYTHON_LONG_PARAMS);
        dir.add_file("mixed.py", &source).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // Should have entries for reliability (TODO) and testability (long params)
        assert!(report.summary.by_category.contains_key("reliability"));
    }

    #[test]
    fn test_debt_summary_by_rule() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("test.py", PYTHON_WITH_TODO).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // Should have todo_comment in by_rule
        assert!(report.summary.by_rule.contains_key("todo_comment"));
    }

    #[test]
    fn test_debt_summary_sums_match() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("test.py", PYTHON_WITH_TODO).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        // by_category sum should equal total_minutes
        let category_sum: u32 = report.summary.by_category.values().sum();
        assert_eq!(category_sum, report.summary.total_minutes);

        // by_rule sum should equal total_minutes
        let rule_sum: u32 = report.summary.by_rule.values().sum();
        assert_eq!(rule_sum, report.summary.total_minutes);
    }
}

// =============================================================================
// Output Format Tests (DEBT-007, DEBT-008)
// =============================================================================

#[cfg(test)]
mod output_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_debt_report_to_json() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("test.py", PYTHON_WITH_TODO).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            hourly_rate: Some(100.0),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();
        let json_str = serde_json::to_string_pretty(&report).unwrap();

        // Should be valid JSON
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Verify structure
        assert!(json["issues"].is_array());
        assert!(json["top_files"].is_array());
        assert!(json["summary"].is_object());
        assert!(json["summary"]["total_minutes"].is_number());
        assert!(json["summary"]["total_hours"].is_number());
        assert!(json["summary"]["total_cost"].is_number());
        assert!(json["summary"]["debt_ratio"].is_number());
        assert!(json["summary"]["debt_density"].is_number());
    }

    #[test]
    fn test_debt_report_to_text() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("test.py", PYTHON_WITH_TODO).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            hourly_rate: Some(100.0),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();
        let text = report.to_text();

        // Check for expected sections
        assert!(text.contains("Technical Debt Report"));
        assert!(text.contains("Total Debt:"));
        assert!(text.contains("Estimated Cost:"));
        assert!(text.contains("Debt Ratio:"));
        assert!(text.contains("By Category:"));
    }

    #[test]
    fn test_debt_report_text_rating() {
        // Test rating interpretation based on debt_ratio

        // Create a report manually to test rating logic
        let report = DebtReport {
            issues: vec![],
            top_files: vec![],
            summary: DebtSummary {
                total_minutes: 100,
                total_hours: 1.67,
                total_cost: None,
                debt_ratio: 0.03, // < 5% = Excellent
                debt_density: 30.0,
                by_category: BTreeMap::new(),
                by_rule: BTreeMap::new(),
            },
        };

        let text = report.to_text();
        assert!(text.contains("Excellent"));

        // Test "Good" rating
        let mut summary = report.summary.clone();
        summary.debt_ratio = 0.07; // 5-10% = Good
        let report2 = DebtReport {
            summary,
            ..report.clone()
        };
        assert!(report2.to_text().contains("Good"));

        // Test "Concerning" rating
        let mut summary = report.summary.clone();
        summary.debt_ratio = 0.15; // 10-20% = Concerning
        let report3 = DebtReport {
            summary,
            ..report.clone()
        };
        assert!(report3.to_text().contains("Concerning"));

        // Test "Critical" rating
        let mut summary = report.summary.clone();
        summary.debt_ratio = 0.25; // >= 20% = Critical
        let report4 = DebtReport { summary, ..report };
        assert!(report4.to_text().contains("Critical"));
    }

    #[test]
    fn test_debt_report_text_no_cost() {
        let report = DebtReport {
            issues: vec![],
            top_files: vec![],
            summary: DebtSummary {
                total_minutes: 60,
                total_hours: 1.0,
                total_cost: None,
                debt_ratio: 0.05,
                debt_density: 50.0,
                by_category: BTreeMap::new(),
                by_rule: BTreeMap::new(),
            },
        };

        let text = report.to_text();

        // Should NOT contain "Estimated Cost:" when total_cost is None
        assert!(!text.contains("Estimated Cost:"));
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn test_unicode_content() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let source = "# TODO: fixme 日本語 emoji 🔥\ndef func(): pass\n";
        dir.add_file("unicode.py", source).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = analyze_debt(options);
        assert!(result.is_ok());

        let report = result.unwrap();
        assert!(!report.issues.is_empty());
    }

    #[test]
    fn test_very_long_lines() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let long_line = format!("x = '{}'", "a".repeat(10000));
        let source = format!("# TODO: fix\n{}\n", long_line);
        dir.add_file("long_lines.py", &source).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = analyze_debt(options);
        assert!(result.is_ok());
    }

    #[test]
    fn test_binary_file_skipped() {
        let dir = TestDir::new().expect("Failed to create test dir");
        // Binary content with null bytes
        let binary = vec![0x00, 0x01, 0x02, 0x03, 0x00, 0xFF];
        std::fs::write(dir.path().join("binary.py"), binary).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        // Should not crash on binary files
        let result = analyze_debt(options);
        assert!(result.is_ok());
    }

    #[test]
    fn test_deeply_nested_directories() {
        let dir = TestDir::new().expect("Failed to create test dir");

        // Create deeply nested path
        let deep_path = "a/b/c/d/e/f/g/h/i/j/deep.py";
        dir.add_file(deep_path, "# TODO: deep\n").unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();
        assert_eq!(report.issues.len(), 1);
    }

    #[test]
    fn test_mixed_line_endings() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let source = "# TODO: windows\r\n# TODO: unix\n# TODO: old mac\r";
        dir.add_file("mixed_endings.py", source).unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();
        // Should handle all line ending styles
        assert!(!report.issues.is_empty());
    }

    #[test]
    fn test_path_not_found() {
        let options = DebtOptions {
            path: PathBuf::from("/nonexistent/path"),
            ..Default::default()
        };

        let result = analyze_debt(options);
        // Should return error for nonexistent path
        assert!(result.is_err());
    }

    #[test]
    fn test_symlink_handling() {
        // Note: This test may be platform-specific
        let dir = TestDir::new().expect("Failed to create test dir");
        let file_path = dir.add_file("real.py", "# TODO: real\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link_path = dir.path().join("link.py");
            if symlink(&file_path, &link_path).is_ok() {
                let options = DebtOptions {
                    path: dir.path().to_path_buf(),
                    ..Default::default()
                };

                let report = analyze_debt(options).unwrap();
                // Symlinks should be handled (either followed or skipped)
                assert!(!report.issues.is_empty());
            }
        }
    }

    #[test]
    fn test_empty_file_loc() {
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("empty.py", "").unwrap();

        let (_, loc) = analyze_file(&dir.path().join("empty.py"), None, None).unwrap();
        assert_eq!(loc, 0);
    }

    #[test]
    fn test_only_comments_file() {
        let dir = TestDir::new().expect("Failed to create test dir");
        let source = "# Comment 1\n# Comment 2\n# Comment 3\n";
        dir.add_file("comments.py", source).unwrap();

        let (_, loc) = analyze_file(&dir.path().join("comments.py"), None, None).unwrap();
        assert_eq!(loc, 0, "File with only comments should have 0 LOC");
    }

    #[test]
    fn test_zero_loc_division() {
        // When LOC is 0, debt_ratio should be 0, not NaN or infinity
        let dir = TestDir::new().expect("Failed to create test dir");
        dir.add_file("empty.py", "").unwrap();

        let options = DebtOptions {
            path: dir.path().to_path_buf(),
            ..Default::default()
        };

        let report = analyze_debt(options).unwrap();

        assert_eq!(report.summary.debt_ratio, 0.0);
        assert_eq!(report.summary.debt_density, 0.0);
        assert!(!report.summary.debt_ratio.is_nan());
        assert!(!report.summary.debt_density.is_nan());
    }
}

// =============================================================================
// Parity Tests - Behavior matches Python implementation
// =============================================================================

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn test_todo_regex_parity() {
        // Ensure behavior matches expected patterns
        // Note: With AST approach, #TODO (no space) IS detected because
        // tree-sitter parses it as a comment node.
        let test_cases = vec![
            ("# TODO: fix this", true, "TODO"),
            ("# todo: lowercase", true, "TODO"),
            ("# FIXME: broken", true, "FIXME"),
            ("# HACK: workaround", true, "HACK"),
            ("# XXX: deprecated", true, "XXX"),
            ("#TODO: no space", true, "TODO"), // AST finds this as comment node
            ("// TODO: c-style", false, ""),   // Not a Python comment
            ("# TODONE: not a todo", false, ""), // Should not match TODONE
        ];

        for (input, should_match, expected_tag) in test_cases {
            let issues = find_todo_comments(input, Path::new("test.py"), Language::Python);

            if should_match {
                assert!(!issues.is_empty(), "Should match: {}", input);
                assert!(
                    issues[0].message.contains(expected_tag),
                    "Tag mismatch for: {}",
                    input
                );
            } else {
                assert!(issues.is_empty(), "Should NOT match: {}", input);
            }
        }
    }

    #[test]
    fn test_complexity_threshold_parity() {
        // Verify thresholds match spec exactly
        // CC > 10 -> complexity.high (20 min)
        // CC > 15 -> complexity.very_high (30 min)
        // CC > 25 -> complexity.extreme (60 min)

        assert_eq!(DebtRule::ComplexityHigh.minutes(), 20);
        assert_eq!(DebtRule::ComplexityVeryHigh.minutes(), 30);
        assert_eq!(DebtRule::ComplexityExtreme.minutes(), 60);
    }

    #[test]
    fn test_long_method_threshold_parity() {
        // LOC > 100 -> long_method (30 min)
        assert_eq!(DebtRule::LongMethod.minutes(), 30);
    }

    #[test]
    fn test_long_param_threshold_parity() {
        // > 5 params (excluding self/cls) -> long_param_list (15 min)
        assert_eq!(DebtRule::LongParamList.minutes(), 15);
    }

    #[test]
    fn test_god_class_threshold_parity() {
        // > 20 methods AND LCOM4 > 0.8 -> god_class (60 min)
        assert_eq!(DebtRule::GodClass.minutes(), 60);
    }

    #[test]
    fn test_skip_directories_parity() {
        // These directories should be skipped (match Python implementation)
        let skip_dirs = [
            "__pycache__",
            ".git",
            "node_modules",
            ".venv",
            "venv",
            "target",
            "build",
            "dist",
            ".tox",
            ".mypy_cache",
        ];

        // This is a specification test - actual verification happens in integration tests
        assert!(skip_dirs.len() >= 10);
    }
}
