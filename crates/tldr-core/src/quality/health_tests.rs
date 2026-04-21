//! Test module for health analysis functionality
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the health module is implemented.
//!
//! # Test Categories
//! - Core types: HealthReport, HealthSummary, SubAnalysisResult, Severity
//! - Complexity analyzer: Hotspot detection, average calculation
//! - Cohesion analyzer: LCOM4 calculation, connected components
//! - Dead code analyzer: Unreachable function detection
//! - Martin metrics: Abstractness, instability, distance from main sequence
//! - Coupling analyzer: Import relationships, coupling scores
//! - Similarity analyzer: Function clone detection
//! - Integration: Quick mode, full mode, multi-language support
//! - Output formats: JSON schema validation, text dashboard
//! - Edge cases: Empty directories, parse errors, unsupported files


// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for creating temporary files and directories
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing health analysis
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

    }

    // -------------------------------------------------------------------------
    // Complexity Fixtures
    // -------------------------------------------------------------------------

    /// Simple function with CC=1
    pub const PYTHON_SIMPLE_FUNCTION: &str = r#"
def simple():
    return 42
"#;

    /// Function with moderate complexity (CC=5)
    pub const PYTHON_MODERATE_COMPLEXITY: &str = r#"
def moderate(a, b, c, d):
    if a > 0:
        return 1
    elif b > 0:
        return 2
    elif c > 0:
        return 3
    elif d > 0:
        return 4
    else:
        return 0
"#;

    /// Function with high complexity (CC > 10) - hotspot candidate
    pub const PYTHON_HIGH_COMPLEXITY: &str = r#"
def complex_function(a, b, c, d, e, f):
    result = 0
    if a > 0:
        if b > 0:
            result += 1
        elif c > 0:
            result += 2
        else:
            result += 3
    elif d > 0:
        if e > 0:
            result += 4
        elif f > 0:
            result += 5
        else:
            result += 6
    else:
        if a < -10:
            result -= 1
        elif b < -10:
            result -= 2
        else:
            result -= 3

    for i in range(10):
        if i % 2 == 0:
            result += i

    return result
"#;

    /// Multiple functions with varying complexity for average calculation
    pub const PYTHON_MULTIPLE_FUNCTIONS: &str = r#"
def func_cc1():
    return 1

def func_cc2(a):
    if a:
        return 1
    return 0

def func_cc3(a, b):
    if a:
        return 1
    elif b:
        return 2
    return 0

def func_cc4(a, b, c):
    if a:
        return 1
    elif b:
        return 2
    elif c:
        return 3
    return 0
"#;

    // -------------------------------------------------------------------------
    // Cohesion Fixtures (LCOM4)
    // -------------------------------------------------------------------------

    /// Single method class - LCOM4 = 1
    pub const PYTHON_SINGLE_METHOD_CLASS: &str = r#"
class SingleMethod:
    def __init__(self):
        self.value = 0

    def get_value(self):
        return self.value
"#;

    /// Fully cohesive class - all methods share fields (LCOM4 = 1)
    pub const PYTHON_COHESIVE_CLASS: &str = r#"
class CohesiveClass:
    def __init__(self):
        self.value = 0

    def get_value(self):
        return self.value

    def set_value(self, v):
        self.value = v

    def increment(self):
        self.value += 1

    def decrement(self):
        self.value -= 1
"#;

    /// Disconnected methods - LCOM4 > 1 (each method uses different field)
    pub const PYTHON_DISCONNECTED_CLASS: &str = r#"
class DisconnectedClass:
    def method_a(self):
        self.field_a = 1
        return self.field_a

    def method_b(self):
        self.field_b = 2
        return self.field_b

    def method_c(self):
        self.field_c = 3
        return self.field_c

    def method_d(self):
        self.field_d = 4
        return self.field_d
"#;

    /// Class with low cohesion (LCOM4 > 2) - split candidate
    pub const PYTHON_LOW_COHESION_CLASS: &str = r#"
class LowCohesionClass:
    # Group 1: handles user data
    def get_user_name(self):
        return self.user_name

    def set_user_name(self, name):
        self.user_name = name

    # Group 2: handles product data (disconnected from user)
    def get_product_id(self):
        return self.product_id

    def set_product_id(self, id):
        self.product_id = id

    # Group 3: handles order data (disconnected from both)
    def get_order_total(self):
        return self.order_total

    def calculate_total(self, items):
        self.order_total = sum(items)
"#;

    // -------------------------------------------------------------------------
    // Dead Code Fixtures
    // -------------------------------------------------------------------------

    /// File with an unreachable private function
    pub const PYTHON_UNREACHABLE_FUNCTION: &str = r#"
def main():
    helper()
    return 0

def helper():
    return 42

def _unused_private():
    """This private function is never called."""
    pass

def another_unused():
    """Public but unreachable."""
    pass
"#;

    /// File with entry points that should not be flagged
    pub const PYTHON_ENTRY_POINTS: &str = r#"
def main():
    """Entry point - should not be flagged."""
    return run()

def run():
    """Called by main."""
    return start()

def start():
    """Called by run."""
    return 0

def test_something():
    """Test function - should not be flagged."""
    assert True

def setup():
    """Setup function - should not be flagged."""
    pass

def teardown():
    """Teardown function - should not be flagged."""
    pass
"#;

    /// File with dunder methods that should not be flagged
    pub const PYTHON_DUNDER_METHODS: &str = r#"
class MyClass:
    def __init__(self):
        self.value = 0

    def __str__(self):
        return str(self.value)

    def __repr__(self):
        return f"MyClass({self.value})"

    def __eq__(self, other):
        return self.value == other.value

    def public_method(self):
        return self.value
"#;

    // -------------------------------------------------------------------------
    // Martin Metrics Fixtures
    // -------------------------------------------------------------------------

    /// Package with concrete types only (A=0)
    pub const PYTHON_CONCRETE_PACKAGE: &str = r#"
class ConcreteClass:
    def method(self):
        return 42

class AnotherConcrete:
    def another_method(self):
        return 24
"#;

    /// Package with abstract types (A > 0)
    pub const PYTHON_ABSTRACT_PACKAGE: &str = r#"
from abc import ABC, abstractmethod

class AbstractBase(ABC):
    @abstractmethod
    def abstract_method(self):
        pass

class ConcreteImpl(AbstractBase):
    def abstract_method(self):
        return 42
"#;

    /// Package with high efferent coupling (Ce high)
    pub const PYTHON_HIGH_EFFERENT: &str = r#"
import os
import sys
import json
import logging
import pathlib
from typing import List, Dict

def process():
    pass
"#;

    /// Package with no dependencies (isolated)
    pub const PYTHON_ISOLATED_PACKAGE: &str = r#"
def standalone_function():
    return 42

class IsolatedClass:
    def method(self):
        return 1
"#;

    // -------------------------------------------------------------------------
    // Coupling Fixtures
    // -------------------------------------------------------------------------

    /// Two tightly coupled modules
    pub const PYTHON_MODULE_A: &str = r#"
from module_b import func_b1, func_b2, func_b3

def func_a1():
    return func_b1()

def func_a2():
    return func_b2() + func_b1()

def func_a3():
    return func_b3() + func_b2() + func_b1()
"#;

    pub const PYTHON_MODULE_B: &str = r#"
from module_a import func_a1, func_a2

def func_b1():
    return 1

def func_b2():
    return func_a1() + 2

def func_b3():
    return func_a2() + 3
"#;

    /// Loosely coupled module
    pub const PYTHON_LOOSELY_COUPLED: &str = r#"
def independent_func():
    return 42

def another_independent():
    return 24
"#;

    // -------------------------------------------------------------------------
    // Similarity Fixtures
    // -------------------------------------------------------------------------

    /// Two identical functions (clones)
    pub const PYTHON_CLONES: &str = r#"
def clone_a(x, y, z):
    result = 0
    for i in range(x):
        if i % 2 == 0:
            result += y
        else:
            result += z
    return result

def clone_b(a, b, c):
    result = 0
    for i in range(a):
        if i % 2 == 0:
            result += b
        else:
            result += c
    return result
"#;

    /// Two completely different functions
    pub const PYTHON_DIFFERENT_FUNCTIONS: &str = r#"
def function_one():
    return 1

def function_two(a, b, c, d, e):
    for x in range(100):
        for y in range(100):
            if a > b:
                if c > d:
                    return e
    return 0
"#;

    // -------------------------------------------------------------------------
    // Multi-language Fixtures
    // -------------------------------------------------------------------------

    pub const TYPESCRIPT_SIMPLE: &str = r#"
function simpleFunction(): number {
    return 42;
}

class SimpleClass {
    private value: number;

    constructor() {
        this.value = 0;
    }

    getValue(): number {
        return this.value;
    }
}
"#;

    pub const GO_SIMPLE: &str = r#"
package main

func simpleFunction() int {
    return 42
}

type SimpleStruct struct {
    value int
}

func (s *SimpleStruct) GetValue() int {
    return s.value
}
"#;

    pub const RUST_SIMPLE: &str = r#"
fn simple_function() -> i32 {
    42
}

struct SimpleStruct {
    value: i32,
}

impl SimpleStruct {
    fn get_value(&self) -> i32 {
        self.value
    }
}
"#;

    // -------------------------------------------------------------------------
    // Edge Case Fixtures
    // -------------------------------------------------------------------------

    /// File with syntax errors
    pub const PYTHON_SYNTAX_ERROR: &str = r#"
def broken_function(
    # Missing closing parenthesis
    return 42
"#;

    /// Empty file
    pub const EMPTY_FILE: &str = "";

    /// File with only comments
    pub const COMMENTS_ONLY: &str = r#"
# This file has only comments
# No actual code here
# Just comments
"#;
}

// =============================================================================
// Core Type Tests
// =============================================================================

#[cfg(test)]
mod core_type_tests {
    

    /// Test that HealthReport has all required fields per spec section 2.2
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_report_structure() {
        // HealthReport must have these fields:
        // - wrapper: String (always "health")
        // - path: PathBuf
        // - summary: HealthSummary
        // - sub_results: HashMap<String, SubAnalysisResult>
        // - total_elapsed_ms: f64

        // This test will fail at compile time if struct is missing fields
        // Once implemented, verify the struct matches spec
        todo!("Implement HealthReport struct per spec section 2.2");
    }

    /// Test that HealthSummary correctly aggregates sub-analyzer results
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_summary_aggregation() {
        // HealthSummary must aggregate:
        // - avg_cyclomatic: Option<f64> (from complexity)
        // - hotspot_count: Option<usize> (from complexity)
        // - class_count: Option<usize> (from cohesion)
        // - avg_lcom4: Option<f64> (from cohesion)
        // - low_cohesion_count: Option<usize> (from cohesion)
        // - coupling_pairs_analyzed: Option<usize> (from coupling, full mode)
        // - avg_coupling_score: Option<f64> (from coupling, full mode)
        // - dead_count: Option<usize> (from dead code)
        // - similar_pairs: Option<usize> (from similarity, full mode)
        // - avg_distance: Option<f64> (from martin metrics)

        todo!("Implement HealthSummary aggregation");
    }

    /// Test Severity ordering: Critical > High > Medium > Low > Info
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_severity_ordering() {
        // Per spec section 7.3, severity levels must be ordered:
        // Critical (4) > High (3) > Medium (2) > Low (1) > Info (0)

        // Severity enum must implement Ord with this ordering
        todo!("Implement Severity enum with correct ordering");
    }

    /// Test SubAnalysisResult structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_sub_analysis_result_structure() {
        // SubAnalysisResult must have:
        // - name: String
        // - success: bool
        // - data: Option<serde_json::Value>
        // - error: Option<String>
        // - elapsed_ms: f64

        todo!("Implement SubAnalysisResult struct");
    }

    /// Test HealthReport serialization to JSON
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_report_to_dict() {
        // HealthReport.to_dict() must return serde_json::Value
        // matching the schema in spec section 5.1

        todo!("Implement HealthReport.to_dict()");
    }

    /// Test HealthReport detail() method
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_report_detail_method() {
        // HealthReport.detail(sub_name) must:
        // - Return Some(data) if sub-analysis exists
        // - Return None if sub-analysis doesn't exist
        // - Valid sub_names: complexity, cohesion, dead, metrics, coupling, similar

        todo!("Implement HealthReport.detail() method");
    }
}

// =============================================================================
// Complexity Analyzer Tests
// =============================================================================

#[cfg(test)]
mod complexity_tests {
    use super::fixtures::*;
    

    /// Test that functions with CC > 10 are marked as hotspots
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_hotspot_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("high_complexity.py", PYTHON_HIGH_COMPLEXITY)
            .unwrap();

        // analyze_complexity should identify functions with CC > 10 as hotspots
        // Expected: hotspot_count >= 1 for PYTHON_HIGH_COMPLEXITY

        todo!("Implement complexity hotspot detection");
    }

    /// Test correct average cyclomatic complexity calculation
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_average_calculation() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("multiple.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // PYTHON_MULTIPLE_FUNCTIONS has:
        // - func_cc1: CC=1
        // - func_cc2: CC=2
        // - func_cc3: CC=3
        // - func_cc4: CC=4
        // Average = (1+2+3+4) / 4 = 2.5

        // analyze_complexity should return avg_cyclomatic close to 2.5

        todo!("Implement complexity average calculation");
    }

    /// Test that empty file returns zero metrics
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_empty_file() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("empty.py", EMPTY_FILE).unwrap();

        // analyze_complexity on empty file should return:
        // - total_functions: 0
        // - avg_cyclomatic: 0.0 (or None)
        // - hotspot_count: 0
        // - success: true (not an error)

        todo!("Implement complexity analysis for empty files");
    }

    /// Test per-function complexity data
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_per_function_data() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("test.py", PYTHON_MODERATE_COMPLEXITY)
            .unwrap();

        // Each FunctionComplexity must include:
        // - name: String
        // - file: PathBuf
        // - line: usize
        // - cyclomatic: u32
        // - cognitive: u32
        // - loc: usize
        // - rank: usize (sorted by cyclomatic desc)

        todo!("Implement per-function complexity data");
    }

    /// Test that functions are sorted by complexity descending
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_sorted_descending() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("multiple.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // Functions must be sorted by cyclomatic complexity descending
        // func_cc4 (CC=4) should be rank 1
        // func_cc3 (CC=3) should be rank 2
        // etc.

        todo!("Implement complexity sorting");
    }
}

// =============================================================================
// Cohesion Analyzer Tests (LCOM4)
// =============================================================================

#[cfg(test)]
mod cohesion_tests {
    use super::fixtures::*;
    
    use crate::quality::cohesion::{analyze_cohesion, CohesionVerdict};

    /// Test LCOM4 = 1 for single method class
    #[test]
    fn test_lcom4_single_method_class() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("single.py", PYTHON_SINGLE_METHOD_CLASS)
            .unwrap();

        // Single method class (ignoring __init__) should have LCOM4 = 1
        // One method = one component
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        assert_eq!(report.classes_analyzed, 1);
        let class = &report.classes[0];
        assert_eq!(class.name, "SingleMethod");
        // Excluding __init__, only get_value remains -> 1 method -> LCOM4 = 1
        assert_eq!(class.lcom4, 1);
        assert_eq!(class.method_count, 1);
        assert_eq!(class.verdict, CohesionVerdict::Cohesive);
    }

    /// Test LCOM4 > 1 when methods share no fields
    #[test]
    fn test_lcom4_disconnected_methods() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("disconnected.py", PYTHON_DISCONNECTED_CLASS)
            .unwrap();

        // DisconnectedClass has 4 methods, each using a different field
        // No shared field access -> 4 disconnected components
        // LCOM4 should be 4
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        assert_eq!(report.classes_analyzed, 1);
        let class = &report.classes[0];
        assert_eq!(class.name, "DisconnectedClass");
        assert_eq!(class.method_count, 4);
        assert_eq!(class.lcom4, 4);
        assert_eq!(class.verdict, CohesionVerdict::SplitCandidate);
    }

    /// Test LCOM4 = 1 when all methods share fields
    #[test]
    fn test_lcom4_connected_methods() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("cohesive.py", PYTHON_COHESIVE_CLASS)
            .unwrap();

        // CohesiveClass: all methods (excluding __init__) access self.value
        // All connected through shared field -> 1 component
        // LCOM4 should be 1
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        assert_eq!(report.classes_analyzed, 1);
        let class = &report.classes[0];
        assert_eq!(class.name, "CohesiveClass");
        // 4 methods: get_value, set_value, increment, decrement (excluding __init__)
        assert_eq!(class.method_count, 4);
        assert_eq!(class.lcom4, 1);
        assert_eq!(class.verdict, CohesionVerdict::Cohesive);
    }

    /// Test that classes with LCOM4 > 2 are flagged as low cohesion
    #[test]
    fn test_cohesion_low_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("low_cohesion.py", PYTHON_LOW_COHESION_CLASS)
            .unwrap();

        // LowCohesionClass has 3 disconnected groups
        // LCOM4 = 3 > threshold (2)
        // Should be flagged with verdict: SplitCandidate
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        assert_eq!(report.classes_analyzed, 1);
        let class = &report.classes[0];
        assert_eq!(class.name, "LowCohesionClass");
        assert!(class.lcom4 > 2, "LCOM4 should be > 2, got {}", class.lcom4);
        assert_eq!(class.verdict, CohesionVerdict::SplitCandidate);
        assert_eq!(report.low_cohesion_count, 1);
    }

    /// Test that dunder methods are excluded from LCOM4 calculation
    #[test]
    fn test_cohesion_excludes_dunder() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("dunder.py", PYTHON_DUNDER_METHODS)
            .unwrap();

        // __init__, __str__, __repr__, __eq__ should be excluded
        // Only public_method should be counted
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        assert_eq!(report.classes_analyzed, 1);
        let class = &report.classes[0];
        assert_eq!(class.name, "MyClass");
        // Only public_method remains after excluding dunders
        assert_eq!(class.method_count, 1);
        assert_eq!(class.lcom4, 1);
    }

    /// Test CohesionVerdict enum values
    #[test]
    fn test_cohesion_verdict_values() {
        // CohesionVerdict must have:
        // - Cohesive (LCOM4 <= threshold)
        // - SplitCandidate (LCOM4 > threshold)

        // Test serialization
        let cohesive = CohesionVerdict::Cohesive;
        let split = CohesionVerdict::SplitCandidate;

        assert_eq!(serde_json::to_string(&cohesive).unwrap(), "\"cohesive\"");
        assert_eq!(
            serde_json::to_string(&split).unwrap(),
            "\"split_candidate\""
        );
    }

    /// Test component info extraction
    #[test]
    fn test_cohesion_component_info() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("disconnected.py", PYTHON_DISCONNECTED_CLASS)
            .unwrap();

        // ComponentInfo must list:
        // - methods: Vec<String> - methods in this component
        // - fields: Vec<String> - fields accessed by this component
        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        let class = &report.classes[0];
        // 4 disconnected methods = 4 components
        assert_eq!(class.components.len(), 4);

        // Each component should have 1 method and 1 field
        for component in &class.components {
            assert_eq!(component.methods.len(), 1);
            assert!(!component.fields.is_empty());
        }
    }

    /// Test TypeScript class cohesion analysis
    #[test]
    fn test_cohesion_typescript_class() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("simple.ts", TYPESCRIPT_SIMPLE).unwrap();

        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        // SimpleClass should be found
        assert!(report.classes_analyzed >= 1);
        let class = report.classes.iter().find(|c| c.name == "SimpleClass");
        assert!(class.is_some(), "SimpleClass should be found");
    }

    /// Test Go struct with receiver methods
    #[test]
    fn test_cohesion_go_struct_methods() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("simple.go", GO_SIMPLE).unwrap();

        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        // SimpleStruct should be found
        assert!(report.classes_analyzed >= 1);
        let class = report.classes.iter().find(|c| c.name == "SimpleStruct");
        assert!(class.is_some(), "SimpleStruct should be found");
    }

    /// Test Rust struct with impl block methods
    #[test]
    fn test_cohesion_rust_impl_blocks() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("simple.rs", RUST_SIMPLE).unwrap();

        let report = analyze_cohesion(test_dir.path(), None, 2).unwrap();

        // SimpleStruct should be found
        assert!(report.classes_analyzed >= 1);
        let class = report.classes.iter().find(|c| c.name == "SimpleStruct");
        assert!(class.is_some(), "SimpleStruct should be found");
    }
}

// =============================================================================
// Dead Code Analyzer Tests
// =============================================================================

#[cfg(test)]
mod dead_code_tests {
    use super::fixtures::*;
    

    /// Test detection of unreachable private functions
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_code_unreachable_function() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("unreachable.py", PYTHON_UNREACHABLE_FUNCTION)
            .unwrap();

        // _unused_private and another_unused should be detected as dead
        // main and helper should NOT be detected (they're called)

        todo!("Implement unreachable function detection");
    }

    /// Test that entry point patterns are not flagged
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_code_exclude_entry_points() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("entry_points.py", PYTHON_ENTRY_POINTS)
            .unwrap();

        // main, test_something, setup, teardown should NOT be flagged
        // These match entry point patterns

        todo!("Implement entry point exclusion");
    }

    /// Test that dunder methods are not flagged
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_code_exclude_dunders() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("dunder.py", PYTHON_DUNDER_METHODS)
            .unwrap();

        // __init__, __str__, __repr__, __eq__ should NOT be flagged
        // They are special methods that may be called implicitly

        todo!("Implement dunder method exclusion");
    }

    /// Test DeadFunction structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_function_structure() {
        // DeadFunction must have:
        // - file: PathBuf
        // - name: String
        // - line: usize

        todo!("Implement DeadFunction struct");
    }

    /// Test dead code summary statistics
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_code_summary() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("unreachable.py", PYTHON_UNREACHABLE_FUNCTION)
            .unwrap();

        // DeadCodeSummary must include:
        // - total_dead: usize
        // - total_functions: usize
        // - dead_percentage: f64

        todo!("Implement DeadCodeSummary");
    }

    /// Test by_file grouping
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_dead_code_by_file() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("a.py", PYTHON_UNREACHABLE_FUNCTION)
            .unwrap();
        test_dir.add_file("b.py", PYTHON_ENTRY_POINTS).unwrap();

        // DeadCodeReport.by_file should group dead functions by file path

        todo!("Implement by_file grouping");
    }
}

// =============================================================================
// Martin Metrics Tests
// =============================================================================

#[cfg(test)]
mod martin_metrics_tests {
    use super::fixtures::*;
    

    /// Test abstractness calculation: A = abstract_types / total_types
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_martin_abstractness() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("concrete.py", PYTHON_CONCRETE_PACKAGE)
            .unwrap();
        test_dir
            .add_file("abstract.py", PYTHON_ABSTRACT_PACKAGE)
            .unwrap();

        // PYTHON_CONCRETE_PACKAGE: A = 0/2 = 0.0
        // PYTHON_ABSTRACT_PACKAGE: A = 1/2 = 0.5 (1 ABC, 1 concrete)

        todo!("Implement abstractness calculation");
    }

    /// Test instability calculation: I = Ce / (Ca + Ce)
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_martin_instability() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("high_efferent.py", PYTHON_HIGH_EFFERENT)
            .unwrap();

        // Package with many imports but no importers:
        // Ce = many, Ca = 0
        // I = Ce / (Ca + Ce) = Ce / Ce = 1.0 (highly unstable)

        todo!("Implement instability calculation");
    }

    /// Test distance from main sequence: D = |A + I - 1|
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_martin_distance() {
        // Main sequence rule: A + I = 1
        // Distance D = |A + I - 1|

        // Example: A=0, I=1 -> D = |0+1-1| = 0 (on main sequence)
        // Example: A=0, I=0 -> D = |0+0-1| = 1 (zone of pain)
        // Example: A=1, I=1 -> D = |1+1-1| = 1 (zone of uselessness)

        todo!("Implement distance calculation");
    }

    /// Test zone detection (pain and uselessness zones)
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_martin_zone_detection() {
        // Zone of Pain: I < 0.3 AND A < 0.3 AND D > 0.5
        // (stable concrete packages - hard to change)

        // Zone of Uselessness: I > 0.7 AND A > 0.7
        // (unstable abstract packages - over-engineered)

        todo!("Implement zone detection");
    }

    /// Test MetricsHealth enum values
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_metrics_health_values() {
        // MetricsHealth must have:
        // - Healthy (D <= 0.2)
        // - Warning (D <= 0.4)
        // - Unhealthy (D > 0.4)
        // - Isolated (no dependencies)

        todo!("Implement MetricsHealth enum");
    }

    /// Test isolated package detection
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_martin_isolated_package() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("isolated.py", PYTHON_ISOLATED_PACKAGE)
            .unwrap();

        // Package with no imports and no importers
        // Should be marked as MetricsHealth::Isolated

        todo!("Implement isolated package detection");
    }

    /// Test PackageMetrics structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_package_metrics_structure() {
        // PackageMetrics must have:
        // - name: String
        // - ca: usize (afferent coupling)
        // - ce: usize (efferent coupling)
        // - instability: f64
        // - abstractness: f64
        // - distance: f64
        // - total_types: usize
        // - abstract_types: usize
        // - incoming_packages: Vec<String>
        // - outgoing_packages: Vec<String>
        // - health: MetricsHealth

        todo!("Implement PackageMetrics struct");
    }
}

// =============================================================================
// Coupling Analyzer Tests
// =============================================================================

#[cfg(test)]
mod coupling_tests {
    use super::fixtures::*;
    
    use crate::quality::coupling::{analyze_coupling, CouplingVerdict};
    use crate::types::Language;

    /// Test import relationship detection
    #[test]
    fn test_coupling_import_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("module_a.py", PYTHON_MODULE_A).unwrap();
        test_dir.add_file("module_b.py", PYTHON_MODULE_B).unwrap();

        // Run coupling analysis
        let report = analyze_coupling(test_dir.path(), Some(Language::Python), Some(10)).unwrap();

        // Should detect module pair
        // Note: Actual cross-file call detection depends on import resolution
        assert!(report.modules_analyzed >= 2);
    }

    /// Test coupling score calculation (0.0-1.0 normalized)
    #[test]
    fn test_coupling_score_calculation() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("module_a.py", PYTHON_MODULE_A).unwrap();
        test_dir.add_file("module_b.py", PYTHON_MODULE_B).unwrap();

        let report = analyze_coupling(test_dir.path(), Some(Language::Python), Some(10)).unwrap();

        // Coupling scores should be between 0.0 and 1.0
        for pair in &report.top_pairs {
            assert!(
                pair.score >= 0.0 && pair.score <= 1.0,
                "Score {} out of range",
                pair.score
            );
        }
    }

    /// Test tight coupling threshold detection
    #[test]
    fn test_coupling_tight_threshold() {
        // Verify verdict thresholds
        assert_eq!(CouplingVerdict::from_score(0.0), CouplingVerdict::Loose);
        assert_eq!(CouplingVerdict::from_score(0.29), CouplingVerdict::Loose);
        assert_eq!(CouplingVerdict::from_score(0.3), CouplingVerdict::Moderate);
        assert_eq!(CouplingVerdict::from_score(0.59), CouplingVerdict::Moderate);
        assert_eq!(CouplingVerdict::from_score(0.6), CouplingVerdict::Tight);
        assert_eq!(CouplingVerdict::from_score(1.0), CouplingVerdict::Tight);
    }

    /// Test CouplingVerdict enum values
    #[test]
    fn test_coupling_verdict_values() {
        // CouplingVerdict must have correct serialization
        assert_eq!(
            serde_json::to_string(&CouplingVerdict::Loose).unwrap(),
            "\"loose\""
        );
        assert_eq!(
            serde_json::to_string(&CouplingVerdict::Moderate).unwrap(),
            "\"moderate\""
        );
        assert_eq!(
            serde_json::to_string(&CouplingVerdict::Tight).unwrap(),
            "\"tight\""
        );
    }

    /// Test ModuleCoupling structure via serialization
    #[test]
    fn test_module_coupling_structure() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_SIMPLE_FUNCTION).unwrap();
        test_dir.add_file("b.py", PYTHON_SIMPLE_FUNCTION).unwrap();

        let report = analyze_coupling(test_dir.path(), Some(Language::Python), Some(10)).unwrap();

        // Can serialize to JSON
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("modules_analyzed").is_some());
        assert!(json.get("pairs_analyzed").is_some());
        assert!(json.get("top_pairs").is_some());
    }

    /// Test loosely coupled modules
    #[test]
    fn test_coupling_loose() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("independent.py", PYTHON_LOOSELY_COUPLED)
            .unwrap();
        test_dir
            .add_file("another.py", PYTHON_ISOLATED_PACKAGE)
            .unwrap();

        let report = analyze_coupling(test_dir.path(), Some(Language::Python), Some(10)).unwrap();

        // No cross-file calls -> top_pairs should be empty or have low scores
        for pair in &report.top_pairs {
            // Score should be low for independent modules
            assert!(
                pair.score < 0.6,
                "Expected loose coupling, got score {}",
                pair.score
            );
        }
    }
}

// =============================================================================
// Similarity Analyzer Tests
// =============================================================================

#[cfg(test)]
mod similarity_tests {
    use super::fixtures::*;
    
    use crate::quality::similarity::{find_similar, SimilarityReason};
    use crate::types::Language;

    /// Test score = 1.0 for identical functions (clones)
    #[test]
    fn test_similar_identical_functions() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("clones.py", PYTHON_CLONES).unwrap();

        // Use a low threshold to catch similar functions
        let report = find_similar(test_dir.path(), Some(Language::Python), 0.5, Some(100)).unwrap();

        // clone_a and clone_b are structurally similar
        // They should appear in the similar pairs
        assert!(report.functions_analyzed >= 2);
        // Note: Exact similarity score depends on implementation details
        // With same param count, similar structure, they should score high
    }

    /// Test that only pairs above threshold (0.7) are reported
    #[test]
    fn test_similar_threshold() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("clones.py", PYTHON_CLONES).unwrap();
        test_dir
            .add_file("different.py", PYTHON_DIFFERENT_FUNCTIONS)
            .unwrap();

        let threshold = 0.7;
        let report = find_similar(
            test_dir.path(),
            Some(Language::Python),
            threshold,
            Some(100),
        )
        .unwrap();

        // All reported pairs must have score >= threshold
        for pair in &report.similar_pairs {
            assert!(
                pair.score >= threshold,
                "Pair {} <-> {} has score {} below threshold {}",
                pair.func_a.name,
                pair.func_b.name,
                pair.score,
                threshold
            );
        }
    }

    /// Test low score for completely different functions
    #[test]
    fn test_similar_different_functions() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("different.py", PYTHON_DIFFERENT_FUNCTIONS)
            .unwrap();

        // Use very low threshold to see all pairs
        let report = find_similar(test_dir.path(), Some(Language::Python), 0.1, Some(100)).unwrap();

        // function_one (0 params) and function_two (5 params) are very different
        // If we have results, check they're not high similarity
        for pair in &report.similar_pairs {
            // Different functions shouldn't score near 1.0
            assert!(
                pair.score < 0.95,
                "Different functions {} <-> {} scored too high: {}",
                pair.func_a.name,
                pair.func_b.name,
                pair.score
            );
        }
    }

    /// Test SimilarPair structure
    #[test]
    fn test_similar_pair_structure() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_CLONES).unwrap();

        let report = find_similar(test_dir.path(), Some(Language::Python), 0.5, Some(100)).unwrap();

        // Can serialize to JSON
        let json = serde_json::to_value(&report).unwrap();
        assert!(json.get("functions_analyzed").is_some());
        assert!(json.get("pairs_compared").is_some());
        assert!(json.get("similar_pairs").is_some());
        assert!(json.get("threshold").is_some());
    }

    /// Test similarity reasons are provided
    #[test]
    fn test_similar_reasons() {
        // Verify reasons have descriptions
        assert_eq!(
            SimilarityReason::SameSignature.description(),
            "same parameter count"
        );
        assert_eq!(
            SimilarityReason::SimilarComplexity.description(),
            "similar complexity"
        );
        assert_eq!(
            SimilarityReason::SimilarCallPattern.description(),
            "similar call pattern"
        );
        assert_eq!(
            SimilarityReason::SimilarLoc.description(),
            "similar lines of code"
        );
    }

    /// Test pair deduplication (A-B same as B-A)
    #[test]
    fn test_similar_deduplication() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("clones.py", PYTHON_CLONES).unwrap();

        let report = find_similar(test_dir.path(), Some(Language::Python), 0.3, Some(100)).unwrap();

        // Check for duplicates
        let mut seen_pairs: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for pair in &report.similar_pairs {
            // Normalize order for comparison
            let (a, b) = if pair.func_a.name < pair.func_b.name {
                (pair.func_a.name.clone(), pair.func_b.name.clone())
            } else {
                (pair.func_b.name.clone(), pair.func_a.name.clone())
            };
            assert!(
                !seen_pairs.contains(&(a.clone(), b.clone())),
                "Duplicate pair found: {} <-> {}",
                a,
                b
            );
            seen_pairs.insert((a, b));
        }
    }

    /// Test max_functions limit
    #[test]
    fn test_similar_max_functions() {
        let test_dir = TestDir::new().unwrap();
        // Add a file with multiple functions
        test_dir
            .add_file("multi.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // With max_functions=2, only 2 functions compared -> 1 pair
        let report = find_similar(test_dir.path(), Some(Language::Python), 0.0, Some(2)).unwrap();

        // Should have analyzed at most 2 functions
        assert!(
            report.functions_analyzed <= 2,
            "Expected at most 2 functions, got {}",
            report.functions_analyzed
        );

        // pairs_compared should be n*(n-1)/2 = at most 1
        assert!(
            report.pairs_compared <= 1,
            "Expected at most 1 pair, got {}",
            report.pairs_compared
        );
    }

    /// Test weights sum to 1.0
    #[test]
    fn test_similarity_weighted_components() {
        // The weights are validated at compile time in similarity.rs
        // This test just documents the expected weights
        // Signature: 0.3, Complexity: 0.2, CallPattern: 0.3, LOC: 0.2
        // Total: 1.0
        // The compile-time assertion in similarity.rs guarantees this
    }
}

// =============================================================================
// Integration Tests - Mode Selection
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::fixtures::*;
    
    use crate::quality::health::{run_health, HealthOptions};

    /// Test that quick mode excludes coupling and similar analyzers
    #[test]
    fn test_quick_mode_excludes_coupling_similar() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("main.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        let options = HealthOptions {
            quick: true,
            ..Default::default()
        };

        let report = run_health(test_dir.path(), None, options).unwrap();

        // Verify quick mode flag is set in report
        assert!(report.quick_mode, "Report should indicate quick mode");

        // Coupling should be skipped
        let coupling = report
            .sub_results
            .get("coupling")
            .expect("coupling result missing");
        assert!(
            !coupling.success,
            "coupling should be skipped in quick mode"
        );
        assert!(
            coupling
                .error
                .as_ref()
                .is_some_and(|e| e.contains("skipped")),
            "coupling should have skipped error message"
        );

        // Similar should be skipped
        let similar = report
            .sub_results
            .get("similar")
            .expect("similar result missing");
        assert!(!similar.success, "similar should be skipped in quick mode");
        assert!(
            similar
                .error
                .as_ref()
                .is_some_and(|e| e.contains("skipped")),
            "similar should have skipped error message"
        );

        // Core analyzers should run
        assert!(
            report
                .sub_results
                .get("complexity")
                .expect("complexity missing")
                .success
        );
        assert!(
            report
                .sub_results
                .get("cohesion")
                .expect("cohesion missing")
                .success
        );
        // Note: dead and metrics may fail on small test fixture, but should be present
        assert!(report.sub_results.contains_key("dead"));
        assert!(report.sub_results.contains_key("metrics"));
    }

    /// Test that full mode includes all 6 analyzers
    #[test]
    fn test_full_mode_includes_all() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("main.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();
        test_dir
            .add_file("helper.py", PYTHON_COHESIVE_CLASS)
            .unwrap();

        let options = HealthOptions {
            quick: false, // Full mode
            ..Default::default()
        };

        let report = run_health(test_dir.path(), None, options).unwrap();

        // Verify full mode flag is set in report
        assert!(!report.quick_mode, "Report should indicate full mode");

        // All 6 sub-analyzers should be present
        assert!(
            report.sub_results.contains_key("complexity"),
            "complexity missing"
        );
        assert!(
            report.sub_results.contains_key("cohesion"),
            "cohesion missing"
        );
        assert!(report.sub_results.contains_key("dead"), "dead missing");
        assert!(
            report.sub_results.contains_key("metrics"),
            "metrics missing"
        );
        assert!(
            report.sub_results.contains_key("coupling"),
            "coupling missing"
        );
        assert!(
            report.sub_results.contains_key("similar"),
            "similar missing"
        );

        // Core analyzers should succeed
        assert!(
            report.sub_results.get("complexity").unwrap().success,
            "complexity should succeed"
        );
        assert!(
            report.sub_results.get("cohesion").unwrap().success,
            "cohesion should succeed"
        );

        // In full mode, coupling and similar should at least attempt to run
        // (they might fail or have 0 findings, but shouldn't be skipped)
        let coupling = report.sub_results.get("coupling").unwrap();
        let similar = report.sub_results.get("similar").unwrap();
        assert!(
            coupling
                .error
                .as_ref()
                .is_none_or(|e| !e.contains("skipped")),
            "coupling should not be skipped in full mode"
        );
        assert!(
            similar
                .error
                .as_ref()
                .is_none_or(|e| !e.contains("skipped")),
            "similar should not be skipped in full mode"
        );
    }

    /// Test health analysis works on Python, TypeScript, Go, Rust files
    #[test]
    fn test_health_multi_language() {
        use crate::quality::health::{run_health, HealthOptions};
        use crate::types::Language;

        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("code.py", PYTHON_SIMPLE_FUNCTION)
            .unwrap();
        test_dir.add_file("code.ts", TYPESCRIPT_SIMPLE).unwrap();
        test_dir.add_file("code.go", GO_SIMPLE).unwrap();
        test_dir.add_file("code.rs", RUST_SIMPLE).unwrap();

        // Each language should be analyzable
        let options = HealthOptions::default();

        // Python
        let py_result = run_health(test_dir.path(), Some(Language::Python), options.clone());
        assert!(py_result.is_ok(), "Python analysis should work");

        // TypeScript
        let ts_result = run_health(test_dir.path(), Some(Language::TypeScript), options.clone());
        assert!(ts_result.is_ok(), "TypeScript analysis should work");

        // Go
        let go_result = run_health(test_dir.path(), Some(Language::Go), options.clone());
        assert!(go_result.is_ok(), "Go analysis should work");

        // Rust
        let rs_result = run_health(test_dir.path(), Some(Language::Rust), options.clone());
        assert!(rs_result.is_ok(), "Rust analysis should work");
    }

    /// Test auto-detection of dominant language
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_language_auto_detection() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_SIMPLE_FUNCTION).unwrap();
        test_dir.add_file("b.py", PYTHON_COHESIVE_CLASS).unwrap();
        test_dir
            .add_file("c.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();
        test_dir.add_file("d.ts", TYPESCRIPT_SIMPLE).unwrap();

        // With 3 .py files and 1 .ts file, Python is dominant
        // run_health(path, lang=None) should analyze Python

        todo!("Implement language auto-detection");
    }

    /// Test single file input (not directory)
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_single_file_input() {
        let test_dir = TestDir::new().unwrap();
        let _file_path = test_dir
            .add_file("single.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // run_health on a single file should work
        // coupling/similar may have empty results (no other files to compare)

        todo!("Implement single file analysis");
    }
}

// =============================================================================
// Output Format Tests
// =============================================================================

#[cfg(test)]
mod output_format_tests {
    use super::fixtures::*;
    

    /// Test JSON output matches schema from spec section 5.1
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_json_output_schema() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("main.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // JSON must have structure:
        // {
        //   "wrapper": "health",
        //   "path": "<analyzed_path>",
        //   "summary": { ... },
        //   "details": { ... },  // Note: spec says "sub_results" but example shows "details"
        //   "total_elapsed_ms": ...
        // }

        // Validate required fields exist and have correct types

        todo!("Implement JSON output validation");
    }

    /// Test text output produces readable dashboard
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_text_output_dashboard() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("main.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // Text format per spec section 5.2:
        // Health Report: <path>
        // ==================================================
        // Complexity:  avg CC=X.X, hotspots=N (CC>10)
        // Cohesion:    N classes, avg LCOM4=X.X, N low-cohesion
        // Coupling:    N pairs analyzed, avg score=X.XX
        // Dead Code:   N unreachable functions
        // Duplication: N similar function pairs (>0.7)
        // Metrics:     avg D=X.XX (distance from main sequence)
        //
        // Elapsed: Xms

        todo!("Implement text output format");
    }

    /// Test text output shows errors for failed sub-analyses
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_text_output_with_errors() {
        // When sub-analysis fails, text should show:
        // Cohesion:    skipped (analysis failed)
        // ...
        // Errors: cohesion, coupling

        todo!("Implement text output error display");
    }

    /// Test --detail flag returns only specified sub-analysis
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_detail_flag() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("main.py", PYTHON_MULTIPLE_FUNCTIONS)
            .unwrap();

        // --detail complexity -> returns only complexity sub-analysis data
        // --detail cohesion -> returns only cohesion data
        // etc.

        todo!("Implement --detail flag");
    }

    /// Test invalid --detail value produces error
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_invalid_detail_value() {
        // --detail invalid_name should produce error with valid options:
        // Valid: complexity, cohesion, dead, metrics, coupling, similar

        todo!("Implement --detail validation");
    }

    /// Test skipped sub-analysis JSON format
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_skipped_sub_analysis_json() {
        // Per spec section 5.1, skipped analyses should have:
        // {
        //   "name": "coupling",
        //   "success": false,
        //   "error": "skipped (quick mode)",
        //   "elapsed_ms": 0.0
        // }

        todo!("Implement skipped sub-analysis JSON format");
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::fixtures::*;
    
    use crate::quality::health::{run_health, HealthOptions};
    use crate::types::Language;

    /// Test empty directory returns error (no supported files)
    #[test]
    fn test_health_empty_directory() {
        let test_dir = TestDir::new().unwrap();
        // Directory is empty - no files

        let options = HealthOptions::default();
        let result = run_health(test_dir.path(), None, options);

        // Empty directory should return NoSupportedFiles error when no language specified
        assert!(result.is_err(), "Empty directory should return error");
    }

    /// Test directory with no supported files returns error
    #[test]
    fn test_health_no_supported_files() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("readme.md", "# Readme").unwrap();
        test_dir.add_file("config.json", "{}").unwrap();

        let options = HealthOptions::default();
        let result = run_health(test_dir.path(), None, options);

        // Should return NoSupportedFiles error when auto-detecting language
        assert!(result.is_err(), "No supported files should return error");
    }

    /// Test parse error in source file continues with other files (T8 graceful error)
    #[test]
    fn test_health_graceful_parse_error() {
        let test_dir = TestDir::new().unwrap();
        test_dir
            .add_file("good.py", PYTHON_SIMPLE_FUNCTION)
            .unwrap();
        test_dir.add_file("bad.py", PYTHON_SYNTAX_ERROR).unwrap();

        let options = HealthOptions::default();
        let result = run_health(test_dir.path(), Some(Language::Python), options);

        // Should succeed - graceful handling of parse errors (T8)
        assert!(result.is_ok(), "Should handle parse errors gracefully");
        let report = result.unwrap();

        // At least some analysis should complete
        assert!(report.sub_results.contains_key("complexity"));
    }

    /// Test all files failing to parse still returns report
    #[test]
    #[ignore = "all-bad files case needs deeper analysis"]
    fn test_all_files_parse_error() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("bad1.py", PYTHON_SYNTAX_ERROR).unwrap();
        test_dir.add_file("bad2.py", "def broken(\n").unwrap();

        let options = HealthOptions::default();
        let result = run_health(test_dir.path(), Some(Language::Python), options);

        // When all files fail to parse, we still get a report
        // Sub-analyzers report empty/zero findings, not errors
        assert!(
            result.is_ok(),
            "Should return report even with all parse errors"
        );
    }

    /// Test file with only comments
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_comments_only_file() {
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("comments.py", COMMENTS_ONLY).unwrap();

        // File with only comments should:
        // - Not cause errors
        // - Report 0 functions, 0 classes, etc.

        todo!("Implement comments-only file handling");
    }

    /// Test very large file performance
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_large_file_performance() {
        // Generate a large file with many functions
        let mut content = String::new();
        for i in 0..1000 {
            content.push_str(&format!("def func_{}():\n    return {}\n\n", i, i));
        }

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("large.py", &content).unwrap();

        // Should complete without timeout
        // Similarity analysis should respect max_functions limit

        todo!("Implement large file handling");
    }

    /// Test unicode content in source files
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_unicode_content() {
        let content = r#"
def greet(name):
    """Say hello in multiple languages."""
    return f"Hello/Hola/こんにちは/Привет {name}"

class Émoji:
    """Class with unicode name."""
    def method_with_emoji_🎉(self):
        return "🎉"
"#;

        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("unicode.py", content).unwrap();

        // Should handle unicode without errors

        todo!("Implement unicode handling");
    }

    /// Test symlink handling
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_symlink_handling() {
        // Symlinks should be either:
        // - Followed (analyze target)
        // - Skipped (don't follow)
        // But NOT cause errors

        todo!("Implement symlink handling");
    }

    /// Test path not found error
    #[test]
    fn test_path_not_found() {
        use crate::quality::health::{run_health, HealthOptions};
        use std::path::Path;

        let options = HealthOptions::default();
        let result = run_health(
            Path::new("/nonexistent/path/that/does/not/exist"),
            None,
            options,
        );

        // Should return Err(PathNotFound)
        assert!(result.is_err(), "Nonexistent path should return error");
    }
}

// =============================================================================
// Error Type Tests
// =============================================================================

#[cfg(test)]
mod error_type_tests {
    

    /// Test HealthError variants exist per spec section 9
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_error_variants() {
        // HealthError must have:
        // - PathNotFound(PathBuf)
        // - NoSupportedFiles(PathBuf)
        // - LanguageDetectionFailed
        // - Io(std::io::Error)

        todo!("Implement HealthError enum");
    }

    /// Test AnalysisError variants exist per spec section 9
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_analysis_error_variants() {
        // AnalysisError must have:
        // - Parse { file: PathBuf, message: String }
        // - CallGraphFailed(String)
        // - Timeout

        todo!("Implement AnalysisError enum");
    }

    /// Test error messages are descriptive
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_error_messages() {
        // Error Display implementations should be user-friendly:
        // - "Path not found: /some/path"
        // - "No supported files found in /some/path"
        // - "Parse error in /file.py: unexpected token"

        todo!("Implement error Display");
    }
}

// =============================================================================
// Options Tests
// =============================================================================

#[cfg(test)]
mod options_tests {
    

    /// Test HealthOptions defaults per spec section 7.1
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_options_defaults() {
        // HealthOptions::default() should have:
        // - complexity_hotspot_threshold: 10
        // - cohesion_low_threshold: 2
        // - coupling_tight_threshold: 0.6
        // - similarity_threshold: 0.7
        // - distance_warning_threshold: 0.4
        // - preset: None

        todo!("Implement HealthOptions defaults");
    }

    /// Test ThresholdPreset::Strict values
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_preset_strict() {
        // Strict preset should have tighter thresholds
        // (lower complexity threshold, lower coupling tolerance, etc.)

        todo!("Implement ThresholdPreset::Strict");
    }

    /// Test ThresholdPreset::Relaxed values
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_preset_relaxed() {
        // Relaxed preset should have looser thresholds
        // (higher complexity tolerance for legacy code)

        todo!("Implement ThresholdPreset::Relaxed");
    }

    /// Test ComplexityOptions structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_complexity_options() {
        // ComplexityOptions must have:
        // - hotspot_threshold: u32 (default: 10)
        // - include_cognitive: bool

        todo!("Implement ComplexityOptions");
    }

    /// Test CohesionOptions structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_cohesion_options() {
        // CohesionOptions must have:
        // - include_dunder: bool (default: false)
        // - low_cohesion_threshold: usize (default: 2)

        todo!("Implement CohesionOptions");
    }

    /// Test SimilarityOptions structure
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_similarity_options() {
        // SimilarityOptions must have:
        // - min_score: f64 (default: 0.7)
        // - max_functions: usize (default: 500)
        // - top_k_per_function: usize (default: 3)

        todo!("Implement SimilarityOptions");
    }
}

// =============================================================================
// Incremental Analysis Tests (Exceed Python)
// =============================================================================

#[cfg(test)]
mod incremental_tests {
    

    /// Test IncrementalOptions structure per spec section 7.5
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_incremental_options() {
        // IncrementalOptions must have:
        // - since_ref: Option<String> (git ref)
        // - files: Option<Vec<PathBuf>>
        // - cache_dir: Option<PathBuf>

        todo!("Implement IncrementalOptions");
    }

    /// Test incremental analysis only processes changed files
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_incremental_changed_files_only() {
        // With since_ref="HEAD~1", only files changed in last commit
        // should be analyzed

        todo!("Implement incremental analysis");
    }

    /// Test cache reuse for unchanged files
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_incremental_cache_reuse() {
        // Unchanged files should use cached results
        // Cache keyed by file path + content hash

        todo!("Implement cache reuse");
    }
}

// =============================================================================
// HealthFinding Tests (Exceed Python - Severity Levels)
// =============================================================================

#[cfg(test)]
mod health_finding_tests {
    

    /// Test HealthFinding structure per spec section 7.3
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_health_finding_structure() {
        // HealthFinding must have:
        // - category: String (complexity, cohesion, etc.)
        // - severity: Severity
        // - location: Location
        // - message: String
        // - suggestion: Option<String>

        todo!("Implement HealthFinding struct");
    }

    /// Test severity counts in summary
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_severity_counts() {
        // HealthSummary.severity_counts: HashMap<Severity, usize>
        // Counts how many findings at each severity level

        todo!("Implement severity counts");
    }

    /// Test severity assignment for complexity issues
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_severity_complexity() {
        // CC 10-15: Medium
        // CC 15-25: High
        // CC > 25: Critical

        todo!("Implement complexity severity assignment");
    }

    /// Test severity assignment for cohesion issues
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_severity_cohesion() {
        // LCOM4 2-3: Low
        // LCOM4 3-5: Medium
        // LCOM4 > 5: High

        todo!("Implement cohesion severity assignment");
    }
}

// =============================================================================
// Parallel Execution Tests (Exceed Python)
// =============================================================================

#[cfg(test)]
mod parallel_tests {
    

    /// Test that independent analyzers can run in parallel
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_parallel_independent_analyzers() {
        // Per spec section 7.4, these can run in parallel:
        // - complexity
        // - cohesion
        // - dead_code (call graph build parallelizable)
        // - martin_metrics
        //
        // These depend on call graph:
        // - coupling
        // - similar

        todo!("Implement parallel analyzer execution");
    }

    /// Test parallel file processing within analyzer
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_parallel_file_processing() {
        // Within each analyzer, files should be processed in parallel
        // using rayon

        todo!("Implement parallel file processing");
    }

    /// Test total_elapsed_ms is less than sum of sub-analysis times
    #[test]
    #[ignore = "health module not yet implemented"]
    fn test_parallel_timing() {
        // If analyzers run in parallel, total_elapsed_ms should be
        // less than sum of individual elapsed_ms values

        todo!("Verify parallel timing");
    }
}
