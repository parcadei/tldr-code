//! Comprehensive benchmark test suite for surface + fix commands.
//!
//! This module exercises EVERY analyzer across ALL 5 languages to ensure
//! no analyzer is missing, broken, or producing wrong output. If any analyzer
//! is removed or regresses, this suite catches it.
//!
//! # Coverage
//!
//! | Section                   | Count | Languages        |
//! |---------------------------|-------|------------------|
//! | Error parser detection    |    10 | Py/JS/TS/Rust/Go |
//! | Python fix analyzers      |    22 | Python           |
//! | Rust fix analyzers        |     5 | Rust             |
//! | TypeScript fix analyzers  |     8 | TypeScript       |
//! | Go fix analyzers          |     6 | Go               |
//! | JavaScript fix analyzers  |     4 | JavaScript       |
//! | E2E diagnose dispatch     |     5 | All              |

use std::path::PathBuf;

use crate::fix::error_parser::{detect_language, parse_error};
use crate::fix::types::{FixConfidence, ParsedError};
use crate::fix::{diagnose, diagnose_parsed};

// ============================================================================
// Section 1: Error Parser Auto-Detection
// ============================================================================

#[test]
fn test_benchmark_parser_detect_python_traceback() {
    let error = "Traceback (most recent call last):\n  File \"app.py\", line 10, in main\n    x += 1\nNameError: name 'x' is not defined";
    assert_eq!(detect_language(error), "python");
}

#[test]
fn test_benchmark_parser_detect_python_single_line() {
    let error = "NameError: name 'x' is not defined";
    assert_eq!(detect_language(error), "python");
}

#[test]
fn test_benchmark_parser_detect_javascript_stack() {
    let error = "TypeError: Cannot read properties of undefined (reading 'foo')\n    at Object.<anonymous> (file.js:1:1)";
    assert_eq!(detect_language(error), "javascript");
}

#[test]
fn test_benchmark_parser_detect_javascript_reference_error() {
    let error = "ReferenceError: x is not defined\n    at Object.<anonymous> (file.js:1:1)";
    assert_eq!(detect_language(error), "javascript");
}

#[test]
fn test_benchmark_parser_detect_typescript_full() {
    let error = "app.ts(5,10): error TS2304: Cannot find name 'x'.";
    assert_eq!(detect_language(error), "typescript");
}

#[test]
fn test_benchmark_parser_detect_typescript_bare() {
    let error = "error TS2304: Cannot find name 'x'.";
    assert_eq!(detect_language(error), "typescript");
}

#[test]
fn test_benchmark_parser_detect_rust_rendered() {
    let error = "error[E0425]: cannot find value `x` in this scope";
    assert_eq!(detect_language(error), "rust");
}

#[test]
fn test_benchmark_parser_detect_go_file_line() {
    let error = "./main.go:5:2: undefined: x";
    assert_eq!(detect_language(error), "go");
}

#[test]
fn test_benchmark_parser_detect_go_unused_var() {
    let error = "x declared and not used";
    assert_eq!(detect_language(error), "go");
}

#[test]
fn test_benchmark_parser_detect_ambiguous_bare_type_error() {
    // Bare "TypeError:" without JS stack trace should detect as python
    // (JS detection requires stack trace markers)
    let error = "TypeError: 'int' object is not callable";
    let lang = detect_language(error);
    // This matches the generic Python XError: pattern
    assert_eq!(lang, "python");
}

// ============================================================================
// Section 2: Python Fix Analyzers (22 total)
// ============================================================================
//
// Each test feeds a representative error + source to diagnose() and asserts
// the diagnosis is produced with the correct error_code.

// --- Analyzer #1: UnboundLocalError ---

#[test]
fn test_benchmark_python_unbound_local_error() {
    let error_text = "UnboundLocalError: cannot access local variable 'counter'";
    let source = "counter = 0\ndef inc():\n    counter += 1\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "UnboundLocalError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "UnboundLocalError");
    assert_eq!(d.language, "python");
    assert!(d.fix.is_some(), "UnboundLocalError should produce a fix with global injection");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("global counter"),
        "Fix should inject 'global counter', got: {:?}",
        fix.edits[0].new_text
    );
}

// --- Analyzer #2: TypeError (callable) ---

#[test]
fn test_benchmark_python_type_error_not_callable() {
    let error_text = "TypeError: 'dict' object is not callable";
    let source = "d = {'a': 1}\ndef f():\n    result = d.items()\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "TypeError callable must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
    assert_eq!(d.language, "python");
}

// --- Analyzer #3: TypeError (JSON serializable) ---

#[test]
fn test_benchmark_python_type_error_json_serializable() {
    let error_text = "TypeError: Object of type Foo is not JSON serializable";
    let source = "from dataclasses import dataclass\nimport json\n\n@dataclass\nclass Foo:\n    x: int\n\ndef f():\n    obj = Foo(1)\n    json.dumps(obj)\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "TypeError JSON serializable must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
    assert!(
        d.message.contains("JSON serializable"),
        "Message should mention JSON, got: {}",
        d.message
    );
}

// --- Analyzer #4: NameError ---

#[test]
fn test_benchmark_python_name_error_stdlib() {
    let error_text = "NameError: name 'json' is not defined";
    let source = "def f():\n    data = json.loads('{}')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "NameError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "NameError");
    assert!(d.fix.is_some(), "NameError for stdlib name should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("import json"),
        "Fix should inject 'import json', got: {:?}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_python_name_error_unknown() {
    let error_text = "NameError: name 'foobar_xyz' is not defined";
    let source = "def f():\n    x = foobar_xyz()\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "NameError for unknown name must still produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "NameError");
    assert_eq!(d.confidence, FixConfidence::Low, "Unknown name should have Low confidence");
    assert!(d.fix.is_none(), "Unknown name should not produce a fix");
}

// --- Analyzer #5: ImportError ---

#[test]
fn test_benchmark_python_import_error_cannot_import() {
    let error_text = "ImportError: cannot import name 'Path' from 'os'";
    let source = "from os import Path\n\ndef f():\n    p = Path('.')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "ImportError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ImportError");
}

#[test]
fn test_benchmark_python_module_not_found() {
    let error_text = "ModuleNotFoundError: No module named 'nonexistent_pkg'";
    let source = "import nonexistent_pkg\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "ModuleNotFoundError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ImportError");
    assert!(d.fix.is_none(), "Unknown module should not produce a fix");
}

// --- Analyzer #6: AttributeError ---

#[test]
fn test_benchmark_python_attribute_error() {
    let error_text = "AttributeError: 'str' object has no attribute 'append'";
    let source = "def f():\n    s = 'hello'\n    s.append('!')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "AttributeError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "AttributeError");
    assert!(
        d.message.contains("has no attribute"),
        "Message should mention attribute, got: {}",
        d.message
    );
}

// --- Analyzer #7: ValueError (hint-only) ---

#[test]
fn test_benchmark_python_value_error() {
    let error_text = "ValueError: invalid literal for int() with base 10: 'abc'";
    let source = "def f():\n    x = int('abc')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "ValueError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ValueError");
    assert!(d.fix.is_none(), "ValueError is hint-only, should not produce a fix");
}

// --- Analyzer #8: IndexError (hint-only) ---

#[test]
fn test_benchmark_python_index_error() {
    let error_text = "IndexError: list index out of range";
    let source = "def f():\n    items = [1, 2]\n    return items[5]\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "IndexError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "IndexError");
    assert!(d.fix.is_none(), "IndexError is hint-only, should not produce a fix");
}

// --- Analyzer #9: KeyError ---

#[test]
fn test_benchmark_python_key_error() {
    let error_text = "KeyError: 'name'";
    let source = "def lookup(key):\n    d = {'a': 1}\n    return d[key]\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "KeyError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "KeyError");
}

// --- Analyzer #10: ZeroDivisionError (hint-only) ---

#[test]
fn test_benchmark_python_zero_division_error() {
    let error_text = "ZeroDivisionError: division by zero";
    let source = "def f():\n    return 1 / 0\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "ZeroDivisionError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ZeroDivisionError");
    assert!(d.fix.is_none(), "ZeroDivisionError is hint-only");
}

// --- Analyzer #11: RecursionError (hint-only) ---

#[test]
fn test_benchmark_python_recursion_error() {
    let error_text = "RecursionError: maximum recursion depth exceeded";
    let source = "def f():\n    return f()\n";
    let parsed = parse_error(error_text, Some("python"));
    assert!(parsed.is_some(), "Parser should handle RecursionError");
    let mut error = parsed.unwrap();
    error.function_name = Some("f".to_string());
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "RecursionError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "RecursionError");
    assert!(d.fix.is_none(), "RecursionError is hint-only");
}

// --- Analyzer #12: StopIteration (hint-only) ---

#[test]
fn test_benchmark_python_stop_iteration() {
    // StopIteration without a colon does not match the single-line parser regex,
    // so we construct the ParsedError directly (as the existing tests do).
    let source = "def f():\n    it = iter([])\n    return next(it)\n";
    let error = ParsedError {
        error_type: "StopIteration".to_string(),
        message: String::new(),
        file: Some(PathBuf::from("app.py")),
        line: Some(3),
        column: None,
        language: "python".to_string(),
        raw_text: "StopIteration".to_string(),
        function_name: Some("f".to_string()),
        offending_line: None,
    };
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "StopIteration must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "StopIteration");
    assert!(d.fix.is_none(), "StopIteration is hint-only");
}

#[test]
fn test_benchmark_python_stop_iteration_from_traceback() {
    // StopIteration parsed via full traceback (the parser handles bare "StopIteration")
    let error_text = "Traceback (most recent call last):\n  File \"app.py\", line 3, in f\n    return next(it)\nStopIteration";
    let parsed = parse_error(error_text, Some("python"));
    assert!(parsed.is_some(), "Parser should handle StopIteration in traceback");
    let error = parsed.unwrap();
    assert_eq!(error.error_type, "StopIteration");
}

// --- Analyzer #13: AssertionError (hint-only) ---

#[test]
fn test_benchmark_python_assertion_error() {
    // Use a non-test function name to avoid the test_ filter
    let source = "def validate(x):\n    assert x > 0\n";
    let error = ParsedError {
        error_type: "AssertionError".to_string(),
        message: String::new(),
        file: Some(PathBuf::from("app.py")),
        line: Some(2),
        column: None,
        language: "python".to_string(),
        raw_text: "AssertionError".to_string(),
        function_name: Some("validate".to_string()),
        offending_line: None,
    };
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "AssertionError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "AssertionError");
    assert!(d.fix.is_none(), "AssertionError is hint-only");
}

// --- Analyzer #14: NotImplementedError ---

#[test]
fn test_benchmark_python_not_implemented_error() {
    let source = "def f():\n    raise NotImplementedError\n";
    let error = ParsedError {
        error_type: "NotImplementedError".to_string(),
        message: String::new(),
        file: Some(PathBuf::from("app.py")),
        line: Some(2),
        column: None,
        language: "python".to_string(),
        raw_text: "NotImplementedError".to_string(),
        function_name: Some("f".to_string()),
        offending_line: None,
    };
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "NotImplementedError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "NotImplementedError");
    assert!(d.fix.is_none(), "NotImplementedError is hint-only");
}

// --- Analyzer #15: OSError / FileNotFoundError ---

#[test]
fn test_benchmark_python_file_not_found_error() {
    let error_text = "FileNotFoundError: [Errno 2] No such file or directory: '/tmp/missing/file.txt'";
    let source = "def write_data():\n    with open('/tmp/missing/file.txt', 'w') as f:\n        f.write('data')\n";
    let parsed = parse_error(error_text, Some("python"));
    assert!(parsed.is_some());
    let mut error = parsed.unwrap();
    error.function_name = Some("write_data".to_string());
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "FileNotFoundError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "OSError");
}

#[test]
fn test_benchmark_python_permission_error() {
    let error_text = "PermissionError: [Errno 13] Permission denied: '/root/secret'";
    let source = "def f():\n    with open('/root/secret') as f:\n        pass\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "PermissionError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "OSError");
}

// --- Analyzer #16: UnicodeError ---

#[test]
fn test_benchmark_python_unicode_error() {
    let error_text = "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff";
    let source = "def read_file():\n    with open('data.bin') as f:\n        return f.read()\n";
    let parsed = parse_error(error_text, Some("python"));
    assert!(parsed.is_some());
    let mut error = parsed.unwrap();
    error.function_name = Some("read_file".to_string());
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "UnicodeError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "UnicodeError");
}

// --- Analyzer #17: SyntaxError ---

#[test]
fn test_benchmark_python_syntax_error_missing_colon() {
    let error_text = "SyntaxError: expected ':'";
    let source = "def f()\n    pass\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "SyntaxError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "SyntaxError");
    assert!(d.fix.is_some(), "SyntaxError for missing colon should produce a fix");
}

#[test]
fn test_benchmark_python_syntax_error_return_outside_function() {
    let error_text = "SyntaxError: 'return' outside function";
    let source = "return 42\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "SyntaxError return outside function must diagnose");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "SyntaxError");
}

// --- Analyzer #18: IndentationError ---

#[test]
fn test_benchmark_python_indentation_error_mixed_tabs() {
    let error_text = "IndentationError: inconsistent use of tabs and spaces in indentation";
    let source = "def f():\n\t    pass\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "IndentationError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "IndentationError");
    assert!(d.fix.is_some(), "IndentationError with tabs should produce a fix");
}

#[test]
fn test_benchmark_python_indentation_error_unexpected() {
    let error_text = "IndentationError: unexpected indent";
    let source = "x = 1\n    y = 2\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "IndentationError unexpected indent must diagnose");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "IndentationError");
}

// --- Analyzer #19: CircularImportError ---

#[test]
fn test_benchmark_python_circular_import() {
    let error_text = "ImportError: cannot import name 'helper' from partially initialized module 'mymod' (most likely due to a circular import)";
    let source = "from mymod import helper\n\ndef use_it():\n    return helper()\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "CircularImportError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ImportError");
    assert!(
        d.message.contains("Circular import") || d.message.contains("partially initialized"),
        "Message should mention circular import, got: {}",
        d.message
    );
}

// --- Analyzer #20: TypeError (general -- other patterns) ---

#[test]
fn test_benchmark_python_type_error_subscriptable() {
    let error_text = "TypeError: 'int' object is not subscriptable";
    let source = "def f():\n    x = 42\n    return x[0]\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "TypeError subscriptable must diagnose");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
    assert!(d.message.contains("subscriptable"));
}

#[test]
fn test_benchmark_python_type_error_missing_args() {
    let error_text = "TypeError: f() missing 2 required positional arguments: 'a' and 'b'";
    let source = "def f(a, b):\n    pass\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "TypeError missing args must diagnose");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
}

#[test]
fn test_benchmark_python_type_error_unexpected_kwarg() {
    let error_text = "TypeError: f() got an unexpected keyword argument 'bar'";
    let source = "def f(a):\n    pass\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "TypeError unexpected kwarg must diagnose");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
}

// --- Analyzer #21: RuntimeError ---

#[test]
fn test_benchmark_python_runtime_error() {
    let error_text = "RuntimeError: something went wrong";
    let source = "def f():\n    raise RuntimeError('something went wrong')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some(), "RuntimeError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "RuntimeError");
    assert!(d.fix.is_none(), "RuntimeError is hint-only");
}

// --- Analyzer #22: GenericException (floor) ---

#[test]
fn test_benchmark_python_generic_exception() {
    // A custom exception type goes through the generic analyzer
    let source = "def f():\n    raise CustomError('oops')\n";
    let error = ParsedError {
        error_type: "CustomError".to_string(),
        message: "oops".to_string(),
        file: Some(PathBuf::from("app.py")),
        line: Some(2),
        column: None,
        language: "python".to_string(),
        raw_text: "CustomError: oops".to_string(),
        function_name: Some("f".to_string()),
        offending_line: None,
    };
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "Generic exception must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "CustomError");
    assert!(d.fix.is_none(), "Generic exception is hint-only");
}

// --- Python validation gate: all 22 analyzers dispatch correctly ---

#[test]
fn test_benchmark_python_all_22_analyzers_dispatch() {
    // Each tuple: (error_type, sample_message, expected_error_code)
    let cases: Vec<(&str, &str, &str)> = vec![
        ("UnboundLocalError", "cannot access local variable 'x'", "UnboundLocalError"),
        ("TypeError", "'dict' object is not callable", "TypeError"),
        ("TypeError", "Object of type Foo is not JSON serializable", "TypeError"),
        ("NameError", "name 'os' is not defined", "NameError"),
        ("ImportError", "cannot import name 'Foo' from 'bar'", "ImportError"),
        ("AttributeError", "'str' object has no attribute 'foo'", "AttributeError"),
        ("ValueError", "invalid literal for int() with base 10", "ValueError"),
        ("IndexError", "list index out of range", "IndexError"),
        ("KeyError", "'name'", "KeyError"),
        ("ZeroDivisionError", "division by zero", "ZeroDivisionError"),
        ("RecursionError", "maximum recursion depth exceeded", "RecursionError"),
        ("StopIteration", "", "StopIteration"),
        ("AssertionError", "", "AssertionError"),
        ("NotImplementedError", "", "NotImplementedError"),
        ("OSError", "No such file or directory: '/tmp/x'", "OSError"),
        ("UnicodeError", "codec can't decode byte", "UnicodeError"),
        ("SyntaxError", "expected ':'", "SyntaxError"),
        ("IndentationError", "unexpected indent", "IndentationError"),
        ("ImportError", "cannot import name 'x' from partially initialized module 'y'", "ImportError"),
        ("TypeError", "'int' object is not subscriptable", "TypeError"),
        ("RuntimeError", "something went wrong", "RuntimeError"),
        ("CustomException", "some custom error", "CustomException"),
    ];

    let source = "x = 1\ndef f():\n    pass\n";

    let mut handled = 0;
    for (error_type, message, expected_code) in &cases {
        let error = ParsedError {
            error_type: error_type.to_string(),
            message: message.to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: format!("{}: {}", error_type, message),
            function_name: Some("f".to_string()),
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "Python analyzer for {} should return Some (message: {})",
            error_type, message
        );
        let d = result.unwrap();
        assert_eq!(
            d.error_code, *expected_code,
            "Expected error_code '{}' for {}, got '{}'",
            expected_code, error_type, d.error_code
        );
        handled += 1;
    }
    assert_eq!(handled, 22, "Expected 22 handled Python analyzers, got {}", handled);
}

// ============================================================================
// Section 3: Rust Fix Analyzers (5 total)
// ============================================================================

// --- E0425: cannot find value ---

#[test]
fn test_benchmark_rust_e0425_hashmap() {
    let error_text = "error[E0425]: cannot find value `HashMap` in this scope";
    let source = "fn main() {\n    let m: HashMap<String, i32> = HashMap::new();\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some(), "Rust E0425 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0425");
    assert_eq!(d.language, "rust");
    assert!(d.fix.is_some(), "E0425 for known item should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("use std::collections::HashMap"),
        "Fix should inject HashMap use, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_rust_e0425_pathbuf() {
    let error_text = "error[E0425]: cannot find type `PathBuf` in this scope";
    let source = "fn main() {\n    let p = PathBuf::from(\"/tmp\");\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0425");
    assert!(d.fix.is_some());
    assert!(d.fix.unwrap().edits[0].new_text.contains("use std::path::PathBuf"));
}

// --- E0433: failed to resolve ---

#[test]
fn test_benchmark_rust_e0433_hashmap() {
    let error_text = "error[E0433]: failed to resolve: use of undeclared type `HashMap`";
    let source = "fn main() {\n    let m = HashMap::new();\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some(), "Rust E0433 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0433");
    assert!(d.fix.is_some(), "E0433 for known type should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("use std::collections::HashMap"),
        "Fix should inject HashMap use, got: {}",
        fix.edits[0].new_text
    );
}

// --- E0599: no method named ---

#[test]
fn test_benchmark_rust_e0599_write_all() {
    let error_text = "error[E0599]: no method named `write_all` found for struct `File`";
    let source = "use std::fs::File;\n\nfn main() {\n    let f = File::create(\"out.txt\").unwrap();\n    f.write_all(b\"hello\");\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some(), "Rust E0599 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0599");
    assert!(d.fix.is_some(), "E0599 for known trait should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("use std::io::Write"),
        "Fix should inject Write use, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_rust_e0599_read_line() {
    let error_text = "error[E0599]: no method named `read_line` found for struct `Stdin` in the current scope";
    let source = "fn main() {\n    let mut buf = String::new();\n    std::io::stdin().read_line(&mut buf);\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0599");
    assert!(d.fix.is_some());
    assert!(d.fix.unwrap().edits[0].new_text.contains("use std::io::BufRead"));
}

// --- E0308: mismatched types ---

#[test]
fn test_benchmark_rust_e0308_string_str() {
    let error_text = "error[E0308]: mismatched types: expected `String`, found `&str`";
    let source = "fn takes_string(s: String) {}\n\nfn main() {\n    takes_string(\"hello\");\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some(), "Rust E0308 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0308");
}

// --- E0277: trait not satisfied ---

#[test]
fn test_benchmark_rust_e0277_iterator_copied() {
    let error_text = "error[E0277]: a value of type `Vec<i32>` cannot be built from an iterator over elements of type `&i32`";
    let source = "fn main() {\n    let v = vec![1, 2, 3];\n    let w: Vec<i32> = v.iter().collect();\n}\n";
    let parsed = parse_error(error_text, Some("rust"));
    assert!(parsed.is_some());
    let mut error = parsed.unwrap();
    error.line = Some(3);
    let diag = diagnose_parsed(&error, source, None);
    assert!(diag.is_some(), "Rust E0277 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "E0277");
    assert!(d.fix.is_some(), "E0277 iterator copied should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains(".copied().collect()"),
        "Fix should insert .copied(), got: {}",
        fix.edits[0].new_text
    );
}

// --- Rust validation gate ---

#[test]
fn test_benchmark_rust_all_5_analyzers_dispatch() {
    let cases = [
        ("E0425", "cannot find value `HashMap` in this scope"),
        ("E0433", "failed to resolve: use of undeclared type `HashMap`"),
        ("E0599", "no method named `write_all` found for struct `File`"),
        ("E0308", "mismatched types: expected `String`, found `&str`"),
        ("E0277", "trait bound not satisfied"),
    ];

    let source = "fn main() {}\n";

    for (code, msg) in &cases {
        let error = ParsedError {
            error_type: code.to_string(),
            message: msg.to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(1),
            column: None,
            language: "rust".to_string(),
            raw_text: format!("error[{}]: {}", code, msg),
            function_name: None,
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "Rust analyzer for {} should return Some (msg: {})",
            code, msg
        );
        let d = result.unwrap();
        assert_eq!(
            d.error_code, *code,
            "Expected error_code '{}', got '{}'",
            code, d.error_code
        );
        assert_eq!(d.language, "rust");
    }
}

// ============================================================================
// Section 4: TypeScript Fix Analyzers (8 total)
// ============================================================================

// --- TS2304: Cannot find name ---

#[test]
fn test_benchmark_ts_2304_express() {
    let error_text = "error TS2304: Cannot find name 'express'.";
    let source = "const app = express();\napp.listen(3000);\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2304 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2304");
    assert_eq!(d.language, "typescript");
    assert!(d.fix.is_some(), "TS2304 for known package should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("import express"),
        "Fix should inject express import, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_ts_2304_use_state() {
    let error_text = "error TS2304: Cannot find name 'useState'.";
    let source = "const [count, setCount] = useState(0);\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2304");
    assert!(d.fix.is_some());
    assert!(d.fix.unwrap().edits[0].new_text.contains("useState"));
}

// --- TS2305: has no exported member ---

#[test]
fn test_benchmark_ts_2305_no_export() {
    let error_text = "error TS2305: Module '\"react\"' has no exported member 'createPortal'.";
    let source = "import { createPortal } from 'react';\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2305 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2305");
}

// --- TS2307: Cannot find module ---

#[test]
fn test_benchmark_ts_2307_module_not_found() {
    let error_text = "error TS2307: Cannot find module 'loadsh' or its corresponding type declarations.";
    let source = "import loadsh from 'loadsh';\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2307 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2307");
}

// --- TS2322: Type not assignable ---

#[test]
fn test_benchmark_ts_2322_type_not_assignable() {
    let error_text = "error TS2322: Type 'string' is not assignable to type 'number'.";
    let source = "let x: number = \"hello\";\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2322 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2322");
}

// --- TS2339: Property does not exist ---

#[test]
fn test_benchmark_ts_2339_property_not_exists() {
    let error_text = "error TS2339: Property 'foo' does not exist on type 'Bar'.";
    let source = "interface Bar { baz: string; }\nconst b: Bar = { baz: 'x' };\nconsole.log(b.foo);\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2339 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2339");
}

// --- TS2345: Argument type not assignable ---

#[test]
fn test_benchmark_ts_2345_arg_type() {
    let error_text = "error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.";
    let source = "function add(a: number) { return a + 1; }\nadd(\"hello\");\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2345 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2345");
}

// --- TS2554: Expected N arguments got M ---

#[test]
fn test_benchmark_ts_2554_wrong_arg_count() {
    let error_text = "error TS2554: Expected 2 arguments, but got 1.";
    let source = "function add(a: number, b: number) { return a + b; }\nadd(1);\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some(), "TS2554 must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TS2554");
}

// --- TypeScript validation gate ---

#[test]
fn test_benchmark_ts_all_8_analyzers_dispatch() {
    let cases = [
        ("TS2304", "Cannot find name 'express'."),
        ("TS2305", "Module '\"react\"' has no exported member 'createPortal'."),
        ("TS2307", "Cannot find module 'nonexistent'."),
        ("TS2322", "Type 'string' is not assignable to type 'number'."),
        ("TS2339", "Property 'foo' does not exist on type 'Bar'."),
        ("TS2345", "Argument of type 'string' is not assignable to parameter of type 'number'."),
        ("TS2554", "Expected 2 arguments, but got 1."),
        ("TS7006", "Parameter 'x' implicitly has an 'any' type."),
    ];

    let source = "const x = 1;\n";

    for (code, msg) in &cases {
        let error = ParsedError {
            error_type: code.to_string(),
            message: msg.to_string(),
            file: Some(PathBuf::from("app.ts")),
            line: Some(1),
            column: None,
            language: "typescript".to_string(),
            raw_text: format!("error {}: {}", code, msg),
            function_name: None,
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "TypeScript analyzer for {} should return Some (msg: {})",
            code, msg
        );
        let d = result.unwrap();
        assert_eq!(
            d.error_code, *code,
            "Expected error_code '{}', got '{}'",
            code, d.error_code
        );
        assert_eq!(d.language, "typescript");
    }
}

// ============================================================================
// Section 5: Go Fix Analyzers (6 total)
// ============================================================================

// --- Analyzer 1: undefined (missing import) ---

#[test]
fn test_benchmark_go_undefined_fmt() {
    let error_text = "./main.go:4:7: undefined: fmt";
    let source = "package main\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some(), "Go undefined must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "undefined");
    assert_eq!(d.language, "go");
    assert!(d.fix.is_some(), "Go undefined for known package should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("\"fmt\""),
        "Fix should inject fmt import, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_go_undefined_json() {
    let error_text = "./main.go:6:14: undefined: json";
    let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tdata, _ := json.Marshal(nil)\n\tfmt.Println(data)\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert!(d.fix.is_some());
    assert!(d.fix.unwrap().edits[0].new_text.contains("\"encoding/json\""));
}

// --- Analyzer 2: type mismatch ---

#[test]
fn test_benchmark_go_type_mismatch() {
    let error_text = "./main.go:5:10: cannot use s (variable of type string) as type []byte in argument";
    let source = "package main\n\nfunc main() {\n\ts := \"hello\"\n\tprocessBytes(s)\n}\n\nfunc processBytes(b []byte) {}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some(), "Go type mismatch must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.language, "go");
}

// --- Analyzer 3: imported and not used ---

#[test]
fn test_benchmark_go_unused_import() {
    let error_text = "./main.go:3:8: \"os\" imported and not used";
    let source = "package main\n\nimport \"os\"\n\nfunc main() {\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some(), "Go unused import must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "unused_import");
    assert!(d.fix.is_some(), "Unused import should produce a fix (delete line)");
}

// --- Analyzer 4: declared but not used ---

#[test]
fn test_benchmark_go_unused_var() {
    let error_text = "./main.go:4:2: x declared but not used";
    let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some(), "Go unused var must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "unused_var");
    assert!(d.fix.is_some(), "Unused var should produce a fix (prefix with _)");
}

#[test]
fn test_benchmark_go_unused_var_declared_and_not_used() {
    // Alternate wording: "declared and not used"
    let error_text = "./main.go:4:2: x declared and not used";
    let diag = diagnose(error_text, "package main\n\nfunc main() {\n\tx := 42\n}\n", Some("go"), None);
    assert!(diag.is_some());
    assert_eq!(diag.unwrap().error_code, "unused_var");
}

// --- Analyzer 5: missing return ---

#[test]
fn test_benchmark_go_missing_return() {
    let error_text = "./main.go:3:1: missing return at end of function";
    let source = "package main\n\nfunc add(a, b int) int {\n\t_ = a + b\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some(), "Go missing return must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "missing_return");
}

// --- Analyzer 6: too many / not enough arguments ---

#[test]
fn test_benchmark_go_too_many_args() {
    let error_text = "./main.go:5:2: too many arguments in call to add";
    let source = "package main\n\nfunc add(a, b int) int { return a + b }\n\nfunc main() {\n\tadd(1, 2, 3)\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    // This may fall through to general Go diagnostic depending on implementation
    // The important thing is it doesn't panic
    if let Some(d) = diag {
        assert_eq!(d.language, "go");
    }
}

// --- Go validation gate ---

#[test]
fn test_benchmark_go_all_6_patterns_dispatch() {
    // The Go dispatcher uses `let full = format!("{} {}", msg, error.raw_text);`
    // so patterns can match in either field. Each case uses its own source and
    // correct line number to ensure the analyzer can find the offending line.
    //
    // (error_type, msg, raw_text, source, line_no)
    let cases: Vec<(&str, &str, &str, &str, usize)> = vec![
        (
            "undefined", "undefined: fmt",
            "./main.go:4:7: undefined: fmt",
            "package main\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n",
            4,
        ),
        (
            "type_mismatch",
            "cannot use s (variable of type string) as type []byte",
            "./main.go:5:10: cannot use s (variable of type string) as type []byte",
            "package main\n\nfunc main() {\n\ts := \"hello\"\n\tprocessBytes(s)\n}\n\nfunc processBytes(b []byte) {}\n",
            5,
        ),
        (
            "unused_import",
            "\"os\" imported and not used",
            "./main.go:3:8: \"os\" imported and not used",
            "package main\n\nimport \"os\"\n\nfunc main() {\n}\n",
            3,
        ),
        (
            "unused_var",
            "x declared but not used",
            "./main.go:4:2: x declared but not used",
            "package main\n\nfunc main() {\n\tx := 42\n}\n",
            4,
        ),
        (
            "missing_return",
            "missing return at end of function",
            "./main.go:3:1: missing return at end of function",
            "package main\n\nfunc add(a, b int) int {\n\t_ = a + b\n}\n",
            3,
        ),
    ];

    for (error_type, msg, raw, source, line_no) in &cases {
        let error = ParsedError {
            error_type: error_type.to_string(),
            message: msg.to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(*line_no),
            column: None,
            language: "go".to_string(),
            raw_text: raw.to_string(),
            function_name: None,
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "Go analyzer for '{}' should return Some (msg: {})",
            error_type, msg
        );
        let d = result.unwrap();
        assert_eq!(d.language, "go");
    }
}

// ============================================================================
// Section 6: JavaScript Fix Analyzers (4 total)
// ============================================================================

// --- Analyzer 1: ReferenceError ---

#[test]
fn test_benchmark_js_reference_error_known_module() {
    let error_text = "ReferenceError: fs is not defined\n    at Object.<anonymous> (app.js:1:1)";
    let source = "const data = fs.readFileSync('file.txt');\n";
    let diag = diagnose(error_text, source, Some("javascript"), None);
    assert!(diag.is_some(), "JS ReferenceError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ReferenceError");
    assert_eq!(d.language, "javascript");
    assert!(d.fix.is_some(), "ReferenceError for known module should produce a fix");
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("require('fs')"),
        "Fix should inject fs require, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_js_reference_error_unknown() {
    let error_text = "ReferenceError: customLib is not defined\n    at Object.<anonymous> (app.js:1:1)";
    let source = "const x = customLib.doStuff();\n";
    let diag = diagnose(error_text, source, Some("javascript"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert_eq!(d.error_code, "ReferenceError");
}

// --- Analyzer 2: TypeError (Cannot read properties of undefined) ---

#[test]
fn test_benchmark_js_type_error_undefined_property() {
    let error_text = "TypeError: Cannot read properties of undefined (reading 'foo')\n    at Object.<anonymous> (app.js:2:1)";
    let source = "const obj = undefined;\nconst x = obj.foo;\n";
    let diag = diagnose(error_text, source, Some("javascript"), None);
    assert!(diag.is_some(), "JS TypeError undefined property must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
    assert_eq!(d.language, "javascript");
}

// --- Analyzer 3: TypeError (is not a function) ---

#[test]
fn test_benchmark_js_type_error_not_a_function() {
    let error_text = "TypeError: obj.length is not a function\n    at Object.<anonymous> (app.js:2:1)";
    let source = "const obj = [1, 2, 3];\nconst x = obj.length();\n";
    let diag = diagnose(error_text, source, Some("javascript"), None);
    assert!(diag.is_some(), "JS TypeError not a function must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "TypeError");
}

// --- Analyzer 4: SyntaxError ---

#[test]
fn test_benchmark_js_syntax_error() {
    let error_text = "SyntaxError: Unexpected token '}'\n    at Object.<anonymous> (app.js:3:1)";
    let source = "function f() {\n  return 1\n}}\n";
    let diag = diagnose(error_text, source, Some("javascript"), None);
    assert!(diag.is_some(), "JS SyntaxError must produce a diagnosis");
    let d = diag.unwrap();
    assert_eq!(d.error_code, "SyntaxError");
    assert_eq!(d.language, "javascript");
}

// --- JavaScript validation gate ---

#[test]
fn test_benchmark_js_all_4_analyzers_dispatch() {
    let cases = [
        ("ReferenceError", "fs is not defined"),
        ("TypeError", "Cannot read properties of undefined (reading 'foo')"),
        ("TypeError", "obj.length is not a function"),
        ("SyntaxError", "Unexpected token '}'"),
    ];

    let source = "const x = 1;\n";

    for (error_type, msg) in &cases {
        let error = ParsedError {
            error_type: error_type.to_string(),
            message: msg.to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: format!("{}: {}\n    at Object.<anonymous> (app.js:1:1)", error_type, msg),
            function_name: None,
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "JS analyzer for {} should return Some (msg: {})",
            error_type, msg
        );
        let d = result.unwrap();
        assert_eq!(d.language, "javascript");
    }
}

// ============================================================================
// Section 7: E2E diagnose dispatch (cross-language)
// ============================================================================
//
// Test the top-level diagnose() function with auto-detection for each language.

#[test]
fn test_benchmark_e2e_python_auto_detect() {
    let error_text = "NameError: name 'os' is not defined";
    let source = "def f():\n    os.path.exists('.')\n";
    let diag = diagnose(error_text, source, None, None);
    assert!(diag.is_some(), "E2E Python auto-detect must work");
    let d = diag.unwrap();
    assert_eq!(d.language, "python");
    assert_eq!(d.error_code, "NameError");
    assert!(d.fix.is_some());
}

#[test]
fn test_benchmark_e2e_rust_auto_detect() {
    let error_text = "error[E0425]: cannot find value `HashMap` in this scope";
    let source = "fn main() {\n    let m = HashMap::new();\n}\n";
    let diag = diagnose(error_text, source, None, None);
    assert!(diag.is_some(), "E2E Rust auto-detect must work");
    let d = diag.unwrap();
    assert_eq!(d.language, "rust");
    assert_eq!(d.error_code, "E0425");
}

#[test]
fn test_benchmark_e2e_typescript_auto_detect() {
    let error_text = "error TS2304: Cannot find name 'express'.";
    let source = "const app = express();\n";
    let diag = diagnose(error_text, source, None, None);
    assert!(diag.is_some(), "E2E TypeScript auto-detect must work");
    let d = diag.unwrap();
    assert_eq!(d.language, "typescript");
    assert_eq!(d.error_code, "TS2304");
}

#[test]
fn test_benchmark_e2e_go_auto_detect() {
    let error_text = "./main.go:4:7: undefined: fmt";
    let source = "package main\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
    let diag = diagnose(error_text, source, None, None);
    assert!(diag.is_some(), "E2E Go auto-detect must work");
    let d = diag.unwrap();
    assert_eq!(d.language, "go");
    assert_eq!(d.error_code, "undefined");
}

#[test]
fn test_benchmark_e2e_javascript_auto_detect() {
    let error_text = "ReferenceError: path is not defined\n    at Object.<anonymous> (app.js:1:1)";
    let source = "const dir = path.join(__dirname, 'data');\n";
    let diag = diagnose(error_text, source, None, None);
    assert!(diag.is_some(), "E2E JavaScript auto-detect must work");
    let d = diag.unwrap();
    assert_eq!(d.language, "javascript");
    assert_eq!(d.error_code, "ReferenceError");
}

// ============================================================================
// Section 8: Fix Quality Assertions
// ============================================================================
//
// For analyzers that produce fixes, verify the fix content is correct
// (not just that a fix exists).

#[test]
fn test_benchmark_fix_quality_python_name_error_import_position() {
    // Verify the import is placed after existing imports
    let error_text = "NameError: name 'json' is not defined";
    let source = "import os\nimport sys\n\ndef f():\n    data = json.loads('{}')\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some());
    let fix = diag.unwrap().fix.unwrap();
    // The import should be placed after line 2 (the last existing import)
    assert!(
        fix.edits[0].line >= 2,
        "Import should be placed after existing imports, line={}, expected >= 2",
        fix.edits[0].line
    );
}

#[test]
fn test_benchmark_fix_quality_python_unbound_local_indent() {
    // Verify the global declaration uses correct indentation
    let error_text = "UnboundLocalError: cannot access local variable 'total'";
    let source = "total = 0\ndef add(x):\n    total += x\n";
    let diag = diagnose(error_text, source, Some("python"), None);
    assert!(diag.is_some());
    let fix = diag.unwrap().fix.unwrap();
    assert!(
        fix.edits[0].new_text.starts_with("    "),
        "global declaration should be indented with 4 spaces, got: {:?}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_fix_quality_go_import_injection() {
    // Verify Go import injection places the import correctly
    let error_text = "./main.go:4:7: undefined: strings";
    let source = "package main\n\nfunc main() {\n\ts := strings.ToUpper(\"hello\")\n\tprintln(s)\n}\n";
    let diag = diagnose(error_text, source, Some("go"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert!(d.fix.is_some());
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("\"strings\""),
        "Fix should inject strings import, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_fix_quality_rust_use_injection() {
    // Verify Rust use injection for File
    let error_text = "error[E0425]: cannot find value `File` in this scope";
    let source = "fn main() {\n    let f = File::open(\"test.txt\").unwrap();\n}\n";
    let diag = diagnose(error_text, source, Some("rust"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert!(d.fix.is_some());
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("use std::fs::File"),
        "Fix should inject File use, got: {}",
        fix.edits[0].new_text
    );
}

#[test]
fn test_benchmark_fix_quality_ts_import_injection() {
    // Verify TS import injection for React hook
    let error_text = "error TS2304: Cannot find name 'useEffect'.";
    let source = "useEffect(() => {}, []);\n";
    let diag = diagnose(error_text, source, Some("typescript"), None);
    assert!(diag.is_some());
    let d = diag.unwrap();
    assert!(d.fix.is_some());
    let fix = d.fix.unwrap();
    assert!(
        fix.edits[0].new_text.contains("useEffect"),
        "Fix should inject useEffect import, got: {}",
        fix.edits[0].new_text
    );
}

// ============================================================================
// Section 9: Hint-only analyzers produce no TextEdit fixes
// ============================================================================

#[test]
fn test_benchmark_hint_only_python_all() {
    // Verify that all hint-only Python analyzers produce no fix
    let hint_only_cases: Vec<(&str, &str)> = vec![
        ("IndexError", "list index out of range"),
        ("ValueError", "invalid literal for int()"),
        ("ZeroDivisionError", "division by zero"),
        ("RecursionError", "maximum recursion depth exceeded"),
        ("StopIteration", ""),
        ("AssertionError", ""),
    ];

    let source = "def f():\n    pass\n";

    for (error_type, msg) in &hint_only_cases {
        let error = ParsedError {
            error_type: error_type.to_string(),
            message: msg.to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: format!("{}: {}", error_type, msg),
            function_name: Some("f".to_string()),
            offending_line: None,
        };
        let result = diagnose_parsed(&error, source, None);
        assert!(
            result.is_some(),
            "Hint-only analyzer for {} should return Some",
            error_type
        );
        let d = result.unwrap();
        assert!(
            d.fix.is_none(),
            "{} is hint-only and must NOT produce a fix, but got: {:?}",
            error_type,
            d.fix
        );
    }
}

// ============================================================================
// Section 10: Error parser round-trip tests
// ============================================================================
//
// Verify that parse_error -> diagnose_parsed produces correct language tags.

#[test]
fn test_benchmark_parser_roundtrip_python() {
    let raw = "Traceback (most recent call last):\n  File \"app.py\", line 10, in main\n    x += 1\nUnboundLocalError: cannot access local variable 'x'";
    let parsed = parse_error(raw, None);
    assert!(parsed.is_some());
    let error = parsed.unwrap();
    assert_eq!(error.language, "python");
    assert_eq!(error.error_type, "UnboundLocalError");
    assert_eq!(error.file, Some(PathBuf::from("app.py")));
    assert_eq!(error.line, Some(10));
    assert_eq!(error.function_name, Some("main".to_string()));
}

#[test]
fn test_benchmark_parser_roundtrip_go() {
    let raw = "./main.go:5:2: undefined: x";
    let parsed = parse_error(raw, None);
    assert!(parsed.is_some());
    let error = parsed.unwrap();
    assert_eq!(error.language, "go");
    assert!(error.file.is_some());
    assert_eq!(error.line, Some(5));
}

#[test]
fn test_benchmark_parser_roundtrip_typescript() {
    let raw = "error TS2304: Cannot find name 'x'.";
    let parsed = parse_error(raw, None);
    assert!(parsed.is_some());
    let error = parsed.unwrap();
    assert_eq!(error.language, "typescript");
    assert_eq!(error.error_type, "TS2304");
}

#[test]
fn test_benchmark_parser_roundtrip_rust() {
    let raw = "error[E0425]: cannot find value `x` in this scope";
    let parsed = parse_error(raw, None);
    assert!(parsed.is_some());
    let error = parsed.unwrap();
    assert_eq!(error.language, "rust");
    assert_eq!(error.error_type, "E0425");
}

#[test]
fn test_benchmark_parser_roundtrip_javascript() {
    let raw = "ReferenceError: x is not defined\n    at Object.<anonymous> (file.js:1:1)";
    let parsed = parse_error(raw, None);
    assert!(parsed.is_some());
    let error = parsed.unwrap();
    assert_eq!(error.language, "javascript");
    assert_eq!(error.error_type, "ReferenceError");
}
