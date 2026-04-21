//! Diagnostic and auto-fix system for `tldr fix`.
//!
//! This module provides:
//! - Error parsing from compiler/runtime output (`error_parser`)
//! - Python error analysis with 22 analyzers (`python`)
//! - Rust error analysis with 5 analyzers (`rust_lang`)
//! - TypeScript error analysis with 8 analyzers (`typescript`)
//! - Go error analysis with 6 analyzers (`go`)
//! - JavaScript error analysis with 4 analyzers (`javascript`)
//! - Patch application for text edits (`patch`)
//! - Core types for the fix lifecycle (`types`)
//! - Check loop: run command, diagnose, fix, repeat (`check`)
//!
//! # Usage
//!
//! ```rust,ignore
//! use tldr_core::fix::{diagnose, apply_fix};
//!
//! let error_text = "UnboundLocalError: cannot access local variable 'counter'";
//! let source = std::fs::read_to_string("app.py")?;
//! let diagnosis = diagnose(error_text, &source, Some("python"), None)?;
//! if let Some(fix) = &diagnosis.fix {
//!     let patched = apply_fix(&source, fix);
//!     std::fs::write("app.py", patched)?;
//! }
//! ```

pub mod check;
pub mod error_parser;
pub mod go;
pub mod javascript;
pub mod patch;
pub mod python;
pub mod rust_lang;
pub mod typescript;
pub mod types;

#[cfg(test)]
mod benchmark_tests;

pub use check::{run_check_loop, CheckConfig, CheckResult, FixAttempt};
pub use patch::apply_fix;
pub use types::{Diagnosis, EditKind, Fix, FixConfidence, FixLocation, ParsedError, TextEdit};

use crate::ast::parser;

/// Diagnose an error from raw error text and source code.
///
/// This is the main entry point for the fix system. It:
/// 1. Parses the raw error text into a structured `ParsedError`
/// 2. Dispatches to the correct language-specific analyzer
/// 3. Returns a `Diagnosis` with an optional fix
///
/// # Arguments
///
/// * `error_text` - Raw error output (traceback, compiler message, etc.)
/// * `source` - The source code of the file where the error occurred
/// * `lang` - Optional language hint (auto-detected if `None`)
/// * `api_surface` - Optional API surface for enhanced analysis (Phase 1 output)
///
/// # Returns
///
/// `Some(Diagnosis)` if the error was recognized, `None` if parsing failed.
pub fn diagnose(
    error_text: &str,
    source: &str,
    lang: Option<&str>,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Step 1: Parse the error text
    let parsed = error_parser::parse_error(error_text, lang)?;

    // Step 2: Dispatch to language-specific analyzer
    match parsed.language.as_str() {
        "python" => diagnose_python(&parsed, source, _api_surface),
        "rust" => diagnose_rust_lang(&parsed, source, _api_surface),
        "typescript" => diagnose_typescript(&parsed, source, _api_surface),
        "go" => diagnose_go(&parsed, source, _api_surface),
        "javascript" => diagnose_javascript(&parsed, source, _api_surface),
        _ => {
            // Try Python analyzer as fallback (handles generic single-line errors)
            diagnose_python(&parsed, source, _api_surface)
        }
    }
}

/// Diagnose a pre-parsed error against source code.
///
/// Use this when you already have a `ParsedError` (e.g., from a custom parser).
pub fn diagnose_parsed(
    error: &ParsedError,
    source: &str,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    match error.language.as_str() {
        "python" => diagnose_python(error, source, _api_surface),
        "rust" => diagnose_rust_lang(error, source, _api_surface),
        "typescript" => diagnose_typescript(error, source, _api_surface),
        "go" => diagnose_go(error, source, _api_surface),
        "javascript" => diagnose_javascript(error, source, _api_surface),
        _ => diagnose_python(error, source, _api_surface),
    }
}

/// Internal: run the Rust diagnostic pipeline.
fn diagnose_rust_lang(
    error: &ParsedError,
    source: &str,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Parse the source with tree-sitter
    let tree = parser::parse(source, crate::Language::Rust).ok()?;
    rust_lang::diagnose_rust(error, source, &tree, _api_surface)
}

/// Internal: run the TypeScript diagnostic pipeline.
fn diagnose_typescript(
    error: &ParsedError,
    source: &str,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Parse the source with tree-sitter
    let tree = parser::parse(source, crate::Language::TypeScript).ok()?;
    typescript::diagnose_typescript(error, source, &tree, _api_surface)
}

/// Internal: run the Go diagnostic pipeline.
fn diagnose_go(
    error: &ParsedError,
    source: &str,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Parse the source with tree-sitter
    let tree = parser::parse(source, crate::Language::Go).ok()?;
    go::diagnose_go(error, source, &tree, _api_surface)
}

/// Internal: run the JavaScript diagnostic pipeline.
fn diagnose_javascript(
    error: &ParsedError,
    source: &str,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Parse the source with tree-sitter (JavaScript uses the TypeScript grammar)
    let tree = parser::parse(source, crate::Language::JavaScript).ok()?;
    javascript::diagnose_javascript(error, source, &tree, _api_surface)
}

/// Internal: run the Python diagnostic pipeline.
fn diagnose_python(
    error: &ParsedError,
    source: &str,
    api_surface: Option<&()>,
) -> Option<Diagnosis> {
    // Parse the source with tree-sitter
    let tree = parser::parse(source, crate::Language::Python).ok()?;
    python::diagnose_python(error, source, &tree, api_surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnose_python_unbound_local() {
        let error_text = "UnboundLocalError: cannot access local variable 'counter'";
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let diag = diagnose(error_text, source, Some("python"), None);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "UnboundLocalError");
        assert_eq!(d.language, "python");
        // This is the single-line error path (no traceback) -- fix MUST be produced
        assert!(
            d.fix.is_some(),
            "Single-line UnboundLocalError must produce a fix with global injection"
        );
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("global counter"),
            "Fix should inject 'global counter', got: {:?}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_diagnose_auto_detect_python() {
        let error_text = "NameError: name 'json' is not defined";
        let source = "def f():\n    data = json.loads('{}')\n";
        let diag = diagnose(error_text, source, None, None);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "NameError");
        assert!(d.fix.is_some());
    }

    #[test]
    fn test_diagnose_with_traceback() {
        let error_text = "\
Traceback (most recent call last):
  File \"app.py\", line 10, in inc
    counter += 1
UnboundLocalError: cannot access local variable 'counter'";

        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let diag = diagnose(error_text, source, None, None);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "UnboundLocalError");
    }

    #[test]
    fn test_diagnose_unparseable_error() {
        let error_text = "just some random text that is not an error";
        let source = "x = 1\n";
        let diag = diagnose(error_text, source, None, None);
        assert!(diag.is_none());
    }

    #[test]
    fn test_diagnose_parsed_direct() {
        let error = ParsedError {
            error_type: "KeyError".to_string(),
            message: "'name'".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "KeyError: 'name'".to_string(),
            function_name: Some("lookup".to_string()),
            offending_line: None,
        };
        let source = "def lookup(name):\n    d = {'a': 1}\n    return d[name]\n";
        let diag = diagnose_parsed(&error, source, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "KeyError");
    }

    #[test]
    fn test_apply_fix_roundtrip() {
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 3,
                column: None,
                kind: EditKind::InsertBefore,
                new_text: "    global counter".to_string(),
            }],
        };
        let patched = apply_fix(source, &fix);
        assert!(patched.contains("global counter"));
        assert!(patched.contains("counter += 1"));
    }

    // ---- Go integration tests ----

    #[test]
    fn test_diagnose_go_undefined_via_main_dispatch() {
        let error_text = "./main.go:4:7: undefined: fmt";
        let source = "package main\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
        let diag = diagnose(error_text, source, Some("go"), None);
        assert!(diag.is_some(), "Go undefined should diagnose via main dispatch");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "undefined");
        assert_eq!(d.language, "go");
        assert!(d.fix.is_some(), "Should produce a fix for known package");
    }

    #[test]
    fn test_diagnose_go_unused_var_via_main_dispatch() {
        let error_text = "./main.go:4:2: x declared but not used";
        let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
        let diag = diagnose(error_text, source, Some("go"), None);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "unused_var");
        assert!(d.fix.is_some());
    }

    #[test]
    fn test_diagnose_go_auto_detect() {
        let error_text = "./main.go:3:1: missing return at end of function";
        let source = "package main\n\nfunc f() int {\n\tprintln(42)\n}\n";
        let diag = diagnose(error_text, source, None, None);
        assert!(diag.is_some(), "Go error should be auto-detected");
        let d = diag.unwrap();
        assert_eq!(d.language, "go");
        assert_eq!(d.error_code, "missing_return");
    }

    #[test]
    fn test_diagnose_parsed_go() {
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"os\" imported and not used".to_string(),
            file: Some(std::path::PathBuf::from("./main.go")),
            line: Some(3),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"os\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };
        let source = "package main\n\nimport \"os\"\n\nfunc main() {\n}\n";
        let diag = diagnose_parsed(&error, source, None);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "unused_import");
        assert_eq!(d.language, "go");
    }
}
