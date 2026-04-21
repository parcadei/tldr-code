//! Token-Level (L1) Diff Tests
//!
//! These tests define the expected behavior for L1 token-level diff using
//! difftastic's graph-based algorithm. They assert that `DiffGranularity::Token`
//! produces individual token-level changes (inserts, deletes, updates for each
//! keyword, identifier, operator) rather than function-level changes.
//!
//! These tests WILL FAIL until the L1 implementation is complete (currently
//! Token granularity falls through to L4 function-level diff).

use std::io::Write;

use tempfile::NamedTempFile;

use tldr_cli::commands::remaining::diff::DiffArgs;
use tldr_cli::commands::remaining::types::{ChangeType, DiffGranularity, DiffReport, NodeKind};

// =============================================================================
// Helpers
// =============================================================================

/// Create a temp file with given content and file extension suffix.
fn write_temp(content: &str, suffix: &str) -> NamedTempFile {
    let mut f = NamedTempFile::with_suffix(suffix).unwrap();
    write!(f, "{}", content).unwrap();
    f.flush().unwrap();
    f
}

/// Run L1 (token-level) diff and return the report.
fn run_l1_diff(file_a: &NamedTempFile, file_b: &NamedTempFile) -> DiffReport {
    let args = DiffArgs {
        file_a: file_a.path().to_path_buf(),
        file_b: file_b.path().to_path_buf(),
        granularity: DiffGranularity::Token,
        semantic_only: false,
        output: None,
    };
    args.run_to_report().expect("L1 diff should succeed")
}

/// Count changes of a specific type in the report.
fn count_changes(report: &DiffReport, change_type: ChangeType) -> usize {
    report
        .changes
        .iter()
        .filter(|c| c.change_type == change_type)
        .count()
}

/// Assert that a report has at least one change of the given type.
fn assert_has_change(report: &DiffReport, change_type: ChangeType, context: &str) {
    let found = report.changes.iter().any(|c| c.change_type == change_type);
    assert!(
        found,
        "{}: expected at least one {:?} change, but found none. Changes: {:?}",
        context,
        change_type,
        report
            .changes
            .iter()
            .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
            .collect::<Vec<_>>()
    );
}

/// Assert that no change in the report has children (L1 is flat).
fn assert_flat_changes(report: &DiffReport, context: &str) {
    for change in &report.changes {
        assert!(
            change.children.is_none() || change.children.as_ref().unwrap().is_empty(),
            "{}: L1 changes should be flat (no children), but found children on {:?}",
            context,
            change.name,
        );
    }
}

/// Assert that every change has NodeKind::Expression (per spec, L1 uses
/// Expression for all token changes).
fn assert_all_expression_kind(report: &DiffReport, context: &str) {
    for change in &report.changes {
        assert_eq!(
            change.node_kind,
            NodeKind::Expression,
            "{}: L1 changes should all have NodeKind::Expression, but found {:?} on {:?}",
            context,
            change.node_kind,
            change.name,
        );
    }
}

/// Assert that report granularity is Token.
fn assert_token_granularity(report: &DiffReport, context: &str) {
    assert_eq!(
        report.granularity,
        DiffGranularity::Token,
        "{}: expected Token granularity, got {:?}",
        context,
        report.granularity,
    );
}

/// Find all changes of a given type.
fn changes_of_type(
    report: &DiffReport,
    change_type: ChangeType,
) -> Vec<&tldr_cli::commands::remaining::types::ASTChange> {
    report
        .changes
        .iter()
        .filter(|c| c.change_type == change_type)
        .collect()
}

// =============================================================================
// Basic Token Tests
// =============================================================================

mod basic {
    use super::*;

    /// Test 1: Two identical files should produce identical=true with no changes.
    #[test]
    fn identical_files_token_level() {
        let content = "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n";
        let a = write_temp(content, ".py");
        let b = write_temp(content, ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "identical files");
        assert!(
            report.identical,
            "Identical files should produce identical=true. Got {} changes: {:?}",
            report.changes.len(),
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert!(
            report.changes.is_empty(),
            "Identical files should have 0 changes, got {}",
            report.changes.len()
        );
    }

    /// Test 2: File B has one extra token (an added variable assignment).
    /// Should produce at least one Insert change at the token level.
    #[test]
    fn single_token_insert() {
        let a = write_temp("def foo(x):\n    return x\n", ".py");
        // Add a single new statement (multiple tokens, but the key assertion is
        // that we get Insert changes at the TOKEN level, not function level)
        let b = write_temp("def foo(x):\n    y = 1\n    return x\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "single token insert");
        assert!(!report.identical, "Files differ, should not be identical");
        assert_has_change(&report, ChangeType::Insert, "single token insert");

        // L1 should produce token-level inserts (individual tokens like "y", "=", "1")
        // NOT a single function-level Update change.
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "Expected at least 3 token-level Insert changes (y, =, 1), got {}. \
             If this is 0 and there's 1 Update, L1 is falling through to L4. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "single token insert");
        assert_all_expression_kind(&report, "single token insert");
    }

    /// Test 3: File B is missing one token (a deleted statement).
    /// Should produce at least one Delete change at the token level.
    #[test]
    fn single_token_delete() {
        let a = write_temp("def foo(x):\n    y = 1\n    return x\n", ".py");
        let b = write_temp("def foo(x):\n    return x\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "single token delete");
        assert!(!report.identical, "Files differ, should not be identical");
        assert_has_change(&report, ChangeType::Delete, "single token delete");

        // L1 should produce token-level deletes
        let deletes = count_changes(&report, ChangeType::Delete);
        assert!(
            deletes >= 3,
            "Expected at least 3 token-level Delete changes (y, =, 1), got {}. Changes: {:?}",
            deletes,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "single token delete");
        assert_all_expression_kind(&report, "single token delete");
    }

    /// Test 4: Variable name changed (token replacement).
    /// For non-comment/non-string tokens, this should produce Delete + Insert
    /// (difftastic marks renamed identifiers as Novel on both sides).
    #[test]
    fn single_token_update() {
        let a = write_temp("def foo(x):\n    return x + 1\n", ".py");
        let b = write_temp("def foo(x):\n    return x + 99\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "single token update");
        assert!(!report.identical, "Files differ, should not be identical");

        // The literal "1" becomes "99" -- this is a normal atom (not comment/string),
        // so difftastic should mark old "1" as Novel(LHS) = Delete, new "99" as
        // Novel(RHS) = Insert. There should be exactly 1 delete and 1 insert for
        // the changed token.
        let deletes = count_changes(&report, ChangeType::Delete);
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            deletes >= 1,
            "Expected at least 1 Delete for old token '1', got {}. Changes: {:?}",
            deletes,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert!(
            inserts >= 1,
            "Expected at least 1 Insert for new token '99', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // The total should be small -- just the changed tokens, not the entire function
        assert!(
            report.changes.len() <= 4,
            "Expected at most 4 token-level changes for a single literal change, got {}. \
             If this is 1 Update of a whole function, L1 is falling through to L4.",
            report.changes.len()
        );

        assert_flat_changes(&report, "single token update");
        assert_all_expression_kind(&report, "single token update");
    }

    /// Test 5: Multiple token changes across the file.
    /// Several insertions and deletions should produce the correct count.
    #[test]
    fn multiple_token_changes() {
        let a = write_temp(
            "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n",
            ".py",
        );
        let b = write_temp(
            "def foo(x):\n    return x + 99\n\ndef bar(y):\n    return y * 3\n\ndef baz(z):\n    return z\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "multiple token changes");
        assert!(!report.identical, "Files differ, should not be identical");

        // We should have token-level changes for:
        // - "1" -> "99" (delete "1", insert "99")
        // - "2" -> "3" (delete "2", insert "3")
        // - entire "def baz(z):\n    return z\n" as inserts
        let total = report.changes.len();
        assert!(
            total >= 6,
            "Expected at least 6 token-level changes (2 pairs of delete+insert for \
             literals, plus several inserts for new function), got {}. Changes: {:?}",
            total,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // Check that we have both inserts and deletes
        assert_has_change(&report, ChangeType::Insert, "multiple changes: inserts");
        assert_has_change(&report, ChangeType::Delete, "multiple changes: deletes");

        assert_flat_changes(&report, "multiple token changes");
        assert_all_expression_kind(&report, "multiple token changes");
    }

    /// Test 6: Only whitespace/formatting changes.
    /// Since difftastic's syntax tree does not represent whitespace tokens,
    /// pure whitespace changes should produce identical=true with no changes.
    #[test]
    fn whitespace_only_changes() {
        // Same code, different formatting (extra blank lines, indentation style)
        let a = write_temp(
            "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n",
            ".py",
        );
        let b = write_temp(
            "def foo(x):\n    return x + 1\n\n\n\ndef bar(y):\n    return y * 2\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "whitespace only");

        // Whitespace-only changes should not produce semantic changes.
        // The structural diff should see the same tokens.
        let semantic_changes: Vec<_> = report
            .changes
            .iter()
            .filter(|c| c.change_type != ChangeType::Format)
            .collect();
        assert!(
            semantic_changes.is_empty(),
            "Whitespace-only changes should produce no semantic changes, got {}. Changes: {:?}",
            semantic_changes.len(),
            semantic_changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Language-Specific Token Tests
// =============================================================================

mod language_specific {
    use super::*;

    /// Test 7: Python function with body change produces token-level changes.
    #[test]
    fn python_token_diff() {
        let a = write_temp(
            "def compute(x, y):\n    result = x + y\n    return result\n",
            ".py",
        );
        let b = write_temp(
            "def compute(x, y):\n    result = x * y + 1\n    return result\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Python token diff");
        assert!(!report.identical, "Python: files should differ");

        // The change is "x + y" -> "x * y + 1"
        // Token-level should detect: delete "+", insert "*", insert "+", insert "1"
        // (or similar -- exact token breakdown depends on tree-sitter parse)
        let total = report.changes.len();
        assert!(
            total >= 2,
            "Python: expected at least 2 token-level changes for operator change, got {}. \
             If got 1 Update for the whole function, L1 is falling through to L4. Changes: {:?}",
            total,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Python token diff");
        assert_all_expression_kind(&report, "Python token diff");
    }

    /// Test 8: Rust function with type annotation change.
    #[test]
    fn rust_token_diff() {
        let a = write_temp("fn compute(x: i32, y: i32) -> i32 {\n    x + y\n}\n", ".rs");
        let b = write_temp("fn compute(x: i64, y: i64) -> i64 {\n    x + y\n}\n", ".rs");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Rust token diff");
        assert!(!report.identical, "Rust: files should differ");

        // Three type annotations changed: i32 -> i64 (x3)
        // Each produces a Delete("i32") + Insert("i64")
        let deletes = count_changes(&report, ChangeType::Delete);
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            deletes >= 3,
            "Rust: expected at least 3 Deletes for old type 'i32', got {}. Changes: {:?}",
            deletes,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert!(
            inserts >= 3,
            "Rust: expected at least 3 Inserts for new type 'i64', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Rust token diff");
        assert_all_expression_kind(&report, "Rust token diff");
    }

    /// Test 9: TypeScript with added type parameter.
    #[test]
    fn typescript_token_diff() {
        let a = write_temp(
            "function identity(x: number): number {\n    return x;\n}\n",
            ".ts",
        );
        let b = write_temp("function identity<T>(x: T): T {\n    return x;\n}\n", ".ts");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "TypeScript token diff");
        assert!(!report.identical, "TypeScript: files should differ");

        // Changes include: insert "<", "T", ">", delete "number" (x2), insert "T" (x2)
        let total = report.changes.len();
        assert!(
            total >= 4,
            "TypeScript: expected at least 4 token-level changes for type parameter addition, \
             got {}. Changes: {:?}",
            total,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_has_change(&report, ChangeType::Insert, "TypeScript: new tokens");
        assert_has_change(&report, ChangeType::Delete, "TypeScript: removed tokens");
        assert_flat_changes(&report, "TypeScript token diff");
        assert_all_expression_kind(&report, "TypeScript token diff");
    }

    /// Test 10: C++ with added statement.
    #[test]
    fn cpp_token_diff() {
        let a = write_temp(
            "#include <iostream>\nint main() {\n    std::cout << \"hello\";\n    return 0;\n}\n",
            ".cpp",
        );
        let b = write_temp(
            "#include <iostream>\nint main() {\n    int x = 42;\n    std::cout << \"hello\";\n    return 0;\n}\n",
            ".cpp",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "C++ token diff");
        assert!(!report.identical, "C++: files should differ");

        // Added `int x = 42;` -- should produce several token-level Insert changes
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "C++: expected at least 3 token-level Insert changes for 'int x = 42', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "C++ token diff");
        assert_all_expression_kind(&report, "C++ token diff");
    }

    /// Test 11: Kotlin with changed value.
    /// NOTE: Kotlin uses tree-sitter-kotlin-ng which may have different node kinds
    /// than difftastic's expectations. If this test fails with parsing issues,
    /// it indicates a grammar mismatch.
    #[test]
    fn kotlin_token_diff() {
        let a = write_temp("fun main() {\n    val x = 1\n    println(x)\n}\n", ".kt");
        let b = write_temp("fun main() {\n    val x = 99\n    println(x)\n}\n", ".kt");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Kotlin token diff");
        assert!(!report.identical, "Kotlin: files should differ");

        // The change is "1" -> "99" -- should produce Delete + Insert at token level
        let deletes = count_changes(&report, ChangeType::Delete);
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            deletes >= 1,
            "Kotlin: expected at least 1 Delete for old token '1', got {}. Changes: {:?}",
            deletes,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert!(
            inserts >= 1,
            "Kotlin: expected at least 1 Insert for new token '99', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Kotlin token diff");
        assert_all_expression_kind(&report, "Kotlin token diff");
    }

    /// Test 12: Swift with added line.
    #[test]
    fn swift_token_diff() {
        let a = write_temp("func greet() {\n    print(\"hello\")\n}\n", ".swift");
        let b = write_temp(
            "func greet() {\n    let name = \"world\"\n    print(\"hello\")\n}\n",
            ".swift",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Swift token diff");
        assert!(!report.identical, "Swift: files should differ");

        // Added `let name = "world"` -- should produce token-level inserts
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "Swift: expected at least 3 token-level Insert changes for 'let name = \"world\"', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Swift token diff");
        assert_all_expression_kind(&report, "Swift token diff");
    }

    /// Test 13: C# with changed string.
    #[test]
    fn csharp_token_diff() {
        let a = write_temp(
            "using System;\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"hello\");\n    }\n}\n",
            ".cs",
        );
        let b = write_temp(
            "using System;\nclass Program {\n    static void Main() {\n        Console.WriteLine(\"goodbye\");\n    }\n}\n",
            ".cs",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "C# token diff");
        assert!(!report.identical, "C#: files should differ");

        // The string changed from "hello" to "goodbye" -- should produce an Update
        // (ReplacedString) or Delete+Insert at token level
        assert!(
            !report.changes.is_empty(),
            "C#: should have at least one change for string replacement. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "C# token diff");
        assert_all_expression_kind(&report, "C# token diff");
    }

    /// Test 14: Scala with added line.
    #[test]
    fn scala_token_diff() {
        let a = write_temp(
            "object Main {\n  def main(args: Array[String]): Unit = {\n    println(\"hello\")\n  }\n}\n",
            ".scala",
        );
        let b = write_temp(
            "object Main {\n  def main(args: Array[String]): Unit = {\n    val x = 42\n    println(\"hello\")\n  }\n}\n",
            ".scala",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Scala token diff");
        assert!(!report.identical, "Scala: files should differ");

        // Added `val x = 42` -- should produce token-level inserts
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "Scala: expected at least 3 token-level Insert changes for 'val x = 42', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Scala token diff");
        assert_all_expression_kind(&report, "Scala token diff");
    }

    /// Test 15: PHP with changed echo value.
    #[test]
    fn php_token_diff() {
        let a = write_temp(
            "<?php\nfunction greet() {\n    echo \"hello\";\n}\n",
            ".php",
        );
        let b = write_temp(
            "<?php\nfunction greet() {\n    echo \"goodbye\";\n}\n",
            ".php",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "PHP token diff");
        assert!(!report.identical, "PHP: files should differ");

        // The string changed from "hello" to "goodbye" -- should produce changes
        assert!(
            !report.changes.is_empty(),
            "PHP: should have at least one change for string replacement. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "PHP token diff");
        assert_all_expression_kind(&report, "PHP token diff");
    }

    /// Test 16: Lua with added line.
    #[test]
    fn lua_token_diff() {
        let a = write_temp("function greet()\n    print(\"hello\")\nend\n", ".lua");
        let b = write_temp(
            "function greet()\n    local x = 42\n    print(\"hello\")\nend\n",
            ".lua",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Lua token diff");
        assert!(!report.identical, "Lua: files should differ");

        // Added `local x = 42` -- should produce token-level inserts
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "Lua: expected at least 3 token-level Insert changes for 'local x = 42', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Lua token diff");
        assert_all_expression_kind(&report, "Lua token diff");
    }

    /// Test 17: Luau with added line.
    #[test]
    fn luau_token_diff() {
        let a = write_temp(
            "local function greet()\n    print(\"hello\")\nend\n",
            ".luau",
        );
        let b = write_temp(
            "local function greet()\n    local x: number = 42\n    print(\"hello\")\nend\n",
            ".luau",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Luau token diff");
        assert!(!report.identical, "Luau: files should differ");

        // Added `local x: number = 42` -- should produce token-level inserts
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            inserts >= 3,
            "Luau: expected at least 3 token-level Insert changes for 'local x: number = 42', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Luau token diff");
        assert_all_expression_kind(&report, "Luau token diff");
    }

    /// Test 18: Elixir with changed value.
    #[test]
    fn elixir_token_diff() {
        let a = write_temp(
            "defmodule Greeter do\n  def greet do\n    IO.puts(\"hello\")\n  end\nend\n",
            ".ex",
        );
        let b = write_temp(
            "defmodule Greeter do\n  def greet do\n    IO.puts(\"goodbye\")\n  end\nend\n",
            ".ex",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Elixir token diff");
        assert!(!report.identical, "Elixir: files should differ");

        // The string changed from "hello" to "goodbye" -- should produce changes
        assert!(
            !report.changes.is_empty(),
            "Elixir: should have at least one change for string replacement. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Elixir token diff");
        assert_all_expression_kind(&report, "Elixir token diff");
    }

    /// Test 19: OCaml with changed value.
    #[test]
    fn ocaml_token_diff() {
        let a = write_temp("let greet () =\n  print_endline \"hello\"\n", ".ml");
        let b = write_temp("let greet () =\n  print_endline \"goodbye\"\n", ".ml");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "OCaml token diff");
        assert!(!report.identical, "OCaml: files should differ");

        // The string changed from "hello" to "goodbye" -- should produce changes
        assert!(
            !report.changes.is_empty(),
            "OCaml: should have at least one change for string replacement. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "OCaml token diff");
        assert_all_expression_kind(&report, "OCaml token diff");
    }

    /// Test 20: Go with changed return value.
    #[test]
    fn go_token_diff() {
        let a = write_temp(
            "package main\n\nfunc add(x int, y int) int {\n\treturn x + y\n}\n",
            ".go",
        );
        let b = write_temp(
            "package main\n\nfunc add(x int, y int) int {\n\treturn x * y\n}\n",
            ".go",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "Go token diff");
        assert!(!report.identical, "Go: files should differ");

        // The change is "+" -> "*" (one operator token changed)
        // This should produce 1 Delete("+") and 1 Insert("*")
        let deletes = count_changes(&report, ChangeType::Delete);
        let inserts = count_changes(&report, ChangeType::Insert);
        assert!(
            deletes >= 1,
            "Go: expected at least 1 Delete for old operator '+', got {}. Changes: {:?}",
            deletes,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert!(
            inserts >= 1,
            "Go: expected at least 1 Insert for new operator '*', got {}. Changes: {:?}",
            inserts,
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // Should be a small number of changes for a single operator swap
        assert!(
            report.changes.len() <= 4,
            "Go: expected at most 4 changes for a single operator swap, got {}. Changes: {:?}",
            report.changes.len(),
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        assert_flat_changes(&report, "Go token diff");
        assert_all_expression_kind(&report, "Go token diff");
    }
}

// =============================================================================
// Edge Cases
// =============================================================================

mod edge_cases {
    use super::*;

    /// Test 11: Empty file A, file B has content.
    /// All tokens in B should be Insert changes.
    #[test]
    fn empty_file_vs_nonempty() {
        let a = write_temp("", ".py");
        let b = write_temp("def foo(x):\n    return x + 1\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "empty vs nonempty");
        assert!(
            !report.identical,
            "Empty vs nonempty should not be identical"
        );

        // Every token in file B should be an Insert
        let inserts = count_changes(&report, ChangeType::Insert);
        let deletes = count_changes(&report, ChangeType::Delete);
        assert!(
            inserts > 0,
            "Empty file A means all tokens in B should be inserts, got 0. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            deletes, 0,
            "Empty file A should produce 0 deletes, got {}",
            deletes
        );

        // All changes should be Insert
        assert!(
            report
                .changes
                .iter()
                .all(|c| c.change_type == ChangeType::Insert),
            "All changes should be Insert when file A is empty. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // Token count should match the number of tokens in the Python function
        // "def", "foo", "(", "x", ")", ":", "return", "x", "+", "1" = ~10 tokens
        assert!(
            inserts >= 5,
            "Expected at least 5 token inserts for a small function, got {}",
            inserts,
        );

        assert_flat_changes(&report, "empty vs nonempty");
        assert_all_expression_kind(&report, "empty vs nonempty");
    }

    /// Test 12: Large file token diff should not crash (graph limit handling).
    /// Generate 500+ line files with a small difference.
    #[test]
    fn large_file_token_diff() {
        // Generate a 500-line Python file with many functions
        let mut content_a = String::new();
        let mut content_b = String::new();
        for i in 0..100 {
            content_a.push_str(&format!(
                "def func_{}(x):\n    y = x + {}\n    z = y * 2\n    return z\n\n",
                i, i
            ));
            if i == 50 {
                // Change one function in file B
                content_b.push_str(&format!(
                    "def func_{}(x):\n    y = x + {}\n    z = y * 999\n    return z\n\n",
                    i, i
                ));
            } else {
                content_b.push_str(&format!(
                    "def func_{}(x):\n    y = x + {}\n    z = y * 2\n    return z\n\n",
                    i, i
                ));
            }
        }

        let a = write_temp(&content_a, ".py");
        let b = write_temp(&content_b, ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "large file");
        // Should not panic or crash -- the graph limit should handle it
        // The diff should detect the change in func_50
        assert!(
            !report.identical,
            "Large file: should detect the difference"
        );

        // Even if we hit graph limit fallback, we should get some result
        assert!(
            !report.changes.is_empty(),
            "Large file: should have at least one change"
        );

        // Critical L1 check: changes should be token-level, not function-level.
        // For a single "2" -> "999" change, we expect a small number of token
        // changes (delete "2", insert "999"), not a single function-level Update.
        // If graph limit was hit, we allow a fallback change, but the granularity
        // must still be Token.
        let has_function_named_change = report
            .changes
            .iter()
            .any(|c| c.name.as_deref().is_some_and(|n| n.starts_with("func_")));
        assert!(
            !has_function_named_change,
            "Large file: L1 changes should be named after tokens, not functions. \
             Found function-named changes, indicating L4 fallthrough. Changes: {:?}",
            report
                .changes
                .iter()
                .take(10)
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
    }

    /// Test 13: Comment text changed should produce an Update change with
    /// old_text and new_text populated (difftastic's ReplacedComment).
    #[test]
    fn comment_replacement() {
        let a = write_temp(
            "# This is the old comment\ndef foo(x):\n    return x\n",
            ".py",
        );
        let b = write_temp(
            "# This is the new comment\ndef foo(x):\n    return x\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "comment replacement");
        assert!(
            !report.identical,
            "Comment changed, should not be identical"
        );

        // Difftastic treats comment replacement as ReplacedComment, which maps
        // to ChangeType::Update with old_text and new_text.
        assert_has_change(&report, ChangeType::Update, "comment replacement");

        // Find the Update change and verify it has old_text/new_text
        let update = report
            .changes
            .iter()
            .find(|c| c.change_type == ChangeType::Update)
            .expect("Should have an Update change for comment replacement");

        assert!(
            update.old_text.is_some(),
            "Comment Update should have old_text populated"
        );
        assert!(
            update.new_text.is_some(),
            "Comment Update should have new_text populated"
        );

        let old_text = update.old_text.as_deref().unwrap();
        let new_text = update.new_text.as_deref().unwrap();
        assert!(
            old_text.contains("old"),
            "old_text should contain 'old', got: {}",
            old_text
        );
        assert!(
            new_text.contains("new"),
            "new_text should contain 'new', got: {}",
            new_text
        );

        // Should have similarity score (Levenshtein percentage)
        assert!(
            update.similarity.is_some(),
            "Comment Update should have similarity score"
        );

        assert_flat_changes(&report, "comment replacement");
        assert_all_expression_kind(&report, "comment replacement");
    }

    /// Test 14: String literal content changed should produce an Update change
    /// (difftastic's ReplacedString).
    #[test]
    fn string_literal_change() {
        let a = write_temp("def greet():\n    return \"hello world\"\n", ".py");
        let b = write_temp("def greet():\n    return \"goodbye world\"\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "string literal change");
        assert!(!report.identical, "String changed, should not be identical");

        // Difftastic treats string replacement as ReplacedString, which maps
        // to ChangeType::Update with old_text and new_text.
        assert_has_change(&report, ChangeType::Update, "string literal change");

        let update = report
            .changes
            .iter()
            .find(|c| c.change_type == ChangeType::Update)
            .expect("Should have an Update change for string replacement");

        assert!(
            update.old_text.is_some(),
            "String Update should have old_text populated"
        );
        assert!(
            update.new_text.is_some(),
            "String Update should have new_text populated"
        );

        let old_text = update.old_text.as_deref().unwrap();
        let new_text = update.new_text.as_deref().unwrap();
        assert!(
            old_text.contains("hello"),
            "old_text should contain 'hello', got: {}",
            old_text
        );
        assert!(
            new_text.contains("goodbye"),
            "new_text should contain 'goodbye', got: {}",
            new_text
        );

        // Should have similarity score
        assert!(
            update.similarity.is_some(),
            "String Update should have similarity score"
        );
        let sim = update.similarity.unwrap();
        assert!(
            sim > 0.0 && sim < 1.0,
            "Similarity should be between 0 and 1 (exclusive), got {}",
            sim
        );

        assert_flat_changes(&report, "string literal change");
        assert_all_expression_kind(&report, "string literal change");
    }

    /// Test 15: Verify that the report granularity field is set to Token
    /// AND that the changes are actually token-level (not function-level).
    #[test]
    fn granularity_is_token() {
        let a = write_temp("def foo():\n    pass\n", ".py");
        let b = write_temp("def foo():\n    return 1\n", ".py");
        let report = run_l1_diff(&a, &b);

        // The critical assertion: granularity must be Token, not Function
        assert_eq!(
            report.granularity,
            DiffGranularity::Token,
            "L1 diff must set granularity to Token, not {:?}. \
             This is the primary indicator that L1 is implemented.",
            report.granularity,
        );

        // Also verify the summary exists and counts are correct
        let summary = report.summary.as_ref().expect("Report should have summary");
        let total_from_summary = summary.inserts + summary.deletes + summary.updates;
        let total_from_changes = report.changes.len() as u32;

        assert_eq!(
            total_from_summary, total_from_changes,
            "Summary counts ({}) should match changes count ({})",
            total_from_summary, total_from_changes
        );

        // Critical: L1 must produce token-level changes, not function-level.
        // "pass" -> "return 1" should produce at least 2 token changes
        // (delete "pass", insert "return", insert "1") not 1 function Update.
        assert!(
            report.changes.len() >= 2,
            "L1 should produce multiple token-level changes for 'pass' -> 'return 1', \
             got {} changes. If this is 1 Update for the whole function, L1 is \
             falling through to L4. Changes: {:?}",
            report.changes.len(),
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // No change should be named after the function (that would be L4 behavior)
        let has_function_name = report
            .changes
            .iter()
            .any(|c| c.name.as_deref() == Some("foo"));
        assert!(
            !has_function_name,
            "L1 changes should be named after tokens, not functions. \
             Found a change named 'foo' which indicates L4 behavior."
        );
    }
}

// =============================================================================
// Structural Invariants
// =============================================================================

mod invariants {
    use super::*;

    /// Every Insert change should have new_location but not necessarily old_location.
    /// At L1, there should be multiple Insert changes (one per token), not a single
    /// function-level Insert.
    #[test]
    fn insert_changes_have_new_location() {
        let a = write_temp("def foo():\n    pass\n", ".py");
        let b = write_temp("def foo():\n    pass\n\ndef bar():\n    return 1\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "insert locations");

        let inserts = changes_of_type(&report, ChangeType::Insert);
        assert!(
            !inserts.is_empty(),
            "Should have Insert changes for new function tokens"
        );

        // L1 check: we should have multiple token-level inserts for the new function
        // "def", "bar", "(", ")", ":", "return", "1" = ~7 tokens
        // NOT a single function-level Insert for "bar"
        assert!(
            inserts.len() >= 4,
            "L1 should produce multiple token-level Insert changes (one per token), \
             got {}. If this is 1, L1 is falling through to L4. Inserts: {:?}",
            inserts.len(),
            inserts
                .iter()
                .map(|c| format!("{:?}", c.name))
                .collect::<Vec<_>>()
        );

        for insert in &inserts {
            assert!(
                insert.new_location.is_some(),
                "Insert change {:?} should have new_location",
                insert.name
            );
        }
    }

    /// Every Delete change should have old_location but not necessarily new_location.
    /// At L1, there should be multiple Delete changes (one per token), not a single
    /// function-level Delete.
    #[test]
    fn delete_changes_have_old_location() {
        let a = write_temp("def foo():\n    pass\n\ndef bar():\n    return 1\n", ".py");
        let b = write_temp("def foo():\n    pass\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "delete locations");

        let deletes = changes_of_type(&report, ChangeType::Delete);
        assert!(
            !deletes.is_empty(),
            "Should have Delete changes for removed function tokens"
        );

        // L1 check: we should have multiple token-level deletes for the removed function
        // "def", "bar", "(", ")", ":", "return", "1" = ~7 tokens
        // NOT a single function-level Delete for "bar"
        assert!(
            deletes.len() >= 4,
            "L1 should produce multiple token-level Delete changes (one per token), \
             got {}. If this is 1, L1 is falling through to L4. Deletes: {:?}",
            deletes.len(),
            deletes
                .iter()
                .map(|c| format!("{:?}", c.name))
                .collect::<Vec<_>>()
        );

        for delete in &deletes {
            assert!(
                delete.old_location.is_some(),
                "Delete change {:?} should have old_location",
                delete.name
            );
        }
    }

    /// Update changes (ReplacedComment/ReplacedString) should have both
    /// old_location and new_location, plus old_text and new_text.
    #[test]
    fn update_changes_have_both_locations_and_text() {
        let a = write_temp(
            "# old comment\ndef foo():\n    return \"old string\"\n",
            ".py",
        );
        let b = write_temp(
            "# new comment\ndef foo():\n    return \"new string\"\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "update locations");

        let updates = changes_of_type(&report, ChangeType::Update);
        assert!(
            !updates.is_empty(),
            "Should have Update changes for comment and string replacement"
        );

        for update in &updates {
            assert!(
                update.old_location.is_some(),
                "Update change {:?} should have old_location",
                update.name
            );
            assert!(
                update.new_location.is_some(),
                "Update change {:?} should have new_location",
                update.name
            );
            assert!(
                update.old_text.is_some(),
                "Update change {:?} should have old_text",
                update.name
            );
            assert!(
                update.new_text.is_some(),
                "Update change {:?} should have new_text",
                update.name
            );
        }
    }

    /// Token names should be populated (the token content, truncated to 80 chars).
    /// Names should be individual token content (like "1", "return", "+"), NOT
    /// function names (like "foo", "compute").
    #[test]
    fn changes_have_names() {
        let a = write_temp("def foo():\n    return 1\n", ".py");
        let b = write_temp("def foo():\n    return 2\n", ".py");
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "change names");
        assert!(!report.changes.is_empty(), "Should have changes");

        for change in &report.changes {
            assert!(
                change.name.is_some(),
                "L1 change should have a name (token content), got None for {:?}",
                change.change_type
            );
            let name = change.name.as_deref().unwrap();
            assert!(!name.is_empty(), "L1 change name should not be empty");
            assert!(
                name.len() <= 83, // 80 chars + "..." truncation
                "L1 change name should be at most 80 chars (+ '...'), got {} chars: {}",
                name.len(),
                name
            );
        }

        // Critical L1 check: the changed token is "1" -> "2", so we should see
        // changes named "1" and "2" (or similar token content), NOT "foo".
        let has_function_name = report
            .changes
            .iter()
            .any(|c| c.name.as_deref() == Some("foo"));
        assert!(
            !has_function_name,
            "L1 change names should be token content (e.g., '1', '2'), not function \
             names (e.g., 'foo'). Found 'foo', indicating L4 behavior. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );
    }

    /// Verify no changes have Move, Rename, Extract, or Inline types at L1.
    /// L1 only produces Insert, Delete, Update (and optionally Format).
    #[test]
    fn no_high_level_change_types_at_l1() {
        let a = write_temp(
            "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n",
            ".py",
        );
        let b = write_temp(
            "def baz(y):\n    return y * 2\n\ndef foo(x):\n    return x + 1\n",
            ".py",
        );
        let report = run_l1_diff(&a, &b);

        assert_token_granularity(&report, "no high-level types");

        // L1 should never produce Move, Rename, Extract, or Inline
        // (those are L4+ concepts)
        for change in &report.changes {
            assert!(
                matches!(
                    change.change_type,
                    ChangeType::Insert
                        | ChangeType::Delete
                        | ChangeType::Update
                        | ChangeType::Format
                ),
                "L1 should only have Insert/Delete/Update/Format, got {:?} for {:?}",
                change.change_type,
                change.name,
            );
        }
    }

    /// Symmetric test: diffing A vs B and then B vs A should produce the
    /// inverse changes (inserts become deletes and vice versa).
    /// At L1, the token counts should be symmetric.
    #[test]
    fn symmetric_insert_delete() {
        let a = write_temp("def foo():\n    return 1\n", ".py");
        let b = write_temp(
            "def foo():\n    return 1\n\ndef bar():\n    return 2\n",
            ".py",
        );

        let report_ab = run_l1_diff(&a, &b);
        let report_ba = run_l1_diff(&b, &a);

        assert_token_granularity(&report_ab, "symmetric A->B");
        assert_token_granularity(&report_ba, "symmetric B->A");

        let ab_inserts = count_changes(&report_ab, ChangeType::Insert);
        let ab_deletes = count_changes(&report_ab, ChangeType::Delete);
        let ba_inserts = count_changes(&report_ba, ChangeType::Insert);
        let ba_deletes = count_changes(&report_ba, ChangeType::Delete);

        // L1 check: should have multiple token-level inserts for bar function
        assert!(
            ab_inserts >= 4,
            "A->B should have multiple token-level inserts for new function, got {}. \
             If this is 1, L1 is falling through to L4. Changes: {:?}",
            ab_inserts,
            report_ab
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}", c.change_type, c.name))
                .collect::<Vec<_>>()
        );

        // Inserts in A->B should be deletes in B->A, and vice versa
        assert_eq!(
            ab_inserts, ba_deletes,
            "A->B inserts ({}) should equal B->A deletes ({})",
            ab_inserts, ba_deletes
        );
        assert_eq!(
            ab_deletes, ba_inserts,
            "A->B deletes ({}) should equal B->A inserts ({})",
            ab_deletes, ba_inserts
        );
    }
}
