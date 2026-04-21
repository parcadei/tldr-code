//! Clone Detection v2 Tests
//!
//! These tests define CORRECT behavior for the v2 rewrite of clone detection.
//! They target the six known bugs in v1:
//!
//! - BUG-1: Fabricated line numbers (not from tree-sitter)
//! - BUG-2: min_lines parameter ignored
//! - BUG-3: include_within_file logic error (only skips overlapping same-file)
//! - BUG-4: Catastrophic false positives from normalization + bag-of-tokens
//! - BUG-5: CloneFragment.preview never populated
//! - BUG-6: Fixed 25-token windows instead of syntactic boundaries
//!
//! These tests verify the clones detection module.
//! That is expected -- this is the TDD "write failing tests first" phase.
//!
//! # Test Categories
//!
//! 1. Accurate line numbers (BUG-1 fix)
//! 2. Function-level fragment extraction (BUG-6 fix)
//! 3. No false positives (BUG-4 fix)
//! 4. Preview populated (BUG-5 fix)
//! 5. include_within_file semantics (BUG-3 fix)
//! 6. min_lines enforced (BUG-2 fix)
//! 7. Sequence matching (prior art: Type-1/2/3 detection)
//! 8. JSON serialization preserved (backward compat)
//! 9. Existing preserved behaviors (is_test_file, is_generated_file, defaults)


// =============================================================================
// Fixture Helpers
// =============================================================================

/// Helper module for creating temp directories with known source files
mod v2_fixtures {
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Temporary directory that cleans up on drop
    pub struct V2TestDir {
        pub dir: TempDir,
    }

    impl V2TestDir {
        pub fn new() -> std::io::Result<Self> {
            Ok(Self {
                dir: TempDir::new()?,
            })
        }

        pub fn path(&self) -> &Path {
            self.dir.path()
        }

        /// Write a file relative to the temp dir, creating parent dirs as needed
        pub fn write_file(&self, rel_path: &str, content: &str) -> std::io::Result<PathBuf> {
            let path = self.dir.path().join(rel_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, content)?;
            Ok(path)
        }
    }

    // =========================================================================
    // Python fixtures -- three functions with known line numbers
    // =========================================================================

    /// A Python file with exactly 3 functions at known line positions.
    /// Line 1:  def add(a, b):
    /// Line 2:      return a + b
    /// Line 3:  (blank)
    /// Line 4:  def multiply(a, b):
    /// Line 5:      result = a * b
    /// Line 6:      return result
    /// Line 7:  (blank)
    /// Line 8:  def factorial(n):
    /// Line 9:      if n <= 1:
    /// Line 10:         return 1
    /// Line 11:     return n * factorial(n - 1)
    pub const PYTHON_THREE_FUNCTIONS: &str = "def add(a, b):\n    return a + b\n\ndef multiply(a, b):\n    result = a * b\n    return result\n\ndef factorial(n):\n    if n <= 1:\n        return 1\n    return n * factorial(n - 1)\n";

    /// Python file: function starts at line 1, ends at line 15
    pub const PYTHON_LONG_FUNCTION_A: &str = "\
def process_records(records):
    results = []
    for record in records:
        name = record.get('name', '')
        age = record.get('age', 0)
        email = record.get('email', '')
        if not name:
            continue
        if age < 0:
            continue
        processed = {'name': name, 'age': age, 'email': email}
        results.append(processed)
    return results
";

    /// Identical function with different name (Type-1 after normalization / raw Type-2)
    pub const PYTHON_LONG_FUNCTION_B: &str = "\
def process_records(records):
    results = []
    for record in records:
        name = record.get('name', '')
        age = record.get('age', 0)
        email = record.get('email', '')
        if not name:
            continue
        if age < 0:
            continue
        processed = {'name': name, 'age': age, 'email': email}
        results.append(processed)
    return results
";

    /// Same structure, renamed identifiers (Type-2 clone)
    pub const PYTHON_LONG_FUNCTION_RENAMED: &str = "\
def handle_entries(entries):
    output = []
    for entry in entries:
        label = entry.get('label', '')
        count = entry.get('count', 0)
        addr = entry.get('addr', '')
        if not label:
            continue
        if count < 0:
            continue
        item = {'label': label, 'count': count, 'addr': addr}
        output.append(item)
    return output
";

    /// Completely different function -- must NOT match
    pub const PYTHON_UNRELATED_FUNCTION: &str = "\
def fibonacci(n):
    if n <= 0:
        return 0
    elif n == 1:
        return 1
    a, b = 0, 1
    for _ in range(2, n + 1):
        a, b = b, a + b
    return b
";

    /// Two functions that share keywords but have different identifiers and structure
    pub const PYTHON_KEYWORD_OVERLAP_A: &str = "\
def check_permissions(user, resource):
    if user.is_admin:
        return True
    if resource.is_public:
        return True
    for role in user.roles:
        if role.has_access(resource):
            return True
    return False
";

    pub const PYTHON_KEYWORD_OVERLAP_B: &str = "\
def validate_input(data, schema):
    if data is None:
        return False
    if schema is None:
        return True
    for field in schema.fields:
        if field.required and field.name not in data:
            return False
    return True
";

    /// Import-heavy files that differ only in imports
    pub const PYTHON_IMPORT_HEAVY_A: &str = "\
from os import path
from sys import argv
from collections import defaultdict
from typing import List, Dict, Optional

def compute(x):
    return x * 2
";

    pub const PYTHON_IMPORT_HEAVY_B: &str = "\
from json import loads
from io import StringIO
from functools import reduce
from typing import Tuple, Set, Any

def transform(y):
    return y + 1
";

    /// A 3-line function (below min_lines=5 threshold)
    pub const PYTHON_SHORT_3_LINES: &str = "\
def tiny(x):
    y = x + 1
    return y
";

    /// A 5-line function (exactly at min_lines=5 threshold)
    pub const PYTHON_EXACTLY_5_LINES: &str = "\
def medium(data):
    result = []
    for item in data:
        result.append(item * 2)
    return result
";

    /// Same as medium but renamed (for min_lines boundary testing)
    pub const PYTHON_EXACTLY_5_LINES_COPY: &str = "\
def medium_copy(data):
    result = []
    for item in data:
        result.append(item * 2)
    return result
";

    /// File with two non-overlapping functions for within-file testing
    pub const PYTHON_TWO_FUNCTIONS_SAME_FILE: &str = "\
def handler_create(request):
    data = request.get_json()
    if not data:
        return {'error': 'No data provided'}, 400
    if 'name' not in data:
        return {'error': 'Name required'}, 400
    result = create_item(data)
    return {'id': result.id}, 201

def handler_update(request, item_id):
    data = request.get_json()
    if not data:
        return {'error': 'No data provided'}, 400
    if 'name' not in data:
        return {'error': 'Name required'}, 400
    result = update_item(item_id, data)
    return {'id': result.id}, 200
";

    // =========================================================================
    // Rust fixtures -- impl block with methods
    // =========================================================================

    /// Rust file with an impl block containing two methods.
    /// The impl block starts at line 5, method `new` at line 6, `area` at line 13.
    pub const RUST_IMPL_BLOCK: &str = "\
struct Rectangle {
    width: f64,
    height: f64,
}

impl Rectangle {
    fn new(width: f64, height: f64) -> Self {
        Self {
            width,
            height,
        }
    }

    fn area(&self) -> f64 {
        self.width * self.height
    }

    fn perimeter(&self) -> f64 {
        2.0 * (self.width + self.height)
    }
}
";

    // =========================================================================
    // Type-3 gap fixtures
    // =========================================================================

    /// Original function for Type-3 testing (~14 lines of code)
    pub const PYTHON_TYPE3_BASE: &str = "\
def process_data(data):
    result = []
    for item in data:
        if item is None:
            continue
        processed = transform(item)
        if processed.is_valid():
            result.append(processed)
    return result
";

    /// Same function with 2 added logging statements (~70-90% similar)
    pub const PYTHON_TYPE3_WITH_LOGGING: &str = "\
def process_data_logged(data):
    print('Starting processing')
    result = []
    for item in data:
        if item is None:
            continue
        processed = transform(item)
        print(f'Processed: {processed}')
        if processed.is_valid():
            result.append(processed)
    return result
";

    /// Completely different function -- must NOT be a Type-3 match
    pub const PYTHON_TYPE3_UNRELATED: &str = "\
def render_template(name, context):
    loader = FileSystemLoader('templates')
    env = Environment(loader=loader)
    template = env.get_template(name)
    output = template.render(context)
    return output
";

    // =========================================================================
    // Sequence matching fixtures (identical / renamed / gapped / different)
    // =========================================================================

    /// Two files with identical token sequences (Type-1, similarity 1.0)
    pub const SEQ_IDENTICAL_A: &str = "\
def compute_sum(values):
    total = 0
    for v in values:
        total += v
    return total
";

    pub const SEQ_IDENTICAL_B: &str = "\
def compute_sum(values):
    total = 0
    for v in values:
        total += v
    return total
";

    /// Renamed identifiers but same structure (Type-2, 0.9+)
    pub const SEQ_RENAMED_A: &str = "\
def calculate_total(numbers):
    accumulator = 0
    for num in numbers:
        accumulator += num
    return accumulator
";

    pub const SEQ_RENAMED_B: &str = "\
def sum_values(items):
    result = 0
    for item in items:
        result += item
    return result
";

    /// A few statements added/removed (Type-3, 0.7-0.9)
    pub const SEQ_GAPPED_A: &str = "\
def fetch_data(url):
    response = requests.get(url)
    if response.status_code != 200:
        raise Exception('Failed')
    data = response.json()
    return data
";

    pub const SEQ_GAPPED_B: &str = "\
def fetch_data_with_retry(url):
    for attempt in range(3):
        response = requests.get(url)
        if response.status_code != 200:
            continue
        data = response.json()
        return data
    raise Exception('All retries failed')
";

    /// Completely different (no match expected)
    pub const SEQ_DIFFERENT_A: &str = "\
def sort_descending(items):
    return sorted(items, reverse=True)
";

    pub const SEQ_DIFFERENT_B: &str = "\
class DatabaseConnection:
    def __init__(self, host, port):
        self.host = host
        self.port = port
        self.connected = False
";
}

// =============================================================================
// 1. ACCURATE LINE NUMBERS (BUG-1 fix)
// =============================================================================

#[cfg(test)]
mod accurate_line_numbers {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: Python file with 3 functions -- each function should have exact
    /// start/end lines from tree-sitter, not fabricated from token indices.
    ///
    /// BUG-1 in v1: line numbers are computed as `tokens.len() / 5 + 1` or
    /// `start / 10 + 1` which are arbitrary approximations.
    ///
    /// v2 REQ-3: Use `node.start_position().row + 1` from tree-sitter.
    #[test]
    fn test_python_function_line_numbers_match_tree_sitter() {
        let td = V2TestDir::new().unwrap();
        td.write_file("a.py", PYTHON_THREE_FUNCTIONS).unwrap();
        // Write a second copy so detection has something to compare against
        td.write_file("b.py", PYTHON_THREE_FUNCTIONS).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // We should have at least some clone pairs between a.py and b.py
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical files"
        );

        // Verify that line numbers are within actual file bounds (11 lines)
        for pair in &report.clone_pairs {
            assert!(
                pair.fragment1.start_line >= 1,
                "start_line must be >= 1, got {}",
                pair.fragment1.start_line
            );
            assert!(
                pair.fragment1.end_line <= 11,
                "end_line must be <= 11 (actual file length), got {}. \
                 This suggests fabricated line numbers (BUG-1).",
                pair.fragment1.end_line
            );
            assert!(
                pair.fragment1.start_line <= pair.fragment1.end_line,
                "start_line ({}) must be <= end_line ({})",
                pair.fragment1.start_line,
                pair.fragment1.end_line
            );
            // Same checks for fragment2
            assert!(
                pair.fragment2.start_line >= 1 && pair.fragment2.end_line <= 11,
                "fragment2 line numbers out of bounds: {}-{}",
                pair.fragment2.start_line,
                pair.fragment2.end_line
            );
        }
    }

    /// Test: Rust file with impl block -- methods should have correct line ranges
    /// from tree-sitter, not token-index heuristics.
    #[test]
    fn test_rust_impl_block_method_line_ranges() {
        let td = V2TestDir::new().unwrap();
        td.write_file("a.rs", RUST_IMPL_BLOCK).unwrap();
        td.write_file("b.rs", RUST_IMPL_BLOCK).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("rust".to_string()), min_tokens: 5, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // Verify line numbers are within file bounds (22 lines)
        for pair in &report.clone_pairs {
            assert!(
                pair.fragment1.end_line <= 22,
                "Rust fragment end_line {} exceeds file length 22 (BUG-1 fabrication)",
                pair.fragment1.end_line
            );
            assert!(
                pair.fragment2.end_line <= 22,
                "Rust fragment2 end_line {} exceeds file length 22",
                pair.fragment2.end_line
            );
        }
    }

    /// Test: Line numbers for a 14-line Python function should never exceed 14.
    /// v1 with 50 tokens would report end_line = 50/5+1 = 11 (coincidentally close)
    /// but for different token counts the heuristic diverges wildly.
    #[test]
    fn test_line_numbers_not_derived_from_token_count() {
        let td = V2TestDir::new().unwrap();
        td.write_file("a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert!(
            !report.clone_pairs.is_empty(),
            "Expected at least one clone pair"
        );

        for pair in &report.clone_pairs {
            // PYTHON_LONG_FUNCTION_A has 14 lines of text
            let frag = &pair.fragment1;
            let line_count = frag.end_line - frag.start_line + 1;

            // The function is ~14 lines. With v1's heuristic of tokens/5,
            // a function with 40+ tokens would report end_line > 14.
            // v2 must report actual line numbers.
            assert!(
                frag.end_line <= 14,
                "Fragment end_line {} exceeds actual file line count 14. \
                 Likely derived from token count, not tree-sitter. (BUG-1)",
                frag.end_line
            );
            assert!(
                line_count >= 5,
                "A 14-line function should produce a fragment of at least 5 lines, got {}",
                line_count
            );
        }
    }
}

// =============================================================================
// 2. FUNCTION-LEVEL FRAGMENT EXTRACTION (BUG-6 fix)
// =============================================================================

#[cfg(test)]
mod function_level_extraction {
    use super::v2_fixtures::*;
    use crate::analysis::clones::{detect_clones, CloneType};

    /// Test: Two identical Python functions in separate files -> detected as clone.
    /// v2 REQ-1: Use tree-sitter to extract function_definition nodes.
    #[test]
    fn test_identical_functions_detected_as_clone() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            1,
            "Two identical functions should produce exactly 1 clone pair"
        );

        let pair = &report.clone_pairs[0];
        assert_eq!(
            pair.clone_type,
            CloneType::Type1,
            "Identical functions should be Type-1"
        );
        assert!(
            (pair.similarity - 1.0).abs() < 1e-6,
            "Type-1 clone should have similarity ~1.0, got {}",
            pair.similarity
        );
    }

    /// Test: Two completely different Python functions -> NOT detected as clones.
    /// This is the false positive guard.
    #[test]
    fn test_different_functions_not_detected() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_UNRELATED_FUNCTION)
            .unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Completely different functions should NOT be detected as clones"
        );
    }

    /// Test: Fragment boundaries align with function definitions,
    /// not arbitrary token windows.
    /// v1 BUG-6: Fixed 25-token sliding windows cut through function boundaries.
    #[test]
    fn test_fragment_boundaries_are_syntactic() {
        let td = V2TestDir::new().unwrap();
        // A file with 3 separate functions
        td.write_file("a.py", PYTHON_THREE_FUNCTIONS).unwrap();
        td.write_file("b.py", PYTHON_THREE_FUNCTIONS).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // Each clone pair fragment should correspond to a function boundary.
        // In PYTHON_THREE_FUNCTIONS:
        //   add:       lines 1-2
        //   multiply:  lines 4-6
        //   factorial:  lines 8-11
        // Fragments should NOT have start_line in the middle of a function.
        for pair in &report.clone_pairs {
            let frag = &pair.fragment1;
            // Valid function start lines in our fixture are: 1, 4, 8
            let valid_starts = [1, 4, 8];
            assert!(
                valid_starts.contains(&frag.start_line),
                "Fragment start_line {} does not align with any function definition \
                 start (expected one of {:?}). This suggests token-window fragmentation \
                 instead of syntactic extraction (BUG-6).",
                frag.start_line,
                valid_starts
            );
        }
    }

    /// Test: Import-only differences do not create false positives.
    /// v2 should skip import/use statements during fragment extraction.
    #[test]
    fn test_import_differences_not_false_positive() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_IMPORT_HEAVY_A).unwrap();
        td.write_file("src/b.py", PYTHON_IMPORT_HEAVY_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // The actual function bodies (compute vs transform) are completely different.
        // Only the import patterns are structurally similar. Should NOT match.
        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Files with different functions but similar import patterns \
             should NOT be detected as clones"
        );
    }

    /// Test: Renamed identifiers detected as Type-2
    #[test]
    fn test_renamed_identifiers_detected_as_type2() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_RENAMED)
            .unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            1,
            "Functions with renamed identifiers should produce 1 clone pair"
        );

        let pair = &report.clone_pairs[0];
        assert!(
            pair.clone_type == CloneType::Type2 || pair.clone_type == CloneType::Type1,
            "Renamed identifiers should be at least Type-2, got {:?}",
            pair.clone_type
        );
        assert!(
            pair.similarity >= 0.9,
            "Type-2 clone should have similarity >= 0.9, got {}",
            pair.similarity
        );
    }
}

// =============================================================================
// 3. NO FALSE POSITIVES (BUG-4 fix)
// =============================================================================

#[cfg(test)]
mod no_false_positives {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: Two files with same keywords but different identifiers and structure
    /// should have similarity < 0.7 (below threshold).
    ///
    /// BUG-4: When normalization=All, all identifiers become $ID, making
    /// unrelated code appear similar in bag-of-tokens comparison.
    #[test]
    fn test_keyword_overlap_below_threshold() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_KEYWORD_OVERLAP_A).unwrap();
        td.write_file("src/b.py", PYTHON_KEYWORD_OVERLAP_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // These functions share keywords (if, return, True, False, for, in)
        // but have different identifiers and logic. Should NOT be clones.
        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Functions with same keywords but different structure/identifiers \
             should NOT be detected as clones. This is the BUG-4 false positive."
        );
    }

    /// Test: Two files with `from X import Y` patterns are NOT clones.
    /// Import statements should be filtered out of fragment comparison.
    #[test]
    fn test_import_pattern_not_clones() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_IMPORT_HEAVY_A).unwrap();
        td.write_file("src/b.py", PYTHON_IMPORT_HEAVY_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Import-heavy files with different function bodies must not match"
        );
    }

    /// Test: Two structurally different functions that happen to use same
    /// keywords are NOT clones -- even with normalization=All.
    ///
    /// v2 REQ-7: Use raw tokens for Dice similarity, not normalized.
    #[test]
    fn test_different_structure_same_keywords_not_clones() {
        // check_permissions: if/return/for/if/return pattern
        // validate_input: if/return/if/return/for/if/return pattern
        // Structurally similar keyword flow but semantically different
        let td = V2TestDir::new().unwrap();
        td.write_file("src/check.py", PYTHON_KEYWORD_OVERLAP_A)
            .unwrap();
        td.write_file("src/validate.py", PYTHON_KEYWORD_OVERLAP_B)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };
        // Explicitly use All normalization to stress-test BUG-4 fix
        opts.normalization = crate::analysis::clones::NormalizationMode::All;

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Even with normalization=All, structurally different functions \
             should NOT be matched. v2 REQ-7 requires raw tokens for Dice."
        );
    }

    /// Test: An unrelated function should never match a real function.
    /// This is a basic sanity check for false positive rate.
    #[test]
    fn test_unrelated_functions_no_match() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/process.py", PYTHON_LONG_FUNCTION_A)
            .unwrap();
        td.write_file("src/fibonacci.py", PYTHON_UNRELATED_FUNCTION)
            .unwrap();
        td.write_file("src/render.py", PYTHON_TYPE3_UNRELATED)
            .unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Three unrelated functions should produce zero clone pairs"
        );
    }
}

// =============================================================================
// 4. PREVIEW POPULATED (BUG-5 fix)
// =============================================================================

#[cfg(test)]
mod preview_populated {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: Every CloneFragment in results has preview != None.
    /// BUG-5: v1 never calls `.with_preview()`, so preview is always None.
    /// v2 REQ-4: Populate preview from source lines.
    #[test]
    fn test_all_fragments_have_preview() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert!(
            !report.clone_pairs.is_empty(),
            "Need at least one pair to test preview"
        );

        for (i, pair) in report.clone_pairs.iter().enumerate() {
            assert!(
                pair.fragment1.preview.is_some(),
                "Clone pair {} fragment1 has preview=None (BUG-5 not fixed)",
                i + 1
            );
            assert!(
                pair.fragment2.preview.is_some(),
                "Clone pair {} fragment2 has preview=None (BUG-5 not fixed)",
                i + 1
            );
        }
    }

    /// Test: Preview contains actual source code from the file.
    /// The preview should be the first ~100 chars of the fragment's source lines.
    #[test]
    fn test_preview_contains_source_code() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert!(!report.clone_pairs.is_empty());

        let preview = report.clone_pairs[0].fragment1.preview.as_ref().unwrap();

        // The function starts with "def process_records"
        assert!(
            preview.contains("def process_records") || preview.contains("process_records"),
            "Preview should contain actual source code from the fragment, \
             got: {:?}",
            preview
        );
    }

    /// Test: Preview is truncated to 100 characters (with "..." suffix).
    #[test]
    fn test_preview_truncated_to_100_chars() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert!(!report.clone_pairs.is_empty());

        for pair in &report.clone_pairs {
            if let Some(ref preview) = pair.fragment1.preview {
                assert!(
                    preview.len() <= 103, // 100 chars + "..."
                    "Preview should be truncated to ~100 chars, got {} chars",
                    preview.len()
                );
            }
        }
    }
}

// =============================================================================
// 5. INCLUDE_WITHIN_FILE SEMANTICS (BUG-3 fix)
// =============================================================================

#[cfg(test)]
mod include_within_file {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: With include_within_file=false, NO same-file pairs returned.
    /// BUG-3: v1 only skips OVERLAPPING same-file pairs. Non-overlapping
    /// same-file function pairs slip through.
    /// v2 REQ-5: Skip ALL same-file pairs unconditionally.
    #[test]
    fn test_within_file_false_excludes_all_same_file() {
        let td = V2TestDir::new().unwrap();
        // This file has two similar but non-overlapping handler functions
        td.write_file("src/handlers.py", PYTHON_TWO_FUNCTIONS_SAME_FILE)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };
        opts.include_within_file = false; // <-- the flag under test

        let report = detect_clones(td.path(), &opts).unwrap();

        // With only one file and include_within_file=false, there should
        // be ZERO pairs. BUG-3 would leak non-overlapping same-file pairs.
        for pair in &report.clone_pairs {
            assert_ne!(
                pair.fragment1.file, pair.fragment2.file,
                "include_within_file=false should exclude ALL same-file pairs, \
                 but found pair with both fragments in {:?} (BUG-3)",
                pair.fragment1.file
            );
        }
    }

    /// Test: With include_within_file=true, same-file non-overlapping
    /// pairs ARE returned.
    #[test]
    fn test_within_file_true_includes_same_file_pairs() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/handlers.py", PYTHON_TWO_FUNCTIONS_SAME_FILE)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };
        opts.include_within_file = true; // <-- allow same-file pairs

        let report = detect_clones(td.path(), &opts).unwrap();

        // handler_create and handler_update are structurally very similar
        // (Type-2 or Type-3 clones). With within-file enabled, should detect.
        assert!(
            !report.clone_pairs.is_empty(),
            "include_within_file=true should detect similar non-overlapping \
             functions in the same file"
        );

        // Verify at least one pair has both fragments from the same file
        let has_same_file_pair = report
            .clone_pairs
            .iter()
            .any(|p| p.fragment1.file == p.fragment2.file);
        assert!(
            has_same_file_pair,
            "Expected at least one same-file clone pair with include_within_file=true"
        );
    }

    /// Test: Same-file OVERLAPPING fragments are always excluded,
    /// regardless of include_within_file setting.
    #[test]
    fn test_overlapping_same_file_always_excluded() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/handlers.py", PYTHON_TWO_FUNCTIONS_SAME_FILE)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 1, ..Default::default() };
        opts.include_within_file = true;

        let report = detect_clones(td.path(), &opts).unwrap();

        // No pair should have overlapping line ranges within the same file
        for pair in &report.clone_pairs {
            if pair.fragment1.file == pair.fragment2.file {
                let f1_start = pair.fragment1.start_line;
                let f1_end = pair.fragment1.end_line;
                let f2_start = pair.fragment2.start_line;
                let f2_end = pair.fragment2.end_line;

                let overlaps = f1_start <= f2_end && f2_start <= f1_end;
                assert!(
                    !overlaps,
                    "Same-file overlapping fragments must always be excluded. \
                     Got overlap: [{}-{}] and [{}-{}] in {:?}",
                    f1_start, f1_end, f2_start, f2_end, pair.fragment1.file
                );
            }
        }
    }
}

// =============================================================================
// 6. MIN_LINES ENFORCED (BUG-2 fix)
// =============================================================================

#[cfg(test)]
mod min_lines_enforced {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: With min_lines=5, fragments with fewer than 5 lines are excluded.
    /// BUG-2: v1 ignores the min_lines parameter entirely (_min_lines).
    /// v2 REQ-6: Enforce fragment.line_count() >= min_lines.
    #[test]
    fn test_min_lines_excludes_short_fragments() {
        let td = V2TestDir::new().unwrap();
        // PYTHON_SHORT_3_LINES is only 3 lines
        td.write_file("src/a.py", PYTHON_SHORT_3_LINES).unwrap();
        td.write_file("src/b.py", PYTHON_SHORT_3_LINES).unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 5, ..Default::default() }; // <-- 3-line functions should be excluded
        opts.include_within_file = false;

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "A 3-line clone pair should NOT be reported when min_lines=5. \
             This suggests min_lines is being ignored (BUG-2)."
        );
    }

    /// Test: A 3-line clone pair is NOT reported when min_lines=5.
    /// Explicit regression test for BUG-2.
    #[test]
    fn test_3_line_clone_not_reported_with_min_lines_5() {
        let td = V2TestDir::new().unwrap();
        // These are 3-line functions: def tiny(x): / y = x + 1 / return y
        let short_a = "def tiny_a(x):\n    y = x + 1\n    return y\n";
        let short_b = "def tiny_b(x):\n    y = x + 1\n    return y\n";
        td.write_file("src/a.py", short_a).unwrap();
        td.write_file("src/b.py", short_b).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 5, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "3-line functions must be excluded when min_lines=5"
        );
    }

    /// Test: Exactly 5-line functions ARE reported when min_lines=5.
    /// Boundary condition: line_count == min_lines should pass.
    #[test]
    fn test_exactly_min_lines_included() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_EXACTLY_5_LINES).unwrap();
        td.write_file("src/b.py", PYTHON_EXACTLY_5_LINES_COPY)
            .unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 5, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // 5-line functions with min_lines=5 should pass the filter
        assert!(
            !report.clone_pairs.is_empty(),
            "5-line functions should be included when min_lines=5 (boundary condition)"
        );
    }

    /// Test: All reported fragments respect the min_lines constraint.
    #[test]
    fn test_all_fragments_respect_min_lines() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();
        // Also add short functions that should be filtered out
        td.write_file("src/tiny.py", PYTHON_SHORT_3_LINES).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 5, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        for pair in &report.clone_pairs {
            let f1_lines = pair.fragment1.end_line - pair.fragment1.start_line + 1;
            let f2_lines = pair.fragment2.end_line - pair.fragment2.start_line + 1;
            assert!(
                f1_lines >= 5,
                "Fragment1 has {} lines, below min_lines=5 (BUG-2)",
                f1_lines
            );
            assert!(
                f2_lines >= 5,
                "Fragment2 has {} lines, below min_lines=5 (BUG-2)",
                f2_lines
            );
        }
    }
}

// =============================================================================
// 7. SEQUENCE MATCHING (prior art: Type-1/2/3 detection)
// =============================================================================

#[cfg(test)]
mod sequence_matching {
    use super::v2_fixtures::*;
    use crate::analysis::clones::{detect_clones, CloneType};

    /// Test: Two identical token sequences -> Type-1 (similarity 1.0)
    #[test]
    fn test_identical_sequences_type1() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", SEQ_IDENTICAL_A).unwrap();
        td.write_file("src/b.py", SEQ_IDENTICAL_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(report.clone_pairs.len(), 1, "Expected exactly 1 clone pair");

        let pair = &report.clone_pairs[0];
        assert_eq!(pair.clone_type, CloneType::Type1);
        assert!(
            (pair.similarity - 1.0).abs() < 1e-6,
            "Type-1 should have similarity == 1.0, got {}",
            pair.similarity
        );
    }

    /// Test: Two sequences with renamed identifiers but same structure -> Type-2 (0.9+)
    #[test]
    fn test_renamed_identifiers_type2() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", SEQ_RENAMED_A).unwrap();
        td.write_file("src/b.py", SEQ_RENAMED_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(report.clone_pairs.len(), 1, "Expected exactly 1 clone pair");

        let pair = &report.clone_pairs[0];
        assert!(
            pair.clone_type == CloneType::Type2 || pair.clone_type == CloneType::Type1,
            "Renamed identifiers should be Type-2 (or Type-1 after normalization), got {:?}",
            pair.clone_type
        );
        assert!(
            pair.similarity >= 0.9,
            "Type-2 should have similarity >= 0.9, got {}",
            pair.similarity
        );
    }

    /// Test: Two sequences with added/removed statements -> Type-3 (0.7-0.9)
    #[test]
    fn test_gapped_sequences_type3() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", SEQ_GAPPED_A).unwrap();
        td.write_file("src/b.py", SEQ_GAPPED_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        // Should detect as Type-3 (gapped) if similarity is 0.7+
        if !report.clone_pairs.is_empty() {
            let pair = &report.clone_pairs[0];
            assert_eq!(
                pair.clone_type,
                CloneType::Type3,
                "Gapped clone should be Type-3, got {:?}",
                pair.clone_type
            );
            assert!(
                pair.similarity >= 0.7 && pair.similarity < 0.9,
                "Type-3 should have 0.7 <= similarity < 0.9, got {}",
                pair.similarity
            );
        }
        // It's acceptable if the gap is too large and no match is found,
        // but if found it must be classified correctly.
    }

    /// Test: Two completely different sequences -> no match
    #[test]
    fn test_completely_different_no_match() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", SEQ_DIFFERENT_A).unwrap();
        td.write_file("src/b.py", SEQ_DIFFERENT_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 3, min_lines: 1, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Completely different code should produce zero clone pairs"
        );
    }

    /// Test: Type-3 detection with logging-augmented function
    #[test]
    fn test_type3_logging_augmented() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/base.py", PYTHON_TYPE3_BASE).unwrap();
        td.write_file("src/logged.py", PYTHON_TYPE3_WITH_LOGGING)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };
        opts.threshold = 0.7;

        let report = detect_clones(td.path(), &opts).unwrap();

        // These are structurally similar with added logging. Should be Type-3.
        if !report.clone_pairs.is_empty() {
            let pair = &report.clone_pairs[0];
            assert!(
                pair.similarity >= 0.7,
                "Logging-augmented function should have similarity >= 0.7, got {}",
                pair.similarity
            );
            assert!(
                pair.clone_type == CloneType::Type3 || pair.clone_type == CloneType::Type2,
                "Expected Type-3 or Type-2 for logging-augmented clone"
            );
        }
    }
}

// =============================================================================
// 8. JSON SERIALIZATION PRESERVED
// =============================================================================

#[cfg(test)]
mod json_serialization {
    use crate::analysis::clones::{
        CloneConfig, CloneFragment, ClonePair, CloneStats, CloneType, ClonesReport,
        NormalizationMode,
    };
    use std::path::PathBuf;

    /// Test: ClonesReport serializes with same serde rules as v1.
    /// The JSON structure must be byte-compatible with v1 output.
    #[test]
    fn test_clones_report_serialization_format() {
        let report = ClonesReport {
            root: PathBuf::from("/tmp/test"),
            language: "auto".to_string(),
            clone_pairs: vec![],
            clone_classes: vec![],
            stats: CloneStats::default(),
            config: CloneConfig::default(),
        };

        let json = serde_json::to_value(&report).unwrap();

        // clone_classes should be ABSENT (not null, not empty array) when empty
        assert!(
            json.get("clone_classes").is_none(),
            "clone_classes should be absent from JSON when empty, not {:?}",
            json.get("clone_classes")
        );

        // Required fields must be present
        assert!(json.get("root").is_some());
        assert!(json.get("language").is_some());
        assert!(json.get("clone_pairs").is_some());
        assert!(json.get("stats").is_some());
        assert!(json.get("config").is_some());
    }

    /// Test: CloneType serializes as "Type-1", "Type-2", "Type-3"
    #[test]
    fn test_clone_type_serde_renames() {
        let type1 = serde_json::to_string(&CloneType::Type1).unwrap();
        let type2 = serde_json::to_string(&CloneType::Type2).unwrap();
        let type3 = serde_json::to_string(&CloneType::Type3).unwrap();

        assert_eq!(type1, "\"Type-1\"", "Type1 should serialize as \"Type-1\"");
        assert_eq!(type2, "\"Type-2\"", "Type2 should serialize as \"Type-2\"");
        assert_eq!(type3, "\"Type-3\"", "Type3 should serialize as \"Type-3\"");

        // Round-trip deserialization
        let rt: CloneType = serde_json::from_str("\"Type-1\"").unwrap();
        assert_eq!(rt, CloneType::Type1);
        let rt: CloneType = serde_json::from_str("\"Type-2\"").unwrap();
        assert_eq!(rt, CloneType::Type2);
        let rt: CloneType = serde_json::from_str("\"Type-3\"").unwrap();
        assert_eq!(rt, CloneType::Type3);
    }

    /// Test: NormalizationMode serializes as lowercase strings
    #[test]
    fn test_normalization_mode_serde() {
        let none = serde_json::to_string(&NormalizationMode::None).unwrap();
        let ident = serde_json::to_string(&NormalizationMode::Identifiers).unwrap();
        let lit = serde_json::to_string(&NormalizationMode::Literals).unwrap();
        let all = serde_json::to_string(&NormalizationMode::All).unwrap();

        assert_eq!(none, "\"none\"");
        assert_eq!(ident, "\"identifiers\"");
        assert_eq!(lit, "\"literals\"");
        assert_eq!(all, "\"all\"");
    }

    /// Test: Optional fields are absent (not null) in JSON when None.
    #[test]
    fn test_optional_fields_absent_not_null() {
        let fragment = CloneFragment::new(PathBuf::from("test.py"), 1, 10, 50);
        let json = serde_json::to_value(&fragment).unwrap();

        // function and preview are None by default
        assert!(
            json.get("function").is_none(),
            "function=None should be absent, not null"
        );
        assert!(
            json.get("preview").is_none(),
            "preview=None should be absent, not null"
        );

        // lines should be present (it's auto-computed to Some(10))
        assert!(
            json.get("lines").is_some(),
            "lines should be present when computed"
        );
    }

    /// Test: ClonePair interpretation field absent when None.
    #[test]
    fn test_interpretation_absent_when_none() {
        let pair = ClonePair::new(
            1,
            CloneType::Type1,
            1.0,
            CloneFragment::new(PathBuf::from("a.py"), 1, 5, 20),
            CloneFragment::new(PathBuf::from("b.py"), 1, 5, 20),
        );

        let json = serde_json::to_value(&pair).unwrap();

        // interpretation should be present (ClonePair::new auto-populates it)
        assert!(
            json.get("interpretation").is_some(),
            "interpretation should be set by ClonePair::new"
        );
    }

    /// Test: CloneStats.class_count absent when None.
    #[test]
    fn test_class_count_absent_when_none() {
        let stats = CloneStats::default();
        let json = serde_json::to_value(&stats).unwrap();

        assert!(
            json.get("class_count").is_none(),
            "class_count=None should be absent from JSON"
        );
    }

    /// Test: CloneConfig.type_filter absent when None.
    #[test]
    fn test_type_filter_absent_when_none() {
        let config = CloneConfig::default();
        let json = serde_json::to_value(&config).unwrap();

        assert!(
            json.get("type_filter").is_none(),
            "type_filter=None should be absent from JSON"
        );
    }

    /// Test: Full round-trip serialization/deserialization.
    #[test]
    fn test_full_report_round_trip() {
        let report = ClonesReport {
            root: PathBuf::from("/tmp/project"),
            language: "python".to_string(),
            clone_pairs: vec![ClonePair::new(
                1,
                CloneType::Type2,
                0.95,
                CloneFragment::new(PathBuf::from("a.py"), 1, 10, 30)
                    .with_preview("def foo():".to_string()),
                CloneFragment::new(PathBuf::from("b.py"), 5, 14, 30)
                    .with_preview("def bar():".to_string()),
            )],
            clone_classes: vec![],
            stats: CloneStats {
                files_analyzed: 2,
                total_tokens: 60,
                clones_found: 1,
                type1_count: 0,
                type2_count: 1,
                type3_count: 0,
                class_count: None,
                detection_time_ms: 42,
            },
            config: CloneConfig::default(),
        };

        let json_str = serde_json::to_string_pretty(&report).unwrap();
        let deserialized: ClonesReport = serde_json::from_str(&json_str).unwrap();

        assert_eq!(deserialized.root, report.root);
        assert_eq!(deserialized.language, report.language);
        assert_eq!(deserialized.clone_pairs.len(), 1);
        assert_eq!(deserialized.clone_pairs[0].clone_type, CloneType::Type2);
        assert_eq!(deserialized.stats.files_analyzed, 2);
        assert_eq!(deserialized.stats.clones_found, 1);
    }
}

// =============================================================================
// 9. EXISTING PRESERVED BEHAVIORS
// =============================================================================

#[cfg(test)]
mod preserved_behaviors {
    use crate::analysis::clones::{
        classify_clone_type, is_generated_file, is_test_file, CloneType, ClonesOptions,
        NormalizationMode,
    };
    use std::path::Path;

    // -------------------------------------------------------------------------
    // is_test_file() must match same patterns as v1
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_test_file_directory_patterns() {
        // Directory patterns that should match
        assert!(is_test_file(Path::new("project/tests/test_foo.py")));
        assert!(is_test_file(Path::new("project/test/helper.py")));
        assert!(is_test_file(Path::new("src/__tests__/component.test.js")));
        assert!(is_test_file(Path::new("spec/models/user_spec.rb")));
        assert!(is_test_file(Path::new("testing/integration.py")));
    }

    #[test]
    fn test_is_test_file_name_patterns() {
        // File name patterns that should match
        assert!(is_test_file(Path::new("test_utils.py")));
        assert!(is_test_file(Path::new("auth_test.py")));
        assert!(is_test_file(Path::new("handler_test.go")));
        assert!(is_test_file(Path::new("parser_test.rs")));
        assert!(is_test_file(Path::new("model_spec.rb")));
        assert!(is_test_file(Path::new("button.test.ts")));
        assert!(is_test_file(Path::new("button.test.js")));
        assert!(is_test_file(Path::new("api.spec.ts")));
        assert!(is_test_file(Path::new("api.spec.js")));
        assert!(is_test_file(Path::new("UserTest.java")));
        assert!(is_test_file(Path::new("UserTests.cs")));
    }

    #[test]
    fn test_is_test_file_non_test_files() {
        // These should NOT be detected as test files
        assert!(!is_test_file(Path::new("src/main.py")));
        assert!(!is_test_file(Path::new("src/utils.rs")));
        assert!(!is_test_file(Path::new("lib/handler.go")));
        assert!(!is_test_file(Path::new("app/models/user.py")));
    }

    // -------------------------------------------------------------------------
    // is_generated_file() must match same patterns as v1
    // -------------------------------------------------------------------------

    #[test]
    fn test_is_generated_file_directory_patterns() {
        assert!(is_generated_file(Path::new("vendor/lib/foo.py")));
        assert!(is_generated_file(Path::new("node_modules/pkg/index.js")));
        assert!(is_generated_file(Path::new("__pycache__/module.pyc")));
        assert!(is_generated_file(Path::new("dist/bundle.js")));
        assert!(is_generated_file(Path::new("build/output.js")));
        assert!(is_generated_file(Path::new("target/debug/main.rs")));
        assert!(is_generated_file(Path::new("gen/proto.go")));
        assert!(is_generated_file(Path::new("generated/types.ts")));
        assert!(is_generated_file(Path::new(".gen/schema.rs")));
        assert!(is_generated_file(Path::new("third_party/lib.go")));
        assert!(is_generated_file(Path::new("external/dep.rs")));
    }

    #[test]
    fn test_is_generated_file_suffix_patterns() {
        // Protobuf
        assert!(is_generated_file(Path::new("api.pb.go")));
        assert!(is_generated_file(Path::new("schema_pb2.py")));
        assert!(is_generated_file(Path::new("types.pb.ts")));
        assert!(is_generated_file(Path::new("types.pb.js")));
        assert!(is_generated_file(Path::new("types.pb.rs")));
        assert!(is_generated_file(Path::new("api_grpc.pb.go")));
        assert!(is_generated_file(Path::new("api_pb2_grpc.py")));

        // Codegen
        assert!(is_generated_file(Path::new("schema.generated.ts")));
        assert!(is_generated_file(Path::new("schema.generated.tsx")));
        assert!(is_generated_file(Path::new("schema.generated.js")));
        assert!(is_generated_file(Path::new("query.graphql.ts")));

        // General generated
        assert!(is_generated_file(Path::new("types_generated.go")));
        assert!(is_generated_file(Path::new("types_generated.ts")));
        assert!(is_generated_file(Path::new("types_generated.rs")));
        assert!(is_generated_file(Path::new("types_generated.py")));
        assert!(is_generated_file(Path::new("schema.gen.go")));
        assert!(is_generated_file(Path::new("schema.gen.ts")));
        assert!(is_generated_file(Path::new("schema.gen.rs")));

        // Mock
        assert!(is_generated_file(Path::new("client_mock.go")));
        assert!(is_generated_file(Path::new("service_mocks.go")));

        // Thrift
        assert!(is_generated_file(Path::new("service.thrift.go")));
    }

    #[test]
    fn test_is_generated_file_prefix_patterns() {
        assert!(is_generated_file(Path::new("generated_types.py")));
        assert!(is_generated_file(Path::new("auto_generated_schema.ts")));
        assert!(is_generated_file(Path::new("autogenerated_client.go")));
        assert!(is_generated_file(Path::new("mock_service.py")));
        assert!(is_generated_file(Path::new("mocks_handler.py")));

        // Case insensitive
        assert!(is_generated_file(Path::new("Generated_Types.py")));
        assert!(is_generated_file(Path::new("AUTO_GENERATED_SCHEMA.ts")));
    }

    #[test]
    fn test_is_generated_file_non_generated() {
        assert!(!is_generated_file(Path::new("src/main.py")));
        assert!(!is_generated_file(Path::new("lib/utils.ts")));
        assert!(!is_generated_file(Path::new("cmd/server.go")));
    }

    // -------------------------------------------------------------------------
    // ClonesOptions defaults must match spec
    // -------------------------------------------------------------------------

    #[test]
    fn test_clones_options_defaults() {
        let opts = ClonesOptions::default();

        assert_eq!(opts.min_tokens, 25, "Default min_tokens should be 25");
        assert_eq!(opts.min_lines, 5, "Default min_lines should be 5");
        assert!(
            (opts.threshold - 0.7).abs() < 1e-9,
            "Default threshold should be 0.7"
        );
        assert_eq!(opts.type_filter, None, "Default type_filter should be None");
        assert_eq!(
            opts.normalization,
            NormalizationMode::All,
            "Default normalization should be All"
        );
        assert_eq!(opts.language, None, "Default language should be None");
        assert!(!opts.show_classes, "Default show_classes should be false");
        assert!(
            !opts.include_within_file,
            "Default include_within_file should be false"
        );
        assert_eq!(opts.max_clones, 100, "Default max_clones should be 100");
        assert_eq!(opts.max_files, 1000, "Default max_files should be 1000");
        assert!(
            !opts.exclude_generated,
            "Default exclude_generated should be false"
        );
        assert!(!opts.exclude_tests, "Default exclude_tests should be false");
    }

    #[test]
    fn test_clones_options_new_equals_default() {
        let new = ClonesOptions::new();
        let default = ClonesOptions::default();

        assert_eq!(new.min_tokens, default.min_tokens);
        assert_eq!(new.min_lines, default.min_lines);
        assert!((new.threshold - default.threshold).abs() < 1e-9);
        assert_eq!(new.normalization, default.normalization);
        assert_eq!(new.max_clones, default.max_clones);
    }

    // -------------------------------------------------------------------------
    // classify_clone_type thresholds
    // -------------------------------------------------------------------------

    #[test]
    fn test_classify_clone_type_type1() {
        assert_eq!(classify_clone_type(1.0), CloneType::Type1);
        assert_eq!(classify_clone_type(0.9999999999), CloneType::Type1);
    }

    #[test]
    fn test_classify_clone_type_type2() {
        assert_eq!(classify_clone_type(0.95), CloneType::Type2);
        assert_eq!(classify_clone_type(0.9), CloneType::Type2);
        assert_eq!(classify_clone_type(0.9000000001), CloneType::Type2);
    }

    #[test]
    fn test_classify_clone_type_type3() {
        assert_eq!(classify_clone_type(0.89), CloneType::Type3);
        assert_eq!(classify_clone_type(0.7), CloneType::Type3);
        assert_eq!(classify_clone_type(0.5), CloneType::Type3);
    }

    // -------------------------------------------------------------------------
    // CloneType methods
    // -------------------------------------------------------------------------

    #[test]
    fn test_clone_type_as_str() {
        assert_eq!(CloneType::Type1.as_str(), "Type-1");
        assert_eq!(CloneType::Type2.as_str(), "Type-2");
        assert_eq!(CloneType::Type3.as_str(), "Type-3");
    }

    #[test]
    fn test_clone_type_min_similarity() {
        assert!((CloneType::Type1.min_similarity() - 1.0).abs() < 1e-9);
        assert!((CloneType::Type2.min_similarity() - 0.9).abs() < 1e-9);
        assert!((CloneType::Type3.min_similarity() - 0.7).abs() < 1e-9);
    }

    #[test]
    fn test_clone_type_display() {
        assert_eq!(format!("{}", CloneType::Type1), "Type-1");
        assert_eq!(format!("{}", CloneType::Type2), "Type-2");
        assert_eq!(format!("{}", CloneType::Type3), "Type-3");
    }

    // -------------------------------------------------------------------------
    // NormalizationMode methods
    // -------------------------------------------------------------------------

    #[test]
    fn test_normalization_mode_as_str() {
        assert_eq!(NormalizationMode::None.as_str(), "none");
        assert_eq!(NormalizationMode::Identifiers.as_str(), "identifiers");
        assert_eq!(NormalizationMode::Literals.as_str(), "literals");
        assert_eq!(NormalizationMode::All.as_str(), "all");
    }

    #[test]
    fn test_normalization_mode_from_str() {
        assert_eq!(
            NormalizationMode::parse("none"),
            Some(NormalizationMode::None)
        );
        assert_eq!(
            NormalizationMode::parse("identifiers"),
            Some(NormalizationMode::Identifiers)
        );
        assert_eq!(
            NormalizationMode::parse("literals"),
            Some(NormalizationMode::Literals)
        );
        assert_eq!(
            NormalizationMode::parse("all"),
            Some(NormalizationMode::All)
        );
        assert_eq!(NormalizationMode::parse("bogus"), None);
    }

    #[test]
    fn test_normalization_mode_flags() {
        assert!(!NormalizationMode::None.normalize_identifiers());
        assert!(!NormalizationMode::None.normalize_literals());

        assert!(NormalizationMode::Identifiers.normalize_identifiers());
        assert!(!NormalizationMode::Identifiers.normalize_literals());

        assert!(!NormalizationMode::Literals.normalize_identifiers());
        assert!(NormalizationMode::Literals.normalize_literals());

        assert!(NormalizationMode::All.normalize_identifiers());
        assert!(NormalizationMode::All.normalize_literals());
    }

    #[test]
    fn test_normalization_mode_default() {
        assert_eq!(NormalizationMode::default(), NormalizationMode::All);
    }
}

// =============================================================================
// 10. EDGE CASES AND INTEGRATION
// =============================================================================

#[cfg(test)]
mod edge_cases {
    use super::v2_fixtures::*;
    use crate::analysis::clones::{detect_clones, ClonesOptions};

    /// Test: Empty directory returns empty report (not an error).
    #[test]
    fn test_empty_directory_returns_empty_report() {
        let td = V2TestDir::new().unwrap();

        let opts = ClonesOptions::default();
        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(report.clone_pairs.len(), 0);
        assert_eq!(report.stats.files_analyzed, 0);
        assert_eq!(report.stats.clones_found, 0);
    }

    /// Test: Single file with include_within_file=false returns no pairs.
    #[test]
    fn test_single_file_no_within_file() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/only.py", PYTHON_LONG_FUNCTION_A)
            .unwrap();

        let opts = ClonesOptions {
            language: Some("python".to_string()),
            include_within_file: false,
            ..Default::default()
        };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Single file with include_within_file=false should have no pairs"
        );
    }

    /// Test: Files below min_tokens produce no fragments.
    #[test]
    fn test_file_below_min_tokens_no_fragments() {
        let td = V2TestDir::new().unwrap();
        // Very short functions with few tokens
        td.write_file("src/a.py", "def f(): pass\n").unwrap();
        td.write_file("src/b.py", "def f(): pass\n").unwrap();

        let opts = ClonesOptions {
            language: Some("python".to_string()),
            min_tokens: 25, // High threshold
            ..Default::default()
        };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "Files with fewer tokens than min_tokens should produce no pairs"
        );
    }

    /// Test: max_clones limits the number of returned pairs.
    #[test]
    fn test_max_clones_limit() {
        let td = V2TestDir::new().unwrap();
        // Create multiple identical files to generate many clone pairs
        for i in 0..10 {
            td.write_file(&format!("src/file_{}.py", i), PYTHON_LONG_FUNCTION_A)
                .unwrap();
        }

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };
        opts.max_clones = 5;

        let report = detect_clones(td.path(), &opts).unwrap();

        assert!(
            report.clone_pairs.len() <= 5,
            "max_clones=5 should limit output to at most 5 pairs, got {}",
            report.clone_pairs.len()
        );
    }

    /// Test: Stats counts are consistent with clone_pairs.
    #[test]
    fn test_stats_consistency() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();
        td.write_file("src/b.py", PYTHON_LONG_FUNCTION_B).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        let expected_total =
            report.stats.type1_count + report.stats.type2_count + report.stats.type3_count;

        assert_eq!(
            report.stats.clones_found, expected_total,
            "clones_found ({}) should equal type1 + type2 + type3 ({})",
            report.stats.clones_found, expected_total
        );
        assert_eq!(
            report.stats.clones_found,
            report.clone_pairs.len(),
            "clones_found ({}) should equal clone_pairs.len() ({})",
            report.stats.clones_found,
            report.clone_pairs.len()
        );
    }

    /// Test: Clone pair IDs are 1-indexed and sequential.
    #[test]
    fn test_clone_pair_ids_sequential() {
        let td = V2TestDir::new().unwrap();
        for i in 0..5 {
            td.write_file(&format!("src/file_{}.py", i), PYTHON_LONG_FUNCTION_A)
                .unwrap();
        }

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        for (idx, pair) in report.clone_pairs.iter().enumerate() {
            assert_eq!(
                pair.id,
                idx + 1,
                "Clone pair ID should be 1-indexed sequential: expected {}, got {}",
                idx + 1,
                pair.id
            );
        }
    }

    /// Test: Canonical ordering -- fragment1.file <= fragment2.file.
    #[test]
    fn test_canonical_pair_ordering() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/z_file.py", PYTHON_LONG_FUNCTION_A)
            .unwrap();
        td.write_file("src/a_file.py", PYTHON_LONG_FUNCTION_B)
            .unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();

        for pair in &report.clone_pairs {
            if pair.fragment1.file != pair.fragment2.file {
                assert!(
                    pair.fragment1.file <= pair.fragment2.file,
                    "Canonical ordering violated: {:?} should come before {:?}",
                    pair.fragment1.file,
                    pair.fragment2.file
                );
            } else {
                // Same file: fragment1.start_line <= fragment2.start_line
                assert!(
                    pair.fragment1.start_line <= pair.fragment2.start_line,
                    "Same-file canonical ordering violated: start_line {} > {}",
                    pair.fragment1.start_line,
                    pair.fragment2.start_line
                );
            }
        }
    }

    /// Test: Report config snapshot reflects the options used.
    #[test]
    fn test_config_snapshot() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/a.py", PYTHON_LONG_FUNCTION_A).unwrap();

        let opts = ClonesOptions {
            language: Some("python".to_string()),
            min_tokens: 15,
            min_lines: 4,
            threshold: 0.8,
            ..Default::default()
        };

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(report.config.min_tokens, 15);
        assert_eq!(report.config.min_lines, 4);
        assert!((report.config.similarity_threshold - 0.8).abs() < 1e-9);
    }

    /// Test: exclude_tests filters out test files.
    #[test]
    fn test_exclude_tests_option() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/main.py", PYTHON_LONG_FUNCTION_A)
            .unwrap();
        td.write_file("tests/test_main.py", PYTHON_LONG_FUNCTION_B)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };
        opts.exclude_tests = true;

        let report = detect_clones(td.path(), &opts).unwrap();

        // test_main.py is in tests/ directory, should be excluded
        assert_eq!(
            report.clone_pairs.len(),
            0,
            "With exclude_tests=true, test files should be excluded"
        );
    }

    /// Test: exclude_generated filters out generated files.
    #[test]
    fn test_exclude_generated_option() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/main.py", PYTHON_LONG_FUNCTION_A)
            .unwrap();
        td.write_file("generated/types_generated.py", PYTHON_LONG_FUNCTION_B)
            .unwrap();

        let mut opts = crate::analysis::clones::ClonesOptions { language: Some("python".to_string()), min_tokens: 10, min_lines: 3, ..Default::default() };
        opts.exclude_generated = true;

        let report = detect_clones(td.path(), &opts).unwrap();

        assert_eq!(
            report.clone_pairs.len(),
            0,
            "With exclude_generated=true, generated files should be excluded"
        );
    }

    /// Test: detect_clones never panics on invalid/empty content.
    #[test]
    fn test_no_panic_on_empty_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/empty.py", "").unwrap();
        td.write_file("src/comment_only.py", "# just a comment\n")
            .unwrap();
        td.write_file("src/whitespace.py", "\n\n\n\n").unwrap();

        let opts = ClonesOptions {
            language: Some("python".to_string()),
            ..Default::default()
        };

        // This should not panic
        let result = detect_clones(td.path(), &opts);
        assert!(
            result.is_ok(),
            "detect_clones should not panic on empty/comment-only files"
        );
    }
}

// =============================================================================
// 10. MULTI-LANGUAGE FILE DISCOVERY (Bug 1: missing 8+ languages)
// =============================================================================

#[cfg(test)]
mod multi_language_discovery {
    use super::v2_fixtures::*;
    use crate::analysis::clones::filter::{discover_source_files, get_language_from_path};
    use std::path::Path;

    /// Test: C files (.c, .h) are discovered when language="c"
    #[test]
    fn test_discover_c_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/main.c", "int main() { return 0; }")
            .unwrap();
        td.write_file("src/util.h", "void util();").unwrap();

        let files = discover_source_files(td.path(), Some("c"), 100, false, false);
        assert!(
            files.len() >= 2,
            "Expected at least 2 C files (.c, .h), found {}",
            files.len()
        );
    }

    /// Test: C# files (.cs) are discovered when language="csharp"
    #[test]
    fn test_discover_csharp_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/Program.cs", "class Program { static void Main() {} }")
            .unwrap();

        let files = discover_source_files(td.path(), Some("csharp"), 100, false, false);
        assert_eq!(files.len(), 1, "Expected 1 C# file, found {}", files.len());
    }

    /// Test: Elixir files (.ex, .exs) are discovered when language="elixir"
    #[test]
    fn test_discover_elixir_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("lib/app.ex", "defmodule App do\nend")
            .unwrap();
        td.write_file("lib/app_helper.exs", "defmodule AppHelper do\nend")
            .unwrap();

        let files = discover_source_files(td.path(), Some("elixir"), 100, false, false);
        assert!(
            files.len() >= 2,
            "Expected at least 2 Elixir files (.ex, .exs), found {}",
            files.len()
        );
    }

    /// Test: Lua files (.lua) are discovered when language="lua"
    #[test]
    fn test_discover_lua_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/main.lua", "print('hello')").unwrap();

        let files = discover_source_files(td.path(), Some("lua"), 100, false, false);
        assert_eq!(files.len(), 1, "Expected 1 Lua file, found {}", files.len());
    }

    /// Test: OCaml files (.ml, .mli) are discovered when language="ocaml"
    #[test]
    fn test_discover_ocaml_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/main.ml", "let () = print_endline \"hello\"")
            .unwrap();
        td.write_file("src/main.mli", "val main : unit -> unit")
            .unwrap();

        let files = discover_source_files(td.path(), Some("ocaml"), 100, false, false);
        assert!(
            files.len() >= 2,
            "Expected at least 2 OCaml files (.ml, .mli), found {}",
            files.len()
        );
    }

    /// Test: PHP files (.php) are discovered when language="php"
    #[test]
    fn test_discover_php_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/index.php", "<?php echo 'hello'; ?>")
            .unwrap();

        let files = discover_source_files(td.path(), Some("php"), 100, false, false);
        assert_eq!(files.len(), 1, "Expected 1 PHP file, found {}", files.len());
    }

    /// Test: Ruby files (.rb) are discovered when language="ruby"
    #[test]
    fn test_discover_ruby_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("lib/app.rb", "puts 'hello'").unwrap();

        let files = discover_source_files(td.path(), Some("ruby"), 100, false, false);
        assert_eq!(
            files.len(),
            1,
            "Expected 1 Ruby file, found {}",
            files.len()
        );
    }

    /// Test: Scala files (.scala) are discovered when language="scala"
    #[test]
    fn test_discover_scala_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file(
            "src/Main.scala",
            "object Main { def main(args: Array[String]) = {} }",
        )
        .unwrap();

        let files = discover_source_files(td.path(), Some("scala"), 100, false, false);
        assert_eq!(
            files.len(),
            1,
            "Expected 1 Scala file, found {}",
            files.len()
        );
    }

    /// Test: Swift files (.swift) are discovered when language="swift"
    #[test]
    fn test_discover_swift_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("Sources/main.swift", "print(\"hello\")")
            .unwrap();

        let files = discover_source_files(td.path(), Some("swift"), 100, false, false);
        assert_eq!(
            files.len(),
            1,
            "Expected 1 Swift file, found {}",
            files.len()
        );
    }

    /// Test: Kotlin files (.kt) are discovered when language="kotlin"
    #[test]
    fn test_discover_kotlin_files() {
        let td = V2TestDir::new().unwrap();
        td.write_file("src/Main.kt", "fun main() { println(\"hello\") }")
            .unwrap();

        let files = discover_source_files(td.path(), Some("kotlin"), 100, false, false);
        assert_eq!(
            files.len(),
            1,
            "Expected 1 Kotlin file, found {}",
            files.len()
        );
    }

    /// Test: Comprehensive check -- all 18 corpus languages have file extension support.
    #[test]
    fn test_discover_all_supported_extensions() {
        let td = V2TestDir::new().unwrap();

        // Create one file per language
        let language_files: Vec<(&str, &str, &str)> = vec![
            ("python", "a.py", "def f(): pass"),
            ("typescript", "a.ts", "function f() {}"),
            ("javascript", "a.js", "function f() {}"),
            ("go", "a.go", "package main\nfunc main() {}"),
            ("rust", "a.rs", "fn main() {}"),
            ("java", "A.java", "class A { void f() {} }"),
            ("c", "a.c", "int main() { return 0; }"),
            ("csharp", "a.cs", "class A { void F() {} }"),
            ("elixir", "a.ex", "defmodule A do\nend"),
            ("lua", "a.lua", "print('hello')"),
            ("ocaml", "a.ml", "let () = ()"),
            ("php", "a.php", "<?php echo 1; ?>"),
            ("ruby", "a.rb", "puts 'hello'"),
            ("scala", "a.scala", "object A {}"),
            ("swift", "a.swift", "print(\"hello\")"),
            ("kotlin", "a.kt", "fun main() {}"),
        ];

        for (lang, filename, content) in &language_files {
            td.write_file(&format!("src/{}", filename), content)
                .unwrap();
            let files = discover_source_files(td.path(), Some(lang), 100, false, false);
            assert!(
                !files.is_empty(),
                "Language '{}' with extension '{}' should discover at least 1 file, found 0",
                lang,
                filename
            );
        }
    }

    /// Test: get_language_from_path works for all new extensions
    #[test]
    fn test_get_language_from_path_all_languages() {
        let cases = vec![
            ("test.py", "python"),
            ("test.ts", "typescript"),
            ("test.js", "javascript"),
            ("test.go", "go"),
            ("test.rs", "rust"),
            ("test.java", "java"),
            ("test.c", "c"),
            ("test.h", "c"),
            ("test.cs", "csharp"),
            ("test.ex", "elixir"),
            ("test.exs", "elixir"),
            ("test.lua", "lua"),
            ("test.ml", "ocaml"),
            ("test.mli", "ocaml"),
            ("test.php", "php"),
            ("test.rb", "ruby"),
            ("test.scala", "scala"),
            ("test.swift", "swift"),
            ("test.kt", "kotlin"),
        ];

        for (path_str, expected_lang) in cases {
            let lang = get_language_from_path(Path::new(path_str));
            assert_eq!(
                lang,
                Some(expected_lang),
                "get_language_from_path('{}') should return Some('{}'), got {:?}",
                path_str,
                expected_lang,
                lang
            );
        }
    }
}

// =============================================================================
// 11. MULTI-LANGUAGE FUNCTION EXTRACTION (Bug 1: missing node types)
// =============================================================================

#[cfg(test)]
mod multi_language_function_extraction {
    use super::v2_fixtures::*;
    use crate::analysis::clones::detect_clones;

    /// Test: C functions are detected as clone pairs between identical files
    #[test]
    fn test_function_extraction_c() {
        let td = V2TestDir::new().unwrap();
        let c_source = r#"
#include <stdio.h>

int add(int a, int b) {
    int result = a + b;
    return result;
}

int multiply(int a, int b) {
    int result = a * b;
    return result;
}
"#;
        td.write_file("src/a.c", c_source).unwrap();
        td.write_file("src/b.c", c_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("c".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical C files, found 0. \
             Likely missing C function extraction support."
        );
    }

    /// Test: Ruby methods are detected as clone pairs between identical files
    #[test]
    fn test_function_extraction_ruby() {
        let td = V2TestDir::new().unwrap();
        let ruby_source = r#"
def process_data(input)
  result = []
  input.each do |item|
    result << item.to_s
  end
  result
end

def transform_data(input)
  output = []
  input.each do |item|
    output << item.to_i
  end
  output
end
"#;
        td.write_file("lib/a.rb", ruby_source).unwrap();
        td.write_file("lib/b.rb", ruby_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("ruby".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical Ruby files, found 0. \
             Likely missing Ruby method extraction support."
        );
    }

    /// Test: PHP functions are detected
    #[test]
    fn test_function_extraction_php() {
        let td = V2TestDir::new().unwrap();
        let php_source = r#"<?php
function processData($input) {
    $result = [];
    foreach ($input as $item) {
        $result[] = $item * 2;
    }
    return $result;
}

function transformData($input) {
    $result = [];
    foreach ($input as $item) {
        $result[] = $item + 1;
    }
    return $result;
}
"#;
        td.write_file("src/a.php", php_source).unwrap();
        td.write_file("src/b.php", php_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("php".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical PHP files, found 0. \
             Likely missing PHP function extraction support."
        );
    }

    /// Test: Swift functions are detected
    #[test]
    fn test_function_extraction_swift() {
        let td = V2TestDir::new().unwrap();
        let swift_source = r#"
import Foundation

func processData(input: [Int]) -> [Int] {
    var result: [Int] = []
    for item in input {
        result.append(item * 2)
    }
    return result
}

func transformData(input: [Int]) -> [Int] {
    var result: [Int] = []
    for item in input {
        result.append(item + 1)
    }
    return result
}
"#;
        td.write_file("Sources/a.swift", swift_source).unwrap();
        td.write_file("Sources/b.swift", swift_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("swift".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical Swift files, found 0. \
             Likely missing Swift function extraction support."
        );
    }

    /// Test: Kotlin functions are detected
    #[test]
    fn test_function_extraction_kotlin() {
        let td = V2TestDir::new().unwrap();
        let kotlin_source = r#"
fun processData(input: List<Int>): List<Int> {
    val result = mutableListOf<Int>()
    for (item in input) {
        result.add(item * 2)
    }
    return result
}

fun transformData(input: List<Int>): List<Int> {
    val result = mutableListOf<Int>()
    for (item in input) {
        result.add(item + 1)
    }
    return result
}
"#;
        td.write_file("src/a.kt", kotlin_source).unwrap();
        td.write_file("src/b.kt", kotlin_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("kotlin".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical Kotlin files, found 0. \
             Likely missing Kotlin function extraction support."
        );
    }

    /// Test: Scala functions are detected
    #[test]
    fn test_function_extraction_scala() {
        let td = V2TestDir::new().unwrap();
        let scala_source = r#"
object DataProcessor {
  def processData(input: List[Int]): List[Int] = {
    val result = input.map(_ * 2)
    result.filter(_ > 0)
  }

  def transformData(input: List[Int]): List[Int] = {
    val result = input.map(_ + 1)
    result.filter(_ > 0)
  }
}
"#;
        td.write_file("src/a.scala", scala_source).unwrap();
        td.write_file("src/b.scala", scala_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("scala".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical Scala files, found 0. \
             Likely missing Scala function extraction support."
        );
    }

    /// Test: C# methods are detected
    #[test]
    fn test_function_extraction_csharp() {
        let td = V2TestDir::new().unwrap();
        let csharp_source = r#"
using System;
using System.Collections.Generic;

public class DataProcessor {
    public List<int> ProcessData(List<int> input) {
        var result = new List<int>();
        foreach (var item in input) {
            result.Add(item * 2);
        }
        return result;
    }

    public List<int> TransformData(List<int> input) {
        var result = new List<int>();
        foreach (var item in input) {
            result.Add(item + 1);
        }
        return result;
    }
}
"#;
        td.write_file("src/a.cs", csharp_source).unwrap();
        td.write_file("src/b.cs", csharp_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("csharp".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical C# files, found 0. \
             Likely missing C# method extraction support."
        );
    }

    /// Test: Lua functions are detected
    #[test]
    fn test_function_extraction_lua() {
        let td = V2TestDir::new().unwrap();
        let lua_source = r#"
function processData(input)
    local result = {}
    for i, item in ipairs(input) do
        result[i] = item * 2
    end
    return result
end

function transformData(input)
    local result = {}
    for i, item in ipairs(input) do
        result[i] = item + 1
    end
    return result
end
"#;
        td.write_file("src/a.lua", lua_source).unwrap();
        td.write_file("src/b.lua", lua_source).unwrap();

        let opts = crate::analysis::clones::ClonesOptions { language: Some("lua".to_string()), min_tokens: 5, min_lines: 3, ..Default::default() };

        let report = detect_clones(td.path(), &opts).unwrap();
        assert!(
            !report.clone_pairs.is_empty(),
            "Expected clone pairs between identical Lua files, found 0. \
             Likely missing Lua function extraction support."
        );
    }
}
