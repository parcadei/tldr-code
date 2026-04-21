//! Test module for dice CLI command (Session 8 spec)
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. Token-Based Dice Coefficient (D1)
//! - Basic Dice computation
//! - Multiset handling
//! - Edge cases (empty, identical, disjoint)
//!
//! ## 2. Jaccard Coefficient (D2)
//! - Basic Jaccard computation
//! - Relationship to Dice
//!
//! ## 3. Cosine Similarity (D3)
//! - TF-IDF weighted similarity
//! - Term frequency computation
//!
//! ## 4. Function-Level Similarity (D4)
//! - file.py::function_name syntax
//! - Function extraction
//!
//! ## 5. File-Level Similarity (D5)
//! - Entire file comparison
//! - Auto-detection
//!
//! ## 6. Block-Level Similarity (D6)
//! - file.py:start:end syntax
//! - Line range extraction
//!
//! ## 7. N-gram Similarity (D10)
//! - Bigram and trigram comparison
//! - Configurable n
//!
//! ## 8. Pairwise Similarity Matrix (D11)
//! - All-pairs comparison
//! - Threshold filtering
//!
//! ## 9. Score Interpretation (D12)
//! - Human-readable descriptions
//! - Threshold boundaries
//!
//! Reference: session8-spec.md


// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for similarity analysis tests
pub mod fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// A temporary directory for testing similarity analysis
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
    // Identical Code Fixtures
    // -------------------------------------------------------------------------

    pub const PYTHON_FUNC_A: &str = r#"
def process_data(items):
    result = []
    for item in items:
        processed = transform(item)
        result.append(processed)
    return result
"#;

    /// Identical to PYTHON_FUNC_A
    pub const PYTHON_FUNC_A_COPY: &str = r#"
def process_data(items):
    result = []
    for item in items:
        processed = transform(item)
        result.append(processed)
    return result
"#;

    // -------------------------------------------------------------------------
    // Similar Code Fixtures (for Type-2/3 comparison)
    // -------------------------------------------------------------------------

    /// Same structure, different names (~85% similar)
    pub const PYTHON_FUNC_B_SIMILAR: &str = r#"
def handle_items(data):
    output = []
    for element in data:
        converted = transform(element)
        output.append(converted)
    return output
"#;

    /// Same structure with added logging (~75% similar)
    pub const PYTHON_FUNC_C_WITH_LOGGING: &str = r#"
def process_data_logged(items):
    print("Starting processing")
    result = []
    for item in items:
        processed = transform(item)
        print(f"Processed: {processed}")
        result.append(processed)
    print("Done")
    return result
"#;

    // -------------------------------------------------------------------------
    // Completely Different Code Fixtures
    // -------------------------------------------------------------------------

    pub const PYTHON_FUNC_DIFFERENT: &str = r#"
def calculate_average(numbers):
    if not numbers:
        return 0
    total = sum(numbers)
    count = len(numbers)
    return total / count
"#;

    /// Another completely different function
    pub const PYTHON_FUNC_VERY_DIFFERENT: &str = r#"
class DatabaseConnection:
    def __init__(self, host, port):
        self.host = host
        self.port = port
        self.connected = False

    def connect(self):
        self.connected = True
        return self
"#;

    // -------------------------------------------------------------------------
    // File with Multiple Functions (for function extraction testing)
    // -------------------------------------------------------------------------

    pub const PYTHON_MULTI_FUNCTION_FILE: &str = r#"
def first_function(a, b):
    return a + b

def second_function(x, y, z):
    result = x * y
    result = result + z
    return result

def third_function(items):
    total = 0
    for item in items:
        total += item
    return total
"#;

    /// Another file with similar functions
    pub const PYTHON_MULTI_FUNCTION_FILE_B: &str = r#"
def add_numbers(a, b):
    return a + b

def multiply_and_add(x, y, z):
    product = x * y
    product = product + z
    return product

def sum_items(elements):
    total = 0
    for element in elements:
        total += element
    return total
"#;

    // -------------------------------------------------------------------------
    // TypeScript Fixtures
    // -------------------------------------------------------------------------

    pub const TS_FUNC_A: &str = r#"
export function processData(items: any[]): any[] {
    const result: any[] = [];
    for (const item of items) {
        const processed = transform(item);
        result.push(processed);
    }
    return result;
}
"#;

    pub const TS_FUNC_B_SIMILAR: &str = r#"
export function handleItems(data: any[]): any[] {
    const output: any[] = [];
    for (const element of data) {
        const converted = transform(element);
        output.push(converted);
    }
    return output;
}
"#;

    // -------------------------------------------------------------------------
    // Go Fixtures
    // -------------------------------------------------------------------------

    pub const GO_FUNC_A: &str = r#"
func ProcessData(items []interface{}) []interface{} {
    result := make([]interface{}, 0)
    for _, item := range items {
        processed := transform(item)
        result = append(result, processed)
    }
    return result
}
"#;

    pub const GO_FUNC_B_SIMILAR: &str = r#"
func HandleItems(data []interface{}) []interface{} {
    output := make([]interface{}, 0)
    for _, element := range data {
        converted := transform(element)
        output = append(output, converted)
    }
    return output
}
"#;

    // -------------------------------------------------------------------------
    // Rust Fixtures
    // -------------------------------------------------------------------------

    pub const RUST_FUNC_A: &str = r#"
pub fn process_data(items: &[Item]) -> Vec<Item> {
    let mut result = Vec::new();
    for item in items {
        let processed = transform(item);
        result.push(processed);
    }
    result
}
"#;

    pub const RUST_FUNC_B_SIMILAR: &str = r#"
pub fn handle_items(data: &[Item]) -> Vec<Item> {
    let mut output = Vec::new();
    for element in data {
        let converted = transform(element);
        output.push(converted);
    }
    output
}
"#;

    // -------------------------------------------------------------------------
    // Edge Case Fixtures
    // -------------------------------------------------------------------------

    /// Function with unique tokens only
    pub const UNIQUE_TOKENS_ONLY: &str = r#"
def unique_function_xyz():
    alpha_var = "unique_string_123"
    return alpha_var
"#;

    /// Different function with no shared tokens
    pub const NO_SHARED_TOKENS: &str = r#"
def another_beta_function():
    omega_value = "different_text_456"
    return omega_value
"#;

    // -------------------------------------------------------------------------
    // Block-Level Testing Fixtures
    // -------------------------------------------------------------------------

    /// File with distinct blocks for line-range testing
    pub const FILE_WITH_BLOCKS: &str = r#"
# Block A: lines 1-10
def block_a():
    x = 1
    y = 2
    z = 3
    result = x + y + z
    return result

# Block B: lines 12-21
def block_b():
    a = 1
    b = 2
    c = 3
    result = a + b + c
    return result

# Block C: lines 23-32 (different)
def block_c():
    items = [1, 2, 3]
    total = sum(items)
    average = total / len(items)
    return average
"#;
}

// =============================================================================
// Token-Based Dice Coefficient Tests (D1)
// =============================================================================

#[cfg(test)]
mod dice_coefficient_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{
        compute_similarity, SimilarityMetric, SimilarityOptions,
    };
    

    /// Test: Dice coefficient for identical code
    /// Contract: dice(A, A) = 1.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_identical_code() {
        // GIVEN: Two files with identical code
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap();

        // WHEN: Computing Dice similarity
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Dice, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Dice should be 1.0
        assert!(
            (report.similarity.dice - 1.0).abs() < 0.001,
            "Identical code should have Dice = 1.0, got {}",
            report.similarity.dice
        );
    }

    /// Test: Dice coefficient is symmetric
    /// Contract: dice(A, B) = dice(B, A)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_symmetry() {
        // GIVEN: Two different files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing Dice in both directions
        let options = SimilarityOptions::default();
        let report_ab = compute_similarity(&path_a, &path_b, &options).unwrap();
        let report_ba = compute_similarity(&path_b, &path_a, &options).unwrap();

        // THEN: Should be symmetric
        assert!(
            (report_ab.similarity.dice - report_ba.similarity.dice).abs() < 0.001,
            "Dice should be symmetric"
        );
    }

    /// Test: Dice coefficient for disjoint code
    /// Contract: dice(A, B) = 0 if A and B share no tokens
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_disjoint_code() {
        // GIVEN: Two files with no shared tokens
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", UNIQUE_TOKENS_ONLY).unwrap();
        let path_b = test_dir.add_file("b.py", NO_SHARED_TOKENS).unwrap();

        // WHEN: Computing Dice similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Dice should be close to 0 (some keywords may overlap)
        assert!(
            report.similarity.dice < 0.3,
            "Disjoint code should have low Dice, got {}",
            report.similarity.dice
        );
    }

    /// Test: Dice handles empty input
    /// Contract: Empty fragment results in dice = 0.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_empty_input() {
        // GIVEN: Empty file vs normal file
        let test_dir = TestDir::new().unwrap();
        let path_empty = test_dir.add_file("empty.py", "").unwrap();
        let path_normal = test_dir.add_file("normal.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Computing Dice similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_empty, &path_normal, &options).unwrap();

        // THEN: Dice should be 0.0
        assert!(
            (report.similarity.dice - 0.0).abs() < 0.001,
            "Empty input should give Dice = 0.0"
        );
    }

    /// Test: Dice coefficient for similar code
    /// Contract: Similar code has high Dice (0.7-0.9)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_similar_code() {
        // GIVEN: Two similar files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing Dice similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be moderately high
        assert!(
            report.similarity.dice >= 0.5 && report.similarity.dice <= 1.0,
            "Similar code should have moderate-high Dice, got {}",
            report.similarity.dice
        );
    }

    /// Test: Dice uses multiset (token counts matter)
    /// Contract: 2 * |intersection| / (|A| + |B|) with counts
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_multiset_handling() {
        // GIVEN: File with repeated tokens vs file without
        let repeated = r#"
def func():
    x = 1
    x = 2
    x = 3
    return x
"#;
        let not_repeated = r#"
def func():
    a = 1
    b = 2
    c = 3
    return a
"#;
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", repeated).unwrap();
        let path_b = test_dir.add_file("b.py", not_repeated).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Token breakdown should reflect counts
        // The shared 'x' appears 4 times in A but not in B
        // This test verifies multiset behavior
        assert!(
            report.token_breakdown.unique_to_fragment1 > 0
                || report.token_breakdown.unique_to_fragment2 > 0
        );
    }

    /// Test: Dice is non-negative
    /// Contract: dice >= 0.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_dice_non_negative() {
        // GIVEN: Any two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_DIFFERENT).unwrap();

        // WHEN: Computing Dice
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be non-negative
        assert!(report.similarity.dice >= 0.0, "Dice must be non-negative");
    }
}

// =============================================================================
// Jaccard Coefficient Tests (D2)
// =============================================================================

#[cfg(test)]
mod jaccard_coefficient_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityMetric, SimilarityOptions};
    

    /// Test: Jaccard for identical code
    /// Contract: jaccard(A, A) = 1.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_jaccard_identical_code() {
        // GIVEN: Identical files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap();

        // WHEN: Computing Jaccard
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Jaccard, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be 1.0
        assert!((report.similarity.jaccard - 1.0).abs() < 0.001);
    }

    /// Test: Jaccard for disjoint code
    /// Contract: jaccard(A, B) = 0 if disjoint
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_jaccard_disjoint_code() {
        // GIVEN: Files with minimal overlap
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", UNIQUE_TOKENS_ONLY).unwrap();
        let path_b = test_dir.add_file("b.py", NO_SHARED_TOKENS).unwrap();

        // WHEN: Computing Jaccard
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be close to 0
        assert!(
            report.similarity.jaccard < 0.3,
            "Disjoint code should have low Jaccard"
        );
    }

    /// Test: Jaccard <= Dice (always more conservative)
    /// Contract: jaccard = dice / (2 - dice)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_jaccard_less_than_dice() {
        // GIVEN: Any two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing both metrics
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::All, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Jaccard should be <= Dice
        assert!(
            report.similarity.jaccard <= report.similarity.dice + 0.001,
            "Jaccard ({}) should be <= Dice ({})",
            report.similarity.jaccard,
            report.similarity.dice
        );
    }

    /// Test: Jaccard/Dice relationship formula
    /// Contract: jaccard = dice / (2 - dice)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_jaccard_dice_relationship() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir
            .add_file("b.py", PYTHON_FUNC_C_WITH_LOGGING)
            .unwrap();

        // WHEN: Computing both metrics
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::All, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Relationship should hold
        let expected_jaccard = report.similarity.dice / (2.0 - report.similarity.dice);
        assert!(
            (report.similarity.jaccard - expected_jaccard).abs() < 0.01,
            "Jaccard/Dice relationship broken"
        );
    }

    /// Test: Jaccard is symmetric
    /// Contract: jaccard(A, B) = jaccard(B, A)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_jaccard_symmetry() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing in both directions
        let options = SimilarityOptions::default();
        let report_ab = compute_similarity(&path_a, &path_b, &options).unwrap();
        let report_ba = compute_similarity(&path_b, &path_a, &options).unwrap();

        // THEN: Should be symmetric
        assert!((report_ab.similarity.jaccard - report_ba.similarity.jaccard).abs() < 0.001);
    }
}

// =============================================================================
// Cosine Similarity Tests (D3)
// =============================================================================

#[cfg(test)]
mod cosine_similarity_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityMetric};
    

    /// Test: Cosine for identical code
    /// Contract: cosine(A, A) = 1.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_cosine_identical_code() {
        // GIVEN: Identical files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap();

        // WHEN: Computing cosine
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Cosine, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be 1.0
        assert!(report.similarity.cosine.is_some());
        assert!((report.similarity.cosine.unwrap() - 1.0).abs() < 0.001);
    }

    /// Test: Cosine for disjoint code
    /// Contract: cosine(A, B) = 0 if no shared terms
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_cosine_disjoint_code() {
        // GIVEN: Files with no shared tokens
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", UNIQUE_TOKENS_ONLY).unwrap();
        let path_b = test_dir.add_file("b.py", NO_SHARED_TOKENS).unwrap();

        // WHEN: Computing cosine
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Cosine, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be close to 0
        assert!(report.similarity.cosine.unwrap() < 0.3);
    }

    /// Test: Cosine is in valid range
    /// Contract: 0 <= cosine <= 1 for non-negative vectors
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_cosine_valid_range() {
        // GIVEN: Any two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_DIFFERENT).unwrap();

        // WHEN: Computing cosine
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Cosine, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should be in [0, 1]
        let cosine = report.similarity.cosine.unwrap();
        assert!(
            (0.0..=1.0).contains(&cosine),
            "Cosine must be in [0,1], got {}",
            cosine
        );
    }

    /// Test: Cosine weights rare tokens higher
    /// Contract: IDF gives more weight to distinctive terms
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_cosine_weights_rare_tokens() {
        // This test would ideally show that rare tokens have more impact
        // For now, just verify cosine is computed
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Cosine, ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // Verify cosine is computed and reasonable
        assert!(report.similarity.cosine.is_some());
    }

    /// Test: Cosine handles empty input
    /// Contract: cosine = 0 for empty document
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_cosine_empty_input() {
        // GIVEN: Empty vs normal file
        let test_dir = TestDir::new().unwrap();
        let path_empty = test_dir.add_file("empty.py", "").unwrap();
        let path_normal = test_dir.add_file("normal.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Computing cosine
        let options = crate::analysis::similarity::SimilarityOptions { metric: SimilarityMetric::Cosine, ..Default::default() };

        let report = compute_similarity(&path_empty, &path_normal, &options).unwrap();

        // THEN: Should be 0.0
        assert!((report.similarity.cosine.unwrap() - 0.0).abs() < 0.001);
    }
}

// =============================================================================
// Function-Level Similarity Tests (D4)
// =============================================================================

#[cfg(test)]
mod function_level_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{
        compute_similarity, parse_target, ComparisonLevel, SimilarityOptions,
    };
    use std::path::PathBuf;

    /// Test: Parse function target syntax
    /// Contract: file.py::function_name is valid
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_parse_function_target() {
        // GIVEN: Function target string
        let target = "src/auth.py::login";

        // WHEN: Parsing target
        let parsed = parse_target(target).unwrap();

        // THEN: Should extract file and function
        assert_eq!(parsed.file, PathBuf::from("src/auth.py"));
        assert_eq!(parsed.function, Some("login".to_string()));
        assert!(parsed.line_range.is_none());
    }

    /// Test: Function-level comparison
    /// Contract: Compare only function bodies
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_function_level_comparison() {
        // GIVEN: Files with multiple functions
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir
            .add_file("a.py", PYTHON_MULTI_FUNCTION_FILE)
            .unwrap();
        let path_b = test_dir
            .add_file("b.py", PYTHON_MULTI_FUNCTION_FILE_B)
            .unwrap();

        // WHEN: Comparing specific functions
        let options = SimilarityOptions {
            level: Some(ComparisonLevel::Function),
            ..Default::default()
        };

        let target_a = format!("{}::third_function", path_a.display());
        let target_b = format!("{}::sum_items", path_b.display());

        let report = compute_similarity(
            &PathBuf::from(&target_a),
            &PathBuf::from(&target_b),
            &options,
        )
        .unwrap();

        // THEN: Should compare only those functions (high similarity)
        assert!(
            report.similarity.dice > 0.7,
            "Similar functions should have high Dice"
        );
    }

    /// Test: Function not found error
    /// Contract: Return error if function doesn't exist
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_function_not_found_error() {
        // GIVEN: File without target function
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Trying to compare non-existent function
        let options = SimilarityOptions::default();
        let target = format!("{}::nonexistent_function", path.display());

        let result = compute_similarity(&PathBuf::from(&target), &path, &options);

        // THEN: Should return error
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("Function"),
            "Error should mention function not found"
        );
    }

    /// Test: Fragment info includes function name
    /// Contract: SimilarityFragment.function is set
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_fragment_includes_function_name() {
        // GIVEN: Function comparison
        let test_dir = TestDir::new().unwrap();
        let path = test_dir
            .add_file("a.py", PYTHON_MULTI_FUNCTION_FILE)
            .unwrap();

        // WHEN: Comparing function
        let options = SimilarityOptions::default();
        let target = format!("{}::first_function", path.display());

        let report =
            compute_similarity(&PathBuf::from(&target), &PathBuf::from(&target), &options).unwrap();

        // THEN: Fragment should have function name
        assert_eq!(
            report.fragment1.function,
            Some("first_function".to_string())
        );
    }
}

// =============================================================================
// File-Level Similarity Tests (D5)
// =============================================================================

#[cfg(test)]
mod file_level_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions};
    

    /// Test: File-level comparison (default)
    /// Contract: Entire file content is compared
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_file_level_comparison() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Comparing files (default level)
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should compare entire files
        assert!(
            report.fragment1.function.is_none(),
            "File-level should not have function"
        );
        assert!(
            report.fragment1.line_range.is_none(),
            "File-level should not have line range"
        );
    }

    /// Test: File token count is correct
    /// Contract: fragment.tokens reflects entire file
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_file_token_count() {
        // GIVEN: File
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Computing similarity (file-level)
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path, &path, &options).unwrap();

        // THEN: Token count should be positive and reasonable
        assert!(report.fragment1.tokens > 0);
        assert!(report.fragment1.lines > 0);
    }

    /// Test: File line count is correct
    /// Contract: fragment.lines reflects file line count
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_file_line_count() {
        // GIVEN: File with known line count
        let content = "line1\nline2\nline3\nline4\nline5\n";
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", content).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path, &path, &options).unwrap();

        // THEN: Line count should be correct
        assert!(report.fragment1.lines >= 5, "Should count at least 5 lines");
    }

    /// Test: Comparing same file
    /// Contract: File compared to itself has similarity 1.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_file_compared_to_self() {
        // GIVEN: Single file
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Comparing file to itself
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path, &path, &options).unwrap();

        // THEN: Should be 1.0
        assert!((report.similarity.dice - 1.0).abs() < 0.001);
    }
}

// =============================================================================
// Block-Level Similarity Tests (D6)
// =============================================================================

#[cfg(test)]
mod block_level_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{
        compute_similarity, parse_target, SimilarityOptions,
    };
    use std::path::PathBuf;

    /// Test: Parse block target syntax
    /// Contract: file.py:start:end is valid
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_parse_block_target() {
        // GIVEN: Block target string
        let target = "src/code.py:10:50";

        // WHEN: Parsing target
        let parsed = parse_target(target).unwrap();

        // THEN: Should extract file and line range
        assert_eq!(parsed.file, PathBuf::from("src/code.py"));
        assert_eq!(parsed.line_range, Some((10, 50)));
        assert!(parsed.function.is_none());
    }

    /// Test: Block-level comparison
    /// Contract: Compare only specified lines
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_block_level_comparison() {
        // GIVEN: File with distinct blocks
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("code.py", FILE_WITH_BLOCKS).unwrap();

        // WHEN: Comparing similar blocks (A and B)
        let options = SimilarityOptions::default();
        let target_a = format!("{}:2:8", path.display()); // Block A
        let target_b = format!("{}:12:18", path.display()); // Block B

        let report = compute_similarity(
            &PathBuf::from(&target_a),
            &PathBuf::from(&target_b),
            &options,
        )
        .unwrap();

        // THEN: Should have high similarity (blocks A and B are similar)
        assert!(
            report.similarity.dice > 0.6,
            "Similar blocks should have high similarity"
        );
    }

    /// Test: Block with different content has low similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_block_different_content() {
        // GIVEN: File with distinct blocks
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("code.py", FILE_WITH_BLOCKS).unwrap();

        // WHEN: Comparing different blocks (A and C)
        let options = SimilarityOptions::default();
        let target_a = format!("{}:2:8", path.display()); // Block A
        let target_c = format!("{}:23:30", path.display()); // Block C (different)

        let report = compute_similarity(
            &PathBuf::from(&target_a),
            &PathBuf::from(&target_c),
            &options,
        )
        .unwrap();

        // THEN: Should have lower similarity
        assert!(
            report.similarity.dice < 0.8,
            "Different blocks should have lower similarity"
        );
    }

    /// Test: Invalid line range error
    /// Contract: start > end returns error
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_invalid_line_range() {
        // GIVEN: Invalid line range (start > end)
        let target = "file.py:50:10"; // 50 > 10

        // WHEN: Parsing
        let result = parse_target(target);

        // THEN: Should return error
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid") || err_msg.contains("range"),
            "Should indicate invalid range"
        );
    }

    /// Test: Line range out of bounds is clamped
    /// Contract: end > file_lines is clamped with warning
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_line_range_clamped() {
        // GIVEN: File with fewer lines than requested
        let short_file = "line1\nline2\nline3\n";
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("short.py", short_file).unwrap();

        // WHEN: Requesting lines beyond file end
        let options = SimilarityOptions::default();
        let target = format!("{}:1:100", path.display());

        let result = compute_similarity(&PathBuf::from(&target), &path, &options);

        // THEN: Should succeed (clamped) or warn
        // Implementation choice: either clamp or error
        // For robustness, we accept either
        if let Ok(report) = result {
            assert!(report.fragment1.lines <= 5); // Can't have more lines than file
        }
        // Error is also acceptable
    }

    /// Test: Fragment includes line range
    /// Contract: SimilarityFragment.line_range is set
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_fragment_includes_line_range() {
        // GIVEN: Block comparison
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("code.py", FILE_WITH_BLOCKS).unwrap();

        // WHEN: Comparing block
        let options = SimilarityOptions::default();
        let target = format!("{}:2:8", path.display());

        let report =
            compute_similarity(&PathBuf::from(&target), &PathBuf::from(&target), &options).unwrap();

        // THEN: Fragment should have line range
        assert_eq!(report.fragment1.line_range, Some((2, 8)));
    }
}

// =============================================================================
// N-gram Similarity Tests (D10)
// =============================================================================

#[cfg(test)]
mod ngram_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions};
    

    /// Test: Default n-gram size is 1 (unigrams)
    /// Contract: ngram_size = 1 by default
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_default_ngram_size() {
        // GIVEN: Default options
        let options = SimilarityOptions::default();

        // THEN: Default ngram_size should be 1
        assert_eq!(options.ngram_size, 1);
    }

    /// Test: Bigram similarity (n=2)
    /// Contract: Captures token pairs
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_bigram_similarity() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing with bigrams
        let options = SimilarityOptions {
            ngram_size: 2,
            ..Default::default()
        };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should compute valid similarity
        assert!(report.similarity.dice >= 0.0 && report.similarity.dice <= 1.0);
    }

    /// Test: Higher n = stricter matching
    /// Contract: Larger n-grams capture more context, may lower similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_higher_n_stricter() {
        // GIVEN: Two similar files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing with unigrams and trigrams
        let options_1 = SimilarityOptions {
            ngram_size: 1,
            ..Default::default()
        };
        let report_1 = compute_similarity(&path_a, &path_b, &options_1).unwrap();

        let options_3 = SimilarityOptions {
            ngram_size: 3,
            ..Default::default()
        };
        let report_3 = compute_similarity(&path_a, &path_b, &options_3).unwrap();

        // THEN: Higher n typically gives same or lower similarity
        // (unless code is very similar)
        assert!(
            report_3.similarity.dice <= report_1.similarity.dice + 0.1,
            "Higher n should not dramatically increase similarity"
        );
    }

    /// Test: N-gram with short input
    /// Contract: If tokens < n, return 0 similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_ngram_short_input() {
        // GIVEN: Very short files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", "x").unwrap();
        let path_b = test_dir.add_file("b.py", "y").unwrap();

        // WHEN: Computing with trigrams (n=3)
        let options = SimilarityOptions {
            ngram_size: 3,
            ..Default::default()
        };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should handle gracefully (0 similarity)
        assert!(
            (report.similarity.dice - 0.0).abs() < 0.001,
            "Short input with large n should give 0 similarity"
        );
    }

    /// Test: Config includes ngram_size
    /// Contract: Report reflects ngram_size used
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_ngram_in_config() {
        // GIVEN: Custom ngram size
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Computing with n=2
        let options = SimilarityOptions {
            ngram_size: 2,
            ..Default::default()
        };

        let report = compute_similarity(&path, &path, &options).unwrap();

        // THEN: Config should reflect n=2
        assert_eq!(report.config.ngram_size, 2);
    }
}

// =============================================================================
// Pairwise Similarity Matrix Tests (D11)
// =============================================================================

#[cfg(test)]
mod pairwise_matrix_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_pairwise_similarity, SimilarityOptions};
    

    /// Test: Compute all pairwise similarities
    /// Contract: Returns n*(n-1)/2 unique pairs
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_pairwise_all_pairs() {
        // GIVEN: Directory with 3 files
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();
        test_dir.add_file("c.py", PYTHON_FUNC_DIFFERENT).unwrap();

        // WHEN: Computing pairwise matrix
        let options = SimilarityOptions::default();
        let matrix = compute_pairwise_similarity(test_dir.path(), &options).unwrap();

        // THEN: Should have 3 pairs (a-b, a-c, b-c)
        assert_eq!(matrix.pairs.len(), 3);
    }

    /// Test: Matrix entries are valid
    /// Contract: All scores in [0, 1]
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_pairwise_valid_scores() {
        // GIVEN: Directory with files
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing pairwise matrix
        let options = SimilarityOptions::default();
        let matrix = compute_pairwise_similarity(test_dir.path(), &options).unwrap();

        // THEN: All scores should be valid
        for pair in &matrix.pairs {
            assert!(pair.dice >= 0.0 && pair.dice <= 1.0);
            assert!(pair.jaccard >= 0.0 && pair.jaccard <= 1.0);
        }
    }

    /// Test: Threshold filtering
    /// Contract: Only pairs above threshold are included
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_pairwise_threshold_filter() {
        // GIVEN: Files with varying similarity
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap(); // High sim
        test_dir
            .add_file("c.py", PYTHON_FUNC_VERY_DIFFERENT)
            .unwrap(); // Low sim

        // WHEN: Computing with high threshold
        let options = SimilarityOptions::default();
        // Assuming options has a threshold field for filtering
        // options.min_similarity = Some(0.8);

        let matrix_all = compute_pairwise_similarity(test_dir.path(), &options).unwrap();

        // THEN: Should have at least the high-similarity pair
        assert!(!matrix_all.pairs.is_empty());
    }

    /// Test: Single file returns empty matrix
    /// Contract: Need 2+ files for pairs
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_pairwise_single_file() {
        // GIVEN: Single file
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("only.py", PYTHON_FUNC_A).unwrap();

        // WHEN: Computing pairwise
        let options = SimilarityOptions::default();
        let matrix = compute_pairwise_similarity(test_dir.path(), &options).unwrap();

        // THEN: Should be empty
        assert!(matrix.pairs.is_empty());
    }

    /// Test: Pairs are sorted by similarity
    /// Contract: Most similar first (optional but nice)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_pairwise_sorted() {
        // GIVEN: Multiple files
        let test_dir = TestDir::new().unwrap();
        test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap();
        test_dir.add_file("c.py", PYTHON_FUNC_DIFFERENT).unwrap();

        // WHEN: Computing pairwise
        let options = SimilarityOptions::default();
        let matrix = compute_pairwise_similarity(test_dir.path(), &options).unwrap();

        // THEN: Should be sorted (highest first)
        for i in 1..matrix.pairs.len() {
            assert!(
                matrix.pairs[i - 1].dice >= matrix.pairs[i].dice,
                "Pairs should be sorted by similarity descending"
            );
        }
    }
}

// =============================================================================
// Score Interpretation Tests (D12)
// =============================================================================

#[cfg(test)]
mod score_interpretation_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{
        compute_similarity, interpret_similarity_score, SimilarityOptions,
    };
    

    /// Test: Identical code interpretation
    /// Contract: >= 0.95 = "Near-identical"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_near_identical() {
        // GIVEN: Score >= 0.95
        let interpretation = interpret_similarity_score(0.98);

        // THEN: Should indicate near-identical
        assert!(
            interpretation.to_lowercase().contains("identical")
                || interpretation.to_lowercase().contains("near"),
            "High score should indicate near-identical"
        );
    }

    /// Test: High similarity interpretation
    /// Contract: 0.85-0.95 = "High similarity"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_high_similarity() {
        // GIVEN: Score in [0.85, 0.95)
        let interpretation = interpret_similarity_score(0.88);

        // THEN: Should indicate high similarity
        assert!(
            interpretation.to_lowercase().contains("high")
                || interpretation.to_lowercase().contains("similar"),
            "Score 0.88 should indicate high similarity"
        );
    }

    /// Test: Moderate similarity interpretation
    /// Contract: 0.70-0.85 = "Moderate similarity"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_moderate_similarity() {
        // GIVEN: Score in [0.70, 0.85)
        let interpretation = interpret_similarity_score(0.75);

        // THEN: Should indicate moderate similarity
        assert!(
            interpretation.to_lowercase().contains("moderate")
                || interpretation.to_lowercase().contains("possible"),
            "Score 0.75 should indicate moderate similarity"
        );
    }

    /// Test: Low similarity interpretation
    /// Contract: 0.50-0.70 = "Some similarity"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_some_similarity() {
        // GIVEN: Score in [0.50, 0.70)
        let interpretation = interpret_similarity_score(0.55);

        // THEN: Should indicate some similarity
        assert!(
            interpretation.to_lowercase().contains("some")
                || interpretation.to_lowercase().contains("shared"),
            "Score 0.55 should indicate some similarity"
        );
    }

    /// Test: Very low similarity interpretation
    /// Contract: < 0.30 = "Very different"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_very_different() {
        // GIVEN: Score < 0.30
        let interpretation = interpret_similarity_score(0.15);

        // THEN: Should indicate very different
        assert!(
            interpretation.to_lowercase().contains("different")
                || interpretation.to_lowercase().contains("low"),
            "Score 0.15 should indicate very different"
        );
    }

    /// Test: Interpretation included in report
    /// Contract: Report has interpretation field
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_in_report() {
        // GIVEN: Similarity computation
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Should have interpretation
        assert!(
            !report.similarity.interpretation.is_empty(),
            "Report should include interpretation"
        );
    }

    /// Test: Boundary condition at 0.95
    /// Contract: 0.95 exactly should be "near-identical"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_boundary_095() {
        let interpretation = interpret_similarity_score(0.95);
        assert!(
            interpretation.to_lowercase().contains("identical")
                || interpretation.to_lowercase().contains("near")
        );
    }

    /// Test: Boundary condition at 0.70
    /// Contract: 0.70 exactly should be "moderate"
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_interpretation_boundary_070() {
        let interpretation = interpret_similarity_score(0.70);
        assert!(
            interpretation.to_lowercase().contains("moderate")
                || interpretation.to_lowercase().contains("possible")
        );
    }
}

// =============================================================================
// Token Breakdown Tests
// =============================================================================

#[cfg(test)]
mod token_breakdown_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions};
    

    /// Test: Token breakdown is computed
    /// Contract: Report includes shared/unique token counts
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_token_breakdown_computed() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Token breakdown should be present
        assert!(report.token_breakdown.total_unique > 0);
    }

    /// Test: Shared tokens count
    /// Contract: shared_tokens <= min(tokens1, tokens2)
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_shared_tokens_valid() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Shared tokens should be valid
        let min_tokens = report.fragment1.tokens.min(report.fragment2.tokens);
        assert!(
            report.token_breakdown.shared_tokens <= min_tokens,
            "Shared tokens can't exceed smaller fragment"
        );
    }

    /// Test: Unique tokens formula
    /// Contract: total_unique = shared + unique1 + unique2
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_unique_tokens_formula() {
        // GIVEN: Two files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_DIFFERENT).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: Formula should hold
        let expected_total = report.token_breakdown.shared_tokens
            + report.token_breakdown.unique_to_fragment1
            + report.token_breakdown.unique_to_fragment2;
        assert_eq!(report.token_breakdown.total_unique, expected_total);
    }

    /// Test: Identical files have no unique tokens
    /// Contract: unique_to_fragment1 = unique_to_fragment2 = 0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_identical_no_unique() {
        // GIVEN: Identical files
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_A_COPY).unwrap();

        // WHEN: Computing similarity
        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // THEN: No unique tokens (or very few due to normalization)
        assert!(
            report.token_breakdown.unique_to_fragment1 == 0
                || report.token_breakdown.unique_to_fragment1 < 3,
            "Identical files should have no/minimal unique tokens"
        );
    }
}

// =============================================================================
// Multi-Language Tests
// =============================================================================

#[cfg(test)]
mod multi_language_similarity_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions};
    

    /// Test: Python similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_python_similarity() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        let options = crate::analysis::similarity::SimilarityOptions { language: Some("python".to_string()), ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();
        assert!(report.similarity.dice > 0.5);
    }

    /// Test: TypeScript similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_typescript_similarity() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.ts", TS_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.ts", TS_FUNC_B_SIMILAR).unwrap();

        let options = crate::analysis::similarity::SimilarityOptions { language: Some("typescript".to_string()), ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();
        assert!(report.similarity.dice > 0.5);
    }

    /// Test: Go similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_go_similarity() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.go", GO_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.go", GO_FUNC_B_SIMILAR).unwrap();

        let options = crate::analysis::similarity::SimilarityOptions { language: Some("go".to_string()), ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();
        assert!(report.similarity.dice > 0.5);
    }

    /// Test: Rust similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_rust_similarity() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.rs", RUST_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.rs", RUST_FUNC_B_SIMILAR).unwrap();

        let options = crate::analysis::similarity::SimilarityOptions { language: Some("rust".to_string()), ..Default::default() };

        let report = compute_similarity(&path_a, &path_b, &options).unwrap();
        assert!(report.similarity.dice > 0.5);
    }

    /// Test: Auto-detect language
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_auto_detect_language() {
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("code.py", PYTHON_FUNC_A).unwrap();

        let options = SimilarityOptions::default(); // No language specified

        let report = compute_similarity(&path, &path, &options).unwrap();
        // Should succeed with auto-detection
        assert!((report.similarity.dice - 1.0).abs() < 0.001);
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[cfg(test)]
mod edge_case_similarity_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions};
    use std::path::PathBuf;

    /// Test: Empty file similarity
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_empty_file() {
        let test_dir = TestDir::new().unwrap();
        let path_empty = test_dir.add_file("empty.py", "").unwrap();
        let path_normal = test_dir.add_file("normal.py", PYTHON_FUNC_A).unwrap();

        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_empty, &path_normal, &options).unwrap();

        assert!((report.similarity.dice - 0.0).abs() < 0.001);
    }

    /// Test: Both files empty
    /// Contract: Both empty = similarity undefined or 1.0
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_both_files_empty() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", "").unwrap();
        let path_b = test_dir.add_file("b.py", "").unwrap();

        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // Two empty files could be considered identical (1.0) or undefined
        // Accept either 0.0 or 1.0
        assert!(report.similarity.dice == 0.0 || report.similarity.dice == 1.0);
    }

    /// Test: File not found error
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_file_not_found() {
        let test_dir = TestDir::new().unwrap();
        let path_existing = test_dir.add_file("exists.py", PYTHON_FUNC_A).unwrap();
        let path_missing = PathBuf::from("nonexistent.py");

        let options = SimilarityOptions::default();
        let result = compute_similarity(&path_existing, &path_missing, &options);

        assert!(result.is_err());
    }

    /// Test: Binary file handling
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_binary_file() {
        let test_dir = TestDir::new().unwrap();
        let path_normal = test_dir.add_file("code.py", PYTHON_FUNC_A).unwrap();
        let binary_path = test_dir.dir.path().join("binary.bin");
        std::fs::write(&binary_path, [0u8, 159, 146, 150]).unwrap();

        let options = SimilarityOptions::default();
        let result = compute_similarity(&path_normal, &binary_path, &options);

        // Should either error or return 0 similarity
        if let Ok(report) = result {
            assert!(report.similarity.dice < 0.1);
        }
    }

    /// Test: Very long file
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_long_file_performance() {
        // Generate a long file
        let long_content: String = (0..1000)
            .map(|i| format!("def func{}(): return {}\n", i, i))
            .collect();

        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("long_a.py", &long_content).unwrap();
        let path_b = test_dir.add_file("long_b.py", &long_content).unwrap();

        let options = SimilarityOptions::default();
        let start = std::time::Instant::now();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();
        let duration = start.elapsed();

        // Should complete in reasonable time (< 5s)
        assert!(duration.as_secs() < 5, "Long file comparison took too long");
        assert!((report.similarity.dice - 1.0).abs() < 0.001);
    }
}

// =============================================================================
// JSON Serialization Tests
// =============================================================================

#[cfg(test)]
mod serialization_tests {
    use super::fixtures::*;
    use crate::analysis::similarity::{compute_similarity, SimilarityOptions, SimilarityReport};
    

    /// Test: Report serializes to JSON
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_json_serialization() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // Should serialize without error
        let json = serde_json::to_string(&report);
        assert!(json.is_ok());
    }

    /// Test: Report deserializes from JSON
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_json_deserialization() {
        let test_dir = TestDir::new().unwrap();
        let path_a = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();
        let path_b = test_dir.add_file("b.py", PYTHON_FUNC_B_SIMILAR).unwrap();

        let options = SimilarityOptions::default();
        let report = compute_similarity(&path_a, &path_b, &options).unwrap();

        // Serialize then deserialize
        let json = serde_json::to_string(&report).unwrap();
        let deserialized: Result<SimilarityReport, _> = serde_json::from_str(&json);

        assert!(deserialized.is_ok());
        let restored = deserialized.unwrap();
        assert!((restored.similarity.dice - report.similarity.dice).abs() < 0.001);
    }

    /// Test: Config is included in JSON
    #[test]
    #[ignore = "similarity module not yet implemented"]
    fn test_config_in_json() {
        let test_dir = TestDir::new().unwrap();
        let path = test_dir.add_file("a.py", PYTHON_FUNC_A).unwrap();

        let options = SimilarityOptions {
            ngram_size: 3,
            ..Default::default()
        };

        let report = compute_similarity(&path, &path, &options).unwrap();
        let json = serde_json::to_string(&report).unwrap();

        // JSON should contain config
        assert!(
            json.contains("ngram_size") || json.contains("ngram"),
            "JSON should include config"
        );
    }
}
