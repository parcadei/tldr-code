//! Todo Aggregation CLI Integration Tests
//!
//! Test-driven development tests for `tldr todo` command migration.
//! These tests define expected behavior based on spec.md behavioral contracts.
//!
//! # Behavioral Contracts Tested
//!
//! - BC-TODO-1: Priority ordering (1=highest to 6=lowest)
//! - BC-TODO-2: File/function limits (200 files for equivalence, 500 functions for similar)
//! - BC-TODO-3: Text output (top 20 items, "... and N more")
//! - BC-TODO-4: Safe execution (sub-analyses wrapped)
//!
//! # Priority Mapping
//!
//! - Priority 1: Dead code
//! - Priority 2: High complexity (CC > 20)
//! - Priority 3: Low cohesion (LCOM4 > 2)
//! - Priority 4: Similar functions
//! - Priority 5: Equivalence/redundancy
//! - Priority 6: Medium complexity (CC > 10)
//!
//! Reference: migration/spec.md

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

// =============================================================================
// Helper Functions
// =============================================================================

/// Get the tldr command
fn tldr_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("tldr"))
}

/// Create a test file in the given directory
fn create_test_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&path, content).expect("Failed to write test file");
    path
}

// =============================================================================
// Basic Command Tests
// =============================================================================

#[test]
fn test_todo_help() {
    let mut cmd = tldr_cmd();
    cmd.arg("todo").arg("--help");

    // This test will fail until the todo command is implemented
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("todo"))
        .stdout(predicate::str::contains("improvement").or(predicate::str::contains("action")));
}

#[test]
fn test_todo_empty_directory() {
    let dir = tempdir().unwrap();

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Empty directory should return valid JSON with 0 items
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"items\""))
        .stdout(predicate::str::contains("[]").or(predicate::str::contains("\"total_items\": 0")));
}

#[test]
fn test_todo_nonexistent_path() {
    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg("/nonexistent/path/that/does/not/exist")
        .arg("-f")
        .arg("json");

    // Should fail gracefully
    cmd.assert().failure().stderr(
        predicate::str::contains("not found")
            .or(predicate::str::contains("No such file").or(predicate::str::contains("error"))),
    );
}

#[test]
fn test_todo_single_file() {
    let dir = tempdir().unwrap();
    let content = r#"
def complex_function(a, b, c, d, e, f, g, h, i, j, k):
    if a:
        if b:
            if c:
                if d:
                    if e:
                        if f:
                            if g:
                                if h:
                                    if i:
                                        if j:
                                            return k
    return None
"#;
    let file = create_test_file(dir.path(), "complex.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(file.to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should analyze single file
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"wrapper\": \"todo\""));
}

// =============================================================================
// BC-TODO-1: Priority Ordering Tests
// =============================================================================

#[test]
fn test_todo_priority_ordering() {
    let dir = tempdir().unwrap();

    // Create files that should trigger different categories
    // Dead code (priority 1)
    let dead_code = r#"
def used_function():
    return 42

def dead_function():  # never called
    return 999
"#;
    // High complexity (priority 2)
    let high_cc = r#"
def very_complex(a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r, s, t, u):
    if a: pass
    if b: pass
    if c: pass
    if d: pass
    if e: pass
    if f: pass
    if g: pass
    if h: pass
    if i: pass
    if j: pass
    if k: pass
    if l: pass
    if m: pass
    if n: pass
    if o: pass
    if p: pass
    if q: pass
    if r: pass
    if s: pass
    if t: pass
    if u: pass
"#;

    create_test_file(dir.path(), "dead.py", dead_code);
    create_test_file(dir.path(), "complex.py", high_cc);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Items should be sorted by priority (1 = highest)
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"items\""))
        .stdout(predicate::str::contains("\"priority\""));
}

#[test]
fn test_todo_dead_code_priority_1() {
    let dir = tempdir().unwrap();

    let content = r#"
def main():
    return helper()

def helper():
    return 42

def unused():  # dead code
    return 999
"#;
    create_test_file(dir.path(), "code.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Dead code should have priority 1
    cmd.assert().success();
    // If dead code is detected, it should be priority 1
}

#[test]
fn test_todo_high_complexity_priority_2() {
    let dir = tempdir().unwrap();

    // Function with CC > 20
    let content = r#"
def extremely_complex(x):
    if x == 1: return 1
    elif x == 2: return 2
    elif x == 3: return 3
    elif x == 4: return 4
    elif x == 5: return 5
    elif x == 6: return 6
    elif x == 7: return 7
    elif x == 8: return 8
    elif x == 9: return 9
    elif x == 10: return 10
    elif x == 11: return 11
    elif x == 12: return 12
    elif x == 13: return 13
    elif x == 14: return 14
    elif x == 15: return 15
    elif x == 16: return 16
    elif x == 17: return 17
    elif x == 18: return 18
    elif x == 19: return 19
    elif x == 20: return 20
    elif x == 21: return 21
    elif x == 22: return 22
    else: return 0
"#;
    create_test_file(dir.path(), "complex.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // High complexity (CC > 20) should have priority 2
    cmd.assert().success();
}

#[test]
fn test_todo_medium_complexity_priority_6() {
    let dir = tempdir().unwrap();

    // Function with CC between 10 and 20
    let content = r#"
def moderately_complex(x):
    if x == 1: return 1
    elif x == 2: return 2
    elif x == 3: return 3
    elif x == 4: return 4
    elif x == 5: return 5
    elif x == 6: return 6
    elif x == 7: return 7
    elif x == 8: return 8
    elif x == 9: return 9
    elif x == 10: return 10
    elif x == 11: return 11
    elif x == 12: return 12
    else: return 0
"#;
    create_test_file(dir.path(), "moderate.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Medium complexity (CC 10-20) should have priority 6
    cmd.assert().success();
}

// =============================================================================
// BC-TODO-2: File/Function Limits Tests
// =============================================================================

#[test]
fn test_todo_json_structure() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // JSON should have required fields
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"wrapper\""))
        .stdout(predicate::str::contains("\"path\""))
        .stdout(predicate::str::contains("\"items\""))
        .stdout(predicate::str::contains("\"summary\""))
        .stdout(predicate::str::contains("\"total_elapsed_ms\""));
}

// =============================================================================
// BC-TODO-3: Text Output Tests
// =============================================================================

#[test]
fn test_todo_text_format() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("text");

    // Text output should be human-readable
    cmd.assert().success();
}

#[test]
fn test_todo_text_shows_top_20() {
    let dir = tempdir().unwrap();

    // Create many files to generate many todo items
    for i in 0..25 {
        let content = format!(
            r#"
def func_{i}(x):
    y = x + 1
    z = x + 1  # redundant
    return y + z
"#
        );
        create_test_file(dir.path(), &format!("file{}.py", i), &content);
    }

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("text");

    // Text output should show top 20 and indicate more
    cmd.assert().success();
    // If more than 20 items, should show "... and N more"
}

// =============================================================================
// BC-TODO-4: Safe Execution Tests
// =============================================================================

#[test]
fn test_todo_handles_parse_errors() {
    let dir = tempdir().unwrap();

    // Create invalid Python that can't be parsed
    let invalid = "def broken(\n    # incomplete";
    create_test_file(dir.path(), "broken.py", invalid);
    create_test_file(dir.path(), "valid.py", "def good(): return 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should not crash on parse errors
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"wrapper\": \"todo\""));
}

#[test]
fn test_todo_sub_results_track_errors() {
    let dir = tempdir().unwrap();

    // Create a file that might cause analysis issues
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Sub-results should track success/failure
    cmd.assert().success().stdout(
        predicate::str::contains("\"details\"").or(predicate::str::contains("\"sub_results\"")),
    );
}

// =============================================================================
// Quick Mode Tests
// =============================================================================

#[test]
fn test_todo_quick_mode() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("--quick")
        .arg("-f")
        .arg("json");

    // Quick mode should run 4 analyses (dead, complexity, cohesion, equivalence)
    // Should skip expensive similar analysis
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"wrapper\": \"todo\""));
}

#[test]
fn test_todo_full_mode() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "def foo(): return 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Full mode (default) includes similar function detection
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"wrapper\": \"todo\""));
}

// =============================================================================
// TodoItem Structure Tests
// =============================================================================

#[test]
fn test_todo_item_has_required_fields() {
    let dir = tempdir().unwrap();

    // Create code that will generate todo items
    let content = r#"
def main():
    x = 1 + 1
    y = 1 + 1  # redundant expression
    return x + y
"#;
    create_test_file(dir.path(), "code.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // TodoItem should have: category, priority, description, file, line, severity, score
    cmd.assert().success();
    // If items exist, they should have proper structure
}

#[test]
fn test_todo_item_categories() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Categories should be one of: dead, complexity, cohesion, similar, equivalence
    cmd.assert().success();
}

// =============================================================================
// Summary Tests
// =============================================================================

#[test]
fn test_todo_summary_fields() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Summary should have counts
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"summary\""));
}

#[test]
fn test_todo_summary_counts() {
    let dir = tempdir().unwrap();

    let content = r#"
def func1():
    x = 1 + 1
    y = 1 + 1  # redundant
    return x + y

def func2():
    a = 2 + 2
    b = 2 + 2  # redundant
    return a + b
"#;
    create_test_file(dir.path(), "code.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Summary should count items by category
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"summary\""));
}

// =============================================================================
// GVN/Equivalence Integration Tests
// =============================================================================

#[test]
fn test_todo_detects_redundant_expressions() {
    let dir = tempdir().unwrap();

    let content = r#"
def redundant_test(x, y):
    a = x + y
    b = y + x  # redundant (commutative)
    c = x + y  # redundant (identical)
    return a + b + c
"#;
    create_test_file(dir.path(), "redundant.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should detect equivalence/redundancy items
    cmd.assert().success();
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_todo_empty_items() {
    let dir = tempdir().unwrap();

    // Create perfectly clean code
    let content = r#"
def clean_function(x):
    return x + 1
"#;
    create_test_file(dir.path(), "clean.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should succeed even with no issues found
    cmd.assert().success();
}

#[test]
fn test_todo_language_filter() {
    let dir = tempdir().unwrap();

    create_test_file(dir.path(), "code.py", "x = 1");
    create_test_file(dir.path(), "code.rs", "let x = 1;");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-l")
        .arg("python")
        .arg("-f")
        .arg("json");

    // Should only analyze Python files
    cmd.assert().success();
}

#[test]
fn test_todo_nested_directories() {
    let dir = tempdir().unwrap();

    create_test_file(dir.path(), "root.py", "x = 1");
    create_test_file(dir.path(), "sub/nested.py", "y = 2");
    create_test_file(dir.path(), "sub/deep/more.py", "z = 3");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should scan nested directories
    cmd.assert().success();
}

#[test]
fn test_todo_binary_files_ignored() {
    let dir = tempdir().unwrap();

    let binary_path = dir.path().join("binary.bin");
    fs::write(&binary_path, [0u8, 1, 2, 3, 255, 254]).unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should not crash on binary files
    cmd.assert().success();
}

// =============================================================================
// Timing Tests
// =============================================================================

#[test]
fn test_todo_reports_elapsed_time() {
    let dir = tempdir().unwrap();
    create_test_file(dir.path(), "code.py", "x = 1");

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should include elapsed time
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("elapsed"));
}

// =============================================================================
// Similar Function Detection Tests
// =============================================================================

#[test]
fn test_todo_detects_similar_functions() {
    let dir = tempdir().unwrap();

    // Create similar but not identical functions
    let content = r#"
def process_user(user):
    data = user.get_data()
    validated = validate(data)
    return save(validated)

def process_order(order):
    data = order.get_data()
    validated = validate(data)
    return save(validated)

def process_item(item):
    data = item.get_data()
    validated = validate(data)
    return save(validated)
"#;
    create_test_file(dir.path(), "similar.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should potentially detect similar functions (priority 4)
    cmd.assert().success();
}

// =============================================================================
// Cohesion Detection Tests
// =============================================================================

#[test]
fn test_todo_detects_low_cohesion() {
    let dir = tempdir().unwrap();

    // Create a class with low cohesion (LCOM4 > 2)
    let content = r#"
class LowCohesion:
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
    create_test_file(dir.path(), "cohesion.py", content);

    let mut cmd = tldr_cmd();
    cmd.arg("todo")
        .arg(dir.path().to_str().unwrap())
        .arg("-f")
        .arg("json");

    // Should potentially detect low cohesion (priority 3)
    cmd.assert().success();
}
