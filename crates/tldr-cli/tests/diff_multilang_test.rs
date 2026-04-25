//! Multi-Language Diff Tests (L4 Function + L5 Class)
//!
//! Validates that `tldr diff` works across all 18 supported languages at:
//! - L4 (function-level): all 18 languages
//! - L5 (class-level): 13 languages that have class/struct constructs
//!   (excludes C, Lua, Luau, Elixir, OCaml)
//!
//! Each test uses minimal fixtures - just enough to verify parsing and
//! change detection work for that language.

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

/// Run L4 (function-level) diff and return the report.
fn run_l4_diff(file_a: &NamedTempFile, file_b: &NamedTempFile) -> DiffReport {
    let args = DiffArgs {
        file_a: file_a.path().to_path_buf(),
        file_b: file_b.path().to_path_buf(),
        granularity: DiffGranularity::Function,
        semantic_only: false,
        output: None,
    };
    args.run_to_report().expect("L4 diff should succeed")
}

/// Run L5 (class-level) diff and return the report.
fn run_l5_diff(file_a: &NamedTempFile, file_b: &NamedTempFile) -> DiffReport {
    let args = DiffArgs {
        file_a: file_a.path().to_path_buf(),
        file_b: file_b.path().to_path_buf(),
        granularity: DiffGranularity::Class,
        semantic_only: false,
        output: None,
    };
    args.run_to_report().expect("L5 diff should succeed")
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

/// Assert that a report has at least one change of the given type with the given node kind.
#[allow(dead_code)]
fn assert_has_change_with_kind(
    report: &DiffReport,
    change_type: ChangeType,
    node_kind: NodeKind,
    context: &str,
) {
    let found = report
        .changes
        .iter()
        .any(|c| c.change_type == change_type && c.node_kind == node_kind);
    assert!(
        found,
        "{}: expected {:?} change with {:?} kind, but found none. Changes: {:?}",
        context,
        change_type,
        node_kind,
        report
            .changes
            .iter()
            .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
            .collect::<Vec<_>>()
    );
}

/// Find class-level Update changes that have children with Insert changes.
fn has_class_update_with_child_insert(report: &DiffReport) -> bool {
    report.changes.iter().any(|c| {
        c.change_type == ChangeType::Update
            && c.node_kind == NodeKind::Class
            && c.children
                .as_ref()
                .map(|children| {
                    children
                        .iter()
                        .any(|ch| ch.change_type == ChangeType::Insert)
                })
                .unwrap_or(false)
    })
}

// =============================================================================
// Python (.py)
// =============================================================================

mod python {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "def foo(x):\n    return x + 1\n\ndef bar(x):\n    return x * 2\n",
            ".py",
        );
        let b = write_temp(
            "def foo(x):\n    return x + 99\n\ndef bar(x):\n    return x * 2\n\ndef baz(x):\n    return x - 1\n",
            ".py",
        );
        let report = run_l4_diff(&a, &b);
        assert!(
            !report.identical,
            "Python L4: files should not be identical"
        );
        assert_has_change(&report, ChangeType::Update, "Python L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Python L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp("class Foo:\n    def bar(self):\n        return 1\n", ".py");
        let b = write_temp(
            "class Foo:\n    def bar(self):\n        return 1\n    def baz(self):\n        return 2\n",
            ".py",
        );
        let report = run_l5_diff(&a, &b);
        assert!(
            !report.identical,
            "Python L5: files should not be identical"
        );
        assert!(
            has_class_update_with_child_insert(&report),
            "Python L5: should detect class update with method insert as child. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// TypeScript (.ts)
// =============================================================================

mod typescript {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "function foo(x: number): number {\n    return x + 1;\n}\n\nfunction bar(x: number): number {\n    return x * 2;\n}\n",
            ".ts",
        );
        let b = write_temp(
            "function foo(x: number): number {\n    return x + 99;\n}\n\nfunction bar(x: number): number {\n    return x * 2;\n}\n\nfunction baz(x: number): number {\n    return x - 1;\n}\n",
            ".ts",
        );
        let report = run_l4_diff(&a, &b);
        assert!(
            !report.identical,
            "TypeScript L4: files should not be identical"
        );
        assert_has_change(&report, ChangeType::Update, "TypeScript L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "TypeScript L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp("class Foo {\n    bar(): number { return 1; }\n}\n", ".ts");
        let b = write_temp(
            "class Foo {\n    bar(): number { return 1; }\n    baz(): number { return 2; }\n}\n",
            ".ts",
        );
        let report = run_l5_diff(&a, &b);
        assert!(
            !report.identical,
            "TypeScript L5: files should not be identical"
        );
        assert!(
            has_class_update_with_child_insert(&report),
            "TypeScript L5: should detect class update with method insert. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// JavaScript (.js)
// =============================================================================

mod javascript {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "function foo(x) {\n    return x + 1;\n}\n\nfunction bar(x) {\n    return x * 2;\n}\n",
            ".js",
        );
        let b = write_temp(
            "function foo(x) {\n    return x + 99;\n}\n\nfunction bar(x) {\n    return x * 2;\n}\n\nfunction baz(x) {\n    return x - 1;\n}\n",
            ".js",
        );
        let report = run_l4_diff(&a, &b);
        assert!(
            !report.identical,
            "JavaScript L4: files should not be identical"
        );
        assert_has_change(&report, ChangeType::Update, "JavaScript L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "JavaScript L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp("class Foo {\n    bar() { return 1; }\n}\n", ".js");
        let b = write_temp(
            "class Foo {\n    bar() { return 1; }\n    baz() { return 2; }\n}\n",
            ".js",
        );
        let report = run_l5_diff(&a, &b);
        assert!(
            !report.identical,
            "JavaScript L5: files should not be identical"
        );
        assert!(
            has_class_update_with_child_insert(&report),
            "JavaScript L5: should detect class update with method insert. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Go (.go)
// =============================================================================

mod go {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "package main\n\nfunc foo(x int) int {\n\treturn x + 1\n}\n\nfunc bar(x int) int {\n\treturn x * 2\n}\n",
            ".go",
        );
        let b = write_temp(
            "package main\n\nfunc foo(x int) int {\n\treturn x + 99\n}\n\nfunc bar(x int) int {\n\treturn x * 2\n}\n\nfunc baz(x int) int {\n\treturn x - 1\n}\n",
            ".go",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Go L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Go L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Go L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        // Go uses structs + methods (not nested in class body)
        // L5 should detect structural changes at the type level
        let a = write_temp(
            "package main\n\ntype Foo struct {\n\tx int\n}\n\nfunc (f *Foo) Bar() int {\n\treturn f.x\n}\n",
            ".go",
        );
        let b = write_temp(
            "package main\n\ntype Foo struct {\n\tx int\n\ty int\n}\n\nfunc (f *Foo) Bar() int {\n\treturn f.x\n}\n\nfunc (f *Foo) Baz() int {\n\treturn f.y\n}\n",
            ".go",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Go L5: files should not be identical");
        // Go methods aren't nested in the struct, so we may see updates differently.
        // At minimum, the diff should detect changes (struct updated, method added).
        assert!(
            !report.changes.is_empty(),
            "Go L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Rust (.rs)
// =============================================================================

mod rust_lang {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\n\nfn bar(x: i32) -> i32 {\n    x * 2\n}\n",
            ".rs",
        );
        let b = write_temp(
            "fn foo(x: i32) -> i32 {\n    x + 99\n}\n\nfn bar(x: i32) -> i32 {\n    x * 2\n}\n\nfn baz(x: i32) -> i32 {\n    x - 1\n}\n",
            ".rs",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Rust L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Rust L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Rust L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "struct Foo {\n    x: i32,\n}\n\nimpl Foo {\n    fn bar(&self) -> i32 {\n        self.x\n    }\n}\n",
            ".rs",
        );
        let b = write_temp(
            "struct Foo {\n    x: i32,\n}\n\nimpl Foo {\n    fn bar(&self) -> i32 {\n        self.x\n    }\n    fn baz(&self) -> i32 {\n        self.x + 1\n    }\n}\n",
            ".rs",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Rust L5: files should not be identical");
        // Rust impl blocks should be detected as class-like containers
        assert!(
            !report.changes.is_empty(),
            "Rust L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Java (.java)
// =============================================================================

mod java {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "public class Main {\n    public static int foo(int x) {\n        return x + 1;\n    }\n    public static int bar(int x) {\n        return x * 2;\n    }\n}\n",
            ".java",
        );
        let b = write_temp(
            "public class Main {\n    public static int foo(int x) {\n        return x + 99;\n    }\n    public static int bar(int x) {\n        return x * 2;\n    }\n    public static int baz(int x) {\n        return x - 1;\n    }\n}\n",
            ".java",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Java L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Java L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Java L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "public class Foo {\n    public int bar() {\n        return 1;\n    }\n}\n",
            ".java",
        );
        let b = write_temp(
            "public class Foo {\n    public int bar() {\n        return 1;\n    }\n    public int baz() {\n        return 2;\n    }\n}\n",
            ".java",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Java L5: files should not be identical");
        assert!(
            has_class_update_with_child_insert(&report),
            "Java L5: should detect class update with method insert. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// C (.c) - L4 only (no classes)
// =============================================================================

mod c_lang {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "int foo(int x) {\n    return x + 1;\n}\n\nint bar(int x) {\n    return x * 2;\n}\n",
            ".c",
        );
        let b = write_temp(
            "int foo(int x) {\n    return x + 99;\n}\n\nint bar(int x) {\n    return x * 2;\n}\n\nint baz(int x) {\n    return x - 1;\n}\n",
            ".c",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "C L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "C L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "C L4 (baz added)");
    }
}

// =============================================================================
// C++ (.cpp)
// =============================================================================

mod cpp {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "int foo(int x) {\n    return x + 1;\n}\n\nint bar(int x) {\n    return x * 2;\n}\n",
            ".cpp",
        );
        let b = write_temp(
            "int foo(int x) {\n    return x + 99;\n}\n\nint bar(int x) {\n    return x * 2;\n}\n\nint baz(int x) {\n    return x - 1;\n}\n",
            ".cpp",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "C++ L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "C++ L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "C++ L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "class Foo {\npublic:\n    int bar() { return 1; }\n};\n",
            ".cpp",
        );
        let b = write_temp(
            "class Foo {\npublic:\n    int bar() { return 1; }\n    int baz() { return 2; }\n};\n",
            ".cpp",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "C++ L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "C++ L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Ruby (.rb)
// =============================================================================

mod ruby {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "def foo(x)\n  x + 1\nend\n\ndef bar(x)\n  x * 2\nend\n",
            ".rb",
        );
        let b = write_temp(
            "def foo(x)\n  x + 99\nend\n\ndef bar(x)\n  x * 2\nend\n\ndef baz(x)\n  x - 1\nend\n",
            ".rb",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Ruby L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Ruby L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Ruby L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp("class Foo\n  def bar\n    1\n  end\nend\n", ".rb");
        let b = write_temp(
            "class Foo\n  def bar\n    1\n  end\n  def baz\n    2\n  end\nend\n",
            ".rb",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Ruby L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "Ruby L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Kotlin (.kt)
// =============================================================================

mod kotlin {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "fun foo(x: Int): Int {\n    return x + 1\n}\n\nfun bar(x: Int): Int {\n    return x * 2\n}\n",
            ".kt",
        );
        let b = write_temp(
            "fun foo(x: Int): Int {\n    return x + 99\n}\n\nfun bar(x: Int): Int {\n    return x * 2\n}\n\nfun baz(x: Int): Int {\n    return x - 1\n}\n",
            ".kt",
        );
        let report = run_l4_diff(&a, &b);
        assert!(
            !report.identical,
            "Kotlin L4: files should not be identical"
        );
        assert_has_change(&report, ChangeType::Update, "Kotlin L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Kotlin L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "class Foo {\n    fun bar(): Int {\n        return 1\n    }\n}\n",
            ".kt",
        );
        let b = write_temp(
            "class Foo {\n    fun bar(): Int {\n        return 1\n    }\n    fun baz(): Int {\n        return 2\n    }\n}\n",
            ".kt",
        );
        let report = run_l5_diff(&a, &b);
        assert!(
            !report.identical,
            "Kotlin L5: files should not be identical"
        );
        assert!(
            !report.changes.is_empty(),
            "Kotlin L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Swift (.swift)
// =============================================================================

mod swift {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "func foo(x: Int) -> Int {\n    return x + 1\n}\n\nfunc bar(x: Int) -> Int {\n    return x * 2\n}\n",
            ".swift",
        );
        let b = write_temp(
            "func foo(x: Int) -> Int {\n    return x + 99\n}\n\nfunc bar(x: Int) -> Int {\n    return x * 2\n}\n\nfunc baz(x: Int) -> Int {\n    return x - 1\n}\n",
            ".swift",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Swift L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Swift L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Swift L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "class Foo {\n    func bar() -> Int {\n        return 1\n    }\n}\n",
            ".swift",
        );
        let b = write_temp(
            "class Foo {\n    func bar() -> Int {\n        return 1\n    }\n    func baz() -> Int {\n        return 2\n    }\n}\n",
            ".swift",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Swift L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "Swift L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// C# (.cs)
// =============================================================================

mod csharp {
    use super::*;

    #[test]
    fn l4_function_diff() {
        // C# requires methods inside a class, so we nest them
        let a = write_temp(
            "class Program {\n    static int Foo(int x) {\n        return x + 1;\n    }\n    static int Bar(int x) {\n        return x * 2;\n    }\n}\n",
            ".cs",
        );
        let b = write_temp(
            "class Program {\n    static int Foo(int x) {\n        return x + 99;\n    }\n    static int Bar(int x) {\n        return x * 2;\n    }\n    static int Baz(int x) {\n        return x - 1;\n    }\n}\n",
            ".cs",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "C# L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "C# L4 (Foo changed)");
        assert_has_change(&report, ChangeType::Insert, "C# L4 (Baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "class Foo {\n    int Bar() {\n        return 1;\n    }\n}\n",
            ".cs",
        );
        let b = write_temp(
            "class Foo {\n    int Bar() {\n        return 1;\n    }\n    int Baz() {\n        return 2;\n    }\n}\n",
            ".cs",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "C# L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "C# L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Scala (.scala)
// =============================================================================

mod scala {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "object Main {\n  def foo(x: Int): Int = {\n    x + 1\n  }\n  def bar(x: Int): Int = {\n    x * 2\n  }\n}\n",
            ".scala",
        );
        let b = write_temp(
            "object Main {\n  def foo(x: Int): Int = {\n    x + 99\n  }\n  def bar(x: Int): Int = {\n    x * 2\n  }\n  def baz(x: Int): Int = {\n    x - 1\n  }\n}\n",
            ".scala",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Scala L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Scala L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Scala L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp("class Foo {\n  def bar: Int = 1\n}\n", ".scala");
        let b = write_temp(
            "class Foo {\n  def bar: Int = 1\n  def baz: Int = 2\n}\n",
            ".scala",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "Scala L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "Scala L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// PHP (.php)
// =============================================================================

mod php {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "<?php\nfunction foo($x) {\n    return $x + 1;\n}\n\nfunction bar($x) {\n    return $x * 2;\n}\n",
            ".php",
        );
        let b = write_temp(
            "<?php\nfunction foo($x) {\n    return $x + 99;\n}\n\nfunction bar($x) {\n    return $x * 2;\n}\n\nfunction baz($x) {\n    return $x - 1;\n}\n",
            ".php",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "PHP L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "PHP L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "PHP L4 (baz added)");
    }

    #[test]
    fn l5_class_diff() {
        let a = write_temp(
            "<?php\nclass Foo {\n    function bar() {\n        return 1;\n    }\n}\n",
            ".php",
        );
        let b = write_temp(
            "<?php\nclass Foo {\n    function bar() {\n        return 1;\n    }\n    function baz() {\n        return 2;\n    }\n}\n",
            ".php",
        );
        let report = run_l5_diff(&a, &b);
        assert!(!report.identical, "PHP L5: files should not be identical");
        assert!(
            !report.changes.is_empty(),
            "PHP L5: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// Lua (.lua) - L4 only (no classes)
// =============================================================================

mod lua {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "function foo(x)\n    return x + 1\nend\n\nfunction bar(x)\n    return x * 2\nend\n",
            ".lua",
        );
        let b = write_temp(
            "function foo(x)\n    return x + 99\nend\n\nfunction bar(x)\n    return x * 2\nend\n\nfunction baz(x)\n    return x - 1\nend\n",
            ".lua",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Lua L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Lua L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Lua L4 (baz added)");
    }
}

// =============================================================================
// Luau (.luau) - L4 only (no classes)
// =============================================================================

mod luau {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "function foo(x: number): number\n    return x + 1\nend\n\nfunction bar(x: number): number\n    return x * 2\nend\n",
            ".luau",
        );
        let b = write_temp(
            "function foo(x: number): number\n    return x + 99\nend\n\nfunction bar(x: number): number\n    return x * 2\nend\n\nfunction baz(x: number): number\n    return x - 1\nend\n",
            ".luau",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "Luau L4: files should not be identical");
        assert_has_change(&report, ChangeType::Update, "Luau L4 (foo changed)");
        assert_has_change(&report, ChangeType::Insert, "Luau L4 (baz added)");
    }
}

// =============================================================================
// Elixir (.ex) - L4 only (modules + functions, no traditional classes)
// =============================================================================

mod elixir {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp(
            "defmodule Foo do\n  def bar(x) do\n    x + 1\n  end\n\n  def baz(x) do\n    x * 2\n  end\nend\n",
            ".ex",
        );
        let b = write_temp(
            "defmodule Foo do\n  def bar(x) do\n    x + 99\n  end\n\n  def baz(x) do\n    x * 2\n  end\n\n  def qux(x) do\n    x - 1\n  end\nend\n",
            ".ex",
        );
        let report = run_l4_diff(&a, &b);
        assert!(
            !report.identical,
            "Elixir L4: files should not be identical"
        );
        // Elixir functions are inside defmodule; the diff should detect changes
        assert!(
            !report.changes.is_empty(),
            "Elixir L4: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}

// =============================================================================
// OCaml (.ml) - L4 only (no traditional classes)
// =============================================================================

mod ocaml {
    use super::*;

    #[test]
    fn l4_function_diff() {
        let a = write_temp("let foo x = x + 1\n\nlet bar x = x * 2\n", ".ml");
        let b = write_temp(
            "let foo x = x + 99\n\nlet bar x = x * 2\n\nlet baz x = x - 1\n",
            ".ml",
        );
        let report = run_l4_diff(&a, &b);
        assert!(!report.identical, "OCaml L4: files should not be identical");
        // OCaml uses let bindings for functions
        assert!(
            !report.changes.is_empty(),
            "OCaml L4: should detect at least some changes. Changes: {:?}",
            report
                .changes
                .iter()
                .map(|c| format!("{:?}:{:?}:{:?}", c.change_type, c.node_kind, c.name))
                .collect::<Vec<_>>()
        );
    }
}
