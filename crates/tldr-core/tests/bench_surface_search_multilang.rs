//! Benchmark-style integration tests for Surface, Search, and Misc command groups.
//!
//! # Surface group
//! - `surface` (extract_api_surface) -- API surface extraction across 5 languages
//! - `diagnostics` -- type/severity construction and filtering
//!
//! # Search group
//! - `search` (text::search) -- regex search with context
//! - `enriched_search` -- BM25 + tree-sitter enriched search
//! - `references` (find_references) -- all references to a symbol
//! - `definition` (find_definition) -- go-to-definition
//!
//! # Misc group
//! - `todo` (run_todo) -- aggregated improvement suggestions
//! - `context` (get_relevant_context) -- LLM-ready context builder

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use tldr_core::analysis::references::{
    find_definition, find_references, DefinitionKind, ReferenceKind, ReferencesOptions,
};
use tldr_core::context::get_relevant_context;
use tldr_core::diagnostics::{
    compute_exit_code, compute_summary, dedupe_diagnostics, filter_diagnostics_by_severity,
    Diagnostic, DiagnosticsReport, DiagnosticsSummary, Severity as DiagSeverity, ToolResult,
};
use tldr_core::search::enriched::{enriched_search, EnrichedSearchOptions, SearchMode};
use tldr_core::search::text::search;
use tldr_core::surface::{extract_api_surface, ApiKind};
use tldr_core::wrappers::{run_todo, TodoItem};
use tldr_core::Language;

// =============================================================================
// Helper: create temp fixtures
// =============================================================================

/// Create a temp directory with a single file and return (dir, file_path).
fn write_fixture(dir: &TempDir, filename: &str, content: &str) -> PathBuf {
    let path = dir.path().join(filename);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

// =============================================================================
// SURFACE tests -- Python
// =============================================================================

mod surface_python {
    use super::*;

    fn python_fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let content = r#"
"""Public module docstring."""

MY_CONST = 42
_PRIVATE_CONST = 99

def public_func(x: int, y: str = "hello") -> bool:
    """A public function."""
    return True

def _private_func():
    """A private function."""
    pass

class MyClass:
    """A public class."""

    def method_one(self, n: int) -> None:
        """Public method."""
        pass

    def _private_method(self):
        pass

class _PrivateClass:
    pass
"#;
        let file_path = write_fixture(&dir, "lib.py", content);
        (dir, file_path)
    }

    #[test]
    fn test_surface_python_extracts_public_apis() {
        let (dir, _file_path) = python_fixture();
        let result = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            false, // exclude private
            None,
            None,
        );

        assert!(
            result.is_ok(),
            "surface extraction failed: {:?}",
            result.err()
        );
        let surface = result.unwrap();

        assert_eq!(surface.language, "python");
        assert!(!surface.apis.is_empty(), "expected non-empty API surface");
        assert_eq!(surface.total, surface.apis.len());

        // Collect all qualified names
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Public function must be present
        assert!(
            names.iter().any(|n| n.contains("public_func")),
            "expected public_func in surface, got: {:?}",
            names
        );

        // Public class must be present
        assert!(
            names.iter().any(|n| n.contains("MyClass")),
            "expected MyClass in surface, got: {:?}",
            names
        );

        // Constant should be present
        assert!(
            names.iter().any(|n| n.contains("MY_CONST")),
            "expected MY_CONST in surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_python_excludes_private() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            false,
            None,
            None,
        )
        .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Private items must NOT be present
        assert!(
            !names.iter().any(|n| n.contains("_private_func")),
            "expected _private_func excluded, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("_PrivateClass")),
            "expected _PrivateClass excluded, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("_PRIVATE_CONST")),
            "expected _PRIVATE_CONST excluded, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_python_includes_private_when_flagged() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            true, // include private
            None,
            None,
        )
        .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Private items SHOULD be present when include_private is true
        assert!(
            names.iter().any(|n| n.contains("_private_func")),
            "expected _private_func included with include_private=true, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_python_api_kinds() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            false,
            None,
            None,
        )
        .unwrap();

        // Find the function entry
        let func_entry = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("public_func"));
        assert!(func_entry.is_some(), "public_func not found in surface");
        assert_eq!(func_entry.unwrap().kind, ApiKind::Function);

        // Find the class entry
        let class_entry = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyClass"));
        assert!(class_entry.is_some(), "MyClass not found in surface");
        assert_eq!(class_entry.unwrap().kind, ApiKind::Class);
    }

    #[test]
    fn test_surface_python_function_signature() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            false,
            None,
            None,
        )
        .unwrap();

        let func_entry = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("public_func"))
            .expect("public_func not found");

        // Should have a signature
        assert!(
            func_entry.signature.is_some(),
            "expected signature for public_func"
        );
        let sig = func_entry.signature.as_ref().unwrap();
        assert!(!sig.params.is_empty(), "expected params in signature");
    }

    #[test]
    fn test_surface_python_location() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            false,
            None,
            None,
        )
        .unwrap();

        let func_entry = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("public_func"))
            .expect("public_func not found");

        // Should have a location with non-zero line
        assert!(
            func_entry.location.is_some(),
            "expected location for public_func"
        );
        let loc = func_entry.location.as_ref().unwrap();
        assert!(
            loc.line > 0,
            "expected positive line number, got {}",
            loc.line
        );
    }

    #[test]
    fn test_surface_python_limit() {
        let (dir, _) = python_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("python"),
            true, // include private so there are many
            Some(2),
            None,
        )
        .unwrap();

        assert!(
            surface.apis.len() <= 2,
            "expected at most 2 APIs with limit=2, got {}",
            surface.apis.len()
        );
    }
}

// =============================================================================
// SURFACE tests -- Rust
// =============================================================================

mod surface_rust {
    use super::*;

    fn rust_fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        // Create a lib.rs in a src/ subdirectory as Rust convention
        let content = r#"
/// A public constant
pub const MY_CONST: u32 = 42;

const PRIVATE_CONST: u32 = 99;

/// A public function
pub fn public_func(x: i32, y: &str) -> bool {
    true
}

fn private_func() -> u32 {
    0
}

/// A public struct
pub struct MyStruct {
    pub field: String,
}

struct PrivateStruct {
    data: Vec<u8>,
}

/// A public trait
pub trait MyTrait {
    fn required_method(&self) -> i32;
}

/// A public enum
pub enum MyEnum {
    VariantA,
    VariantB(i32),
}
"#;
        let file_path = write_fixture(&dir, "lib.rs", content);
        (dir, file_path)
    }

    #[test]
    fn test_surface_rust_extracts_pub_items() {
        let (dir, _) = rust_fixture();
        let result = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("rust"),
            false,
            None,
            None,
        );

        assert!(
            result.is_ok(),
            "Rust surface extraction failed: {:?}",
            result.err()
        );
        let surface = result.unwrap();

        assert_eq!(surface.language, "rust");
        assert!(
            !surface.apis.is_empty(),
            "expected non-empty Rust API surface"
        );

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Public items
        assert!(
            names.iter().any(|n| n.contains("public_func")),
            "expected public_func in Rust surface, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MyStruct")),
            "expected MyStruct in Rust surface, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MY_CONST")),
            "expected MY_CONST in Rust surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_rust_excludes_non_pub() {
        let (dir, _) = rust_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("rust"),
            false,
            None,
            None,
        )
        .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            !names.iter().any(|n| n.contains("private_func")),
            "expected private_func excluded from Rust surface, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("PrivateStruct")),
            "expected PrivateStruct excluded from Rust surface, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("PRIVATE_CONST")),
            "expected PRIVATE_CONST excluded from Rust surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_rust_api_kinds() {
        let (dir, _) = rust_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("rust"),
            false,
            None,
            None,
        )
        .unwrap();

        // Check kinds
        let func = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("public_func"));
        assert!(func.is_some());
        assert_eq!(func.unwrap().kind, ApiKind::Function);

        let st = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyStruct"));
        assert!(st.is_some());
        assert_eq!(st.unwrap().kind, ApiKind::Struct);

        let tr = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyTrait"));
        assert!(tr.is_some());
        assert_eq!(tr.unwrap().kind, ApiKind::Trait);

        let en = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyEnum"));
        assert!(en.is_some());
        assert_eq!(en.unwrap().kind, ApiKind::Enum);
    }
}

// =============================================================================
// SURFACE tests -- Go
// =============================================================================

mod surface_go {
    use super::*;

    fn go_fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let content = r#"package mypackage

// PublicFunc is an exported function.
func PublicFunc(x int, y string) bool {
	return true
}

func privateFunc() int {
	return 0
}

// MyStruct is an exported struct.
type MyStruct struct {
	Name string
	age  int
}

type privateStruct struct {
	data []byte
}

// MyInterface is an exported interface.
type MyInterface interface {
	DoSomething() error
}
"#;
        let file_path = write_fixture(&dir, "lib.go", content);
        (dir, file_path)
    }

    #[test]
    fn test_surface_go_extracts_exported() {
        let (dir, _) = go_fixture();
        let result =
            extract_api_surface(dir.path().to_str().unwrap(), Some("go"), false, None, None);

        assert!(
            result.is_ok(),
            "Go surface extraction failed: {:?}",
            result.err()
        );
        let surface = result.unwrap();

        assert_eq!(surface.language, "go");
        assert!(
            !surface.apis.is_empty(),
            "expected non-empty Go API surface"
        );

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Exported (uppercase) items
        assert!(
            names.iter().any(|n| n.contains("PublicFunc")),
            "expected PublicFunc in Go surface, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MyStruct")),
            "expected MyStruct in Go surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_go_excludes_unexported() {
        let (dir, _) = go_fixture();
        let surface =
            extract_api_surface(dir.path().to_str().unwrap(), Some("go"), false, None, None)
                .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        // Unexported (lowercase) items must be excluded
        assert!(
            !names.iter().any(|n| n.contains("privateFunc")),
            "expected privateFunc excluded from Go surface, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("privateStruct")),
            "expected privateStruct excluded from Go surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_go_api_kinds() {
        let (dir, _) = go_fixture();
        let surface =
            extract_api_surface(dir.path().to_str().unwrap(), Some("go"), false, None, None)
                .unwrap();

        let func = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("PublicFunc"));
        assert!(func.is_some(), "PublicFunc not found");
        assert_eq!(func.unwrap().kind, ApiKind::Function);

        let st = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyStruct"));
        assert!(st.is_some(), "MyStruct not found");
        assert_eq!(st.unwrap().kind, ApiKind::Struct);
    }
}

// =============================================================================
// SURFACE tests -- JavaScript
// =============================================================================

mod surface_javascript {
    use super::*;

    fn js_fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let content = r#"
/**
 * A public exported function.
 * @param {number} x - The input number
 * @returns {boolean} The result
 */
export function publicFunc(x) {
    return true;
}

function internalFunc() {
    return 42;
}

export class MyClass {
    constructor(name) {
        this.name = name;
    }

    greet() {
        return `Hello, ${this.name}`;
    }
}

export const MY_CONST = 42;
"#;
        let file_path = write_fixture(&dir, "lib.mjs", content);
        (dir, file_path)
    }

    #[test]
    fn test_surface_javascript_extracts_exports() {
        let (dir, _) = js_fixture();
        let result = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("javascript"),
            false,
            None,
            None,
        );

        assert!(
            result.is_ok(),
            "JavaScript surface extraction failed: {:?}",
            result.err()
        );
        let surface = result.unwrap();

        assert_eq!(surface.language, "javascript");
        assert!(
            !surface.apis.is_empty(),
            "expected non-empty JS API surface"
        );

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("publicFunc")),
            "expected publicFunc in JS surface, got: {:?}",
            names
        );

        assert!(
            names.iter().any(|n| n.contains("MyClass")),
            "expected MyClass in JS surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_javascript_excludes_non_exported() {
        let (dir, _) = js_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("javascript"),
            false,
            None,
            None,
        )
        .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            !names.iter().any(|n| n.contains("internalFunc")),
            "expected internalFunc excluded from JS surface, got: {:?}",
            names
        );
    }
}

// =============================================================================
// SURFACE tests -- TypeScript
// =============================================================================

mod surface_typescript {
    use super::*;

    fn ts_fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let content = r#"
export interface MyInterface {
    id: number;
    name: string;
    process(): void;
}

export type MyType = string | number;

export function publicFunc(x: number, y: string): boolean {
    return true;
}

function privateFunc(): void {}

export class MyClass {
    constructor(public name: string) {}

    greet(): string {
        return `Hello, ${this.name}`;
    }
}

export const MY_CONST: number = 42;
"#;
        let file_path = write_fixture(&dir, "lib.ts", content);
        (dir, file_path)
    }

    #[test]
    fn test_surface_typescript_extracts_exports() {
        let (dir, _) = ts_fixture();
        let result = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("typescript"),
            false,
            None,
            None,
        );

        assert!(
            result.is_ok(),
            "TypeScript surface extraction failed: {:?}",
            result.err()
        );
        let surface = result.unwrap();

        assert_eq!(surface.language, "typescript");
        assert!(
            !surface.apis.is_empty(),
            "expected non-empty TS API surface"
        );

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("publicFunc")),
            "expected publicFunc in TS surface, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MyInterface")),
            "expected MyInterface in TS surface, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("MyClass")),
            "expected MyClass in TS surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_typescript_interface_kind() {
        let (dir, _) = ts_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("typescript"),
            false,
            None,
            None,
        )
        .unwrap();

        let iface = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyInterface"));
        assert!(iface.is_some(), "MyInterface not found in TS surface");
        assert_eq!(iface.unwrap().kind, ApiKind::Interface);
    }

    #[test]
    fn test_surface_typescript_excludes_non_exported() {
        let (dir, _) = ts_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("typescript"),
            false,
            None,
            None,
        )
        .unwrap();

        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            !names.iter().any(|n| n.contains("privateFunc")),
            "expected privateFunc excluded from TS surface, got: {:?}",
            names
        );
    }

    #[test]
    fn test_surface_typescript_type_alias() {
        let (dir, _) = ts_fixture();
        let surface = extract_api_surface(
            dir.path().to_str().unwrap(),
            Some("typescript"),
            false,
            None,
            None,
        )
        .unwrap();

        let type_entry = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("MyType"));
        assert!(type_entry.is_some(), "MyType not found in TS surface");
        assert_eq!(type_entry.unwrap().kind, ApiKind::TypeAlias);
    }
}

// =============================================================================
// SURFACE tests -- unsupported language
// =============================================================================

#[test]
fn test_surface_unsupported_language_errors() {
    let dir = TempDir::new().unwrap();
    write_fixture(&dir, "lib.rb", "def hello; end");

    let result = extract_api_surface(
        dir.path().to_str().unwrap(),
        Some("ruby"),
        false,
        None,
        None,
    );

    assert!(
        result.is_err(),
        "expected error for unsupported language, got: {:?}",
        result
    );
}

// =============================================================================
// SURFACE tests -- lookup filter
// =============================================================================

#[test]
fn test_surface_lookup_filters_to_single_entry() {
    let dir = TempDir::new().unwrap();
    let content = r#"
def alpha():
    pass

def beta():
    pass

def gamma():
    pass
"#;
    write_fixture(&dir, "multi.py", content);

    let surface = extract_api_surface(
        dir.path().to_str().unwrap(),
        Some("python"),
        false,
        None,
        Some("beta"),
    )
    .unwrap();

    assert_eq!(
        surface.apis.len(),
        1,
        "lookup should filter to exactly 1 entry"
    );
    assert!(surface.apis[0].qualified_name.contains("beta"));
}

// =============================================================================
// SEARCH tests -- text::search
// =============================================================================

mod search_text {
    use super::*;

    fn search_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "main.py",
            r#"
def main():
    """Entry point."""
    result = process_data([1, 2, 3])
    print(result)

def process_data(items):
    total = 0
    for item in items:
        total += item
    return total
"#,
        );
        write_fixture(
            &dir,
            "utils.py",
            r#"
def helper_function(x):
    return x * 2

def process_data(data):
    """Another process_data in utils."""
    return [helper_function(d) for d in data]
"#,
        );
        write_fixture(
            &dir,
            "config.js",
            r#"
function setupConfig() {
    return { debug: true };
}

function process_data(input) {
    return input.map(x => x + 1);
}
"#,
        );
        dir
    }

    #[test]
    fn test_search_finds_pattern() {
        let dir = search_fixture();

        let results = search("process_data", dir.path(), None, 0, 100, 100, None).unwrap();

        assert!(
            !results.is_empty(),
            "expected search to find 'process_data' matches"
        );
    }

    #[test]
    fn test_search_returns_file_and_line() {
        let dir = search_fixture();

        let results = search("def main", dir.path(), None, 0, 100, 100, None).unwrap();

        assert!(!results.is_empty(), "expected 'def main' match");
        let first = &results[0];
        assert!(first.line > 0, "expected positive line number");
        assert!(
            !first.file.to_string_lossy().is_empty(),
            "expected non-empty file path"
        );
        assert!(
            first.content.contains("def main"),
            "expected content to contain 'def main', got: {}",
            first.content
        );
    }

    #[test]
    fn test_search_with_context_lines() {
        let dir = search_fixture();

        let results = search("def main", dir.path(), None, 2, 100, 100, None).unwrap();

        assert!(!results.is_empty());
        let first = &results[0];
        assert!(
            first.context.is_some(),
            "expected context lines with context_lines=2"
        );
        assert!(
            !first.context.as_ref().unwrap().is_empty(),
            "expected non-empty context"
        );
    }

    #[test]
    fn test_search_filters_by_extension() {
        let dir = search_fixture();

        let py_exts: HashSet<String> = [".py".to_string()].into_iter().collect();
        let results = search(
            "process_data",
            dir.path(),
            Some(&py_exts),
            0,
            100,
            100,
            None,
        )
        .unwrap();

        // All results should be .py files only
        for m in &results {
            assert!(
                m.file.to_string_lossy().ends_with(".py"),
                "expected .py file, got: {:?}",
                m.file
            );
        }

        // Also verify the JS file's process_data is excluded
        let js_exts: HashSet<String> = [".js".to_string()].into_iter().collect();
        let js_results = search(
            "process_data",
            dir.path(),
            Some(&js_exts),
            0,
            100,
            100,
            None,
        )
        .unwrap();

        for m in &js_results {
            assert!(
                m.file.to_string_lossy().ends_with(".js"),
                "expected .js file, got: {:?}",
                m.file
            );
        }
    }

    #[test]
    fn test_search_regex_pattern() {
        let dir = search_fixture();

        let results = search(r"def\s+\w+\(", dir.path(), None, 0, 100, 100, None).unwrap();

        assert!(
            results.len() >= 3,
            "expected at least 3 function definitions, got {}",
            results.len()
        );
    }

    #[test]
    fn test_search_respects_max_results() {
        let dir = search_fixture();

        let results = search("process_data", dir.path(), None, 0, 1, 100, None).unwrap();

        assert!(
            results.len() <= 1,
            "expected at most 1 result with max_results=1, got {}",
            results.len()
        );
    }

    #[test]
    fn test_search_nonexistent_pattern_returns_empty() {
        let dir = search_fixture();

        let results = search(
            "zzz_nonexistent_pattern_xyz",
            dir.path(),
            None,
            0,
            100,
            100,
            None,
        )
        .unwrap();

        assert!(
            results.is_empty(),
            "expected no matches for nonexistent pattern"
        );
    }

    #[test]
    fn test_search_invalid_regex_returns_error() {
        let dir = search_fixture();

        let result = search("[invalid(regex", dir.path(), None, 0, 100, 100, None);

        assert!(result.is_err(), "expected error for invalid regex");
    }
}

// =============================================================================
// SEARCH tests -- enriched_search
// =============================================================================

mod search_enriched {
    use super::*;

    fn enriched_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "calculator.py",
            r#"
def add(a, b):
    """Add two numbers."""
    return a + b

def subtract(a, b):
    """Subtract b from a."""
    return a - b

def multiply(a, b):
    """Multiply two numbers."""
    return a * b

def divide(a, b):
    """Divide a by b."""
    if b == 0:
        raise ValueError("Cannot divide by zero")
    return a / b

def calculate(op, a, b):
    """Perform a calculation."""
    if op == "add":
        return add(a, b)
    elif op == "sub":
        return subtract(a, b)
    elif op == "mul":
        return multiply(a, b)
    elif op == "div":
        return divide(a, b)
"#,
        );
        dir
    }

    #[test]
    fn test_enriched_search_finds_functions() {
        let dir = enriched_fixture();

        // Use regex mode to directly find function definitions -- BM25 on very
        // small corpuses can produce zero results because tokenization needs
        // enough documents for TF-IDF to score hits.
        let options = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: false,
            search_mode: SearchMode::Regex(r"def\s+(add|subtract|multiply)".to_string()),
        };

        let result = enriched_search(
            "add subtract multiply",
            dir.path(),
            Language::Python,
            options,
        );

        assert!(result.is_ok(), "enriched_search failed: {:?}", result.err());
        let report = result.unwrap();
        assert!(
            !report.results.is_empty(),
            "expected enriched search results"
        );

        // Each result should have a name, file, and signature
        for r in &report.results {
            assert!(!r.name.is_empty(), "expected non-empty name");
            assert!(
                !r.file.to_string_lossy().is_empty(),
                "expected non-empty file path"
            );
            assert!(
                !r.signature.is_empty(),
                "expected non-empty signature for {}",
                r.name
            );
            assert!(
                r.line_range.0 > 0,
                "expected positive start line for {}",
                r.name
            );
            assert!(
                r.line_range.1 >= r.line_range.0,
                "expected end >= start for {}",
                r.name
            );
        }
    }

    #[test]
    fn test_enriched_search_returns_score() {
        let dir = enriched_fixture();

        let options = EnrichedSearchOptions {
            top_k: 5,
            include_callgraph: false,
            search_mode: SearchMode::default(),
        };

        let report =
            enriched_search("calculate add", dir.path(), Language::Python, options).unwrap();

        if !report.results.is_empty() {
            // Results should be sorted by score descending
            for window in report.results.windows(2) {
                assert!(
                    window[0].score >= window[1].score,
                    "results should be sorted by score descending: {} >= {}",
                    window[0].score,
                    window[1].score
                );
            }
        }
    }

    #[test]
    fn test_enriched_search_regex_mode() {
        let dir = enriched_fixture();

        let options = EnrichedSearchOptions {
            top_k: 10,
            include_callgraph: false,
            search_mode: SearchMode::Regex(r"def\s+(add|subtract)".to_string()),
        };

        let result = enriched_search("add subtract", dir.path(), Language::Python, options);

        assert!(
            result.is_ok(),
            "regex enriched search failed: {:?}",
            result.err()
        );
        let report = result.unwrap();
        // Should find at least the add and subtract functions
        let names: Vec<&str> = report.results.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.iter().any(|n| *n == "add" || *n == "subtract"),
            "expected add or subtract in regex enriched results, got: {:?}",
            names
        );
    }
}

// =============================================================================
// REFERENCES tests
// =============================================================================

mod references_tests {
    use super::*;

    fn references_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "main.py",
            r#"from utils import helper

def main():
    x = helper(10)
    y = helper(20)
    result = x + y
    print(result)
    helper(30)
"#,
        );
        write_fixture(
            &dir,
            "utils.py",
            r#"def helper(value):
    """A helper function used multiple times."""
    return value * 2
"#,
        );
        dir
    }

    #[test]
    fn test_references_finds_symbol() {
        let dir = references_fixture();

        let options = ReferencesOptions::new().with_language("python".to_string());

        let result = find_references("helper", dir.path(), &options);

        assert!(result.is_ok(), "find_references failed: {:?}", result.err());
        let report = result.unwrap();
        assert_eq!(report.symbol, "helper");
        assert!(
            !report.references.is_empty(),
            "expected at least one reference to 'helper'"
        );
        assert!(
            report.total_references > 0,
            "expected positive total_references"
        );
    }

    #[test]
    fn test_references_counts_all_usages() {
        let dir = references_fixture();

        let options = ReferencesOptions::new().with_language("python".to_string());

        let report = find_references("helper", dir.path(), &options).unwrap();

        // helper is used: import, 3 calls, 1 definition = at least 4 references
        assert!(
            report.total_references >= 3,
            "expected at least 3 references to 'helper', got {}",
            report.total_references
        );
    }

    #[test]
    fn test_references_includes_file_and_line() {
        let dir = references_fixture();

        let options = ReferencesOptions::new().with_language("python".to_string());

        let report = find_references("helper", dir.path(), &options).unwrap();

        for reference in &report.references {
            assert!(reference.line > 0, "expected positive line number");
            assert!(reference.column > 0, "expected positive column number");
            assert!(
                !reference.context.is_empty(),
                "expected non-empty context for reference"
            );
        }
    }

    #[test]
    fn test_references_with_limit() {
        let dir = references_fixture();

        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_limit(2);

        let report = find_references("helper", dir.path(), &options).unwrap();

        assert!(
            report.references.len() <= 2,
            "expected at most 2 references with limit=2, got {}",
            report.references.len()
        );
    }

    #[test]
    fn test_references_unknown_symbol_returns_empty() {
        let dir = references_fixture();

        let options = ReferencesOptions::new().with_language("python".to_string());

        let report = find_references("nonexistent_symbol_xyz", dir.path(), &options).unwrap();

        assert!(
            report.references.is_empty(),
            "expected no references for unknown symbol"
        );
    }
}

// =============================================================================
// DEFINITION tests
// =============================================================================

mod definition_tests {
    use super::*;

    fn definition_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "calculator.py",
            r#"def add(a, b):
    return a + b

def subtract(a, b):
    return a - b

class Calculator:
    def compute(self, op, a, b):
        if op == "add":
            return add(a, b)
        return subtract(a, b)
"#,
        );
        write_fixture(
            &dir,
            "app.go",
            r#"package app

func ProcessRequest(req string) string {
	return "processed: " + req
}

type Handler struct {
	Name string
}

func (h *Handler) Handle() string {
	return ProcessRequest(h.Name)
}
"#,
        );
        write_fixture(
            &dir,
            "module.ts",
            r#"export function parseInput(raw: string): number {
    return parseInt(raw, 10);
}

export class Parser {
    parse(input: string): number {
        return parseInput(input);
    }
}
"#,
        );
        write_fixture(
            &dir,
            "lib.rs",
            r#"pub fn transform(input: &str) -> String {
    input.to_uppercase()
}

pub struct Transformer {
    prefix: String,
}

impl Transformer {
    pub fn new(prefix: &str) -> Self {
        Self { prefix: prefix.to_string() }
    }
}
"#,
        );
        dir
    }

    #[test]
    fn test_definition_python() {
        let dir = definition_fixture();

        let result = find_definition("add", dir.path(), Some("python"));

        assert!(result.is_ok(), "find_definition failed: {:?}", result.err());
        let def = result.unwrap();
        assert!(def.is_some(), "expected to find definition of 'add'");

        let def = def.unwrap();
        assert!(
            def.file.to_string_lossy().contains("calculator.py"),
            "expected definition in calculator.py, got: {:?}",
            def.file
        );
        assert_eq!(def.line, 1, "expected 'add' defined at line 1");
        assert_eq!(def.kind, DefinitionKind::Function);
    }

    #[test]
    fn test_definition_python_class() {
        let dir = definition_fixture();

        let def = find_definition("Calculator", dir.path(), Some("python"))
            .unwrap()
            .expect("expected to find Calculator class definition");

        assert_eq!(def.kind, DefinitionKind::Class);
        assert!(def.file.to_string_lossy().contains("calculator.py"));
    }

    #[test]
    fn test_definition_go() {
        let dir = definition_fixture();

        let def = find_definition("ProcessRequest", dir.path(), Some("go"))
            .unwrap()
            .expect("expected to find ProcessRequest definition");

        assert!(def.file.to_string_lossy().contains("app.go"));
        assert_eq!(def.kind, DefinitionKind::Function);
        assert!(def.line > 0, "expected positive line number");
    }

    #[test]
    fn test_definition_typescript() {
        let dir = definition_fixture();

        let def = find_definition("parseInput", dir.path(), None)
            .unwrap()
            .expect("expected to find parseInput definition");

        assert!(def.file.to_string_lossy().contains("module.ts"));
        assert_eq!(def.kind, DefinitionKind::Function);
    }

    #[test]
    fn test_definition_rust() {
        let dir = definition_fixture();

        let def = find_definition("transform", dir.path(), Some("rust"))
            .unwrap()
            .expect("expected to find transform definition");

        assert!(def.file.to_string_lossy().contains("lib.rs"));
        assert_eq!(def.kind, DefinitionKind::Function);
    }

    #[test]
    fn test_definition_not_found() {
        let dir = definition_fixture();

        let result = find_definition("nonexistent_function_xyz", dir.path(), None).unwrap();

        assert!(
            result.is_none(),
            "expected None for nonexistent symbol, got: {:?}",
            result
        );
    }
}

// =============================================================================
// DIAGNOSTICS tests -- type construction and filtering
// =============================================================================

mod diagnostics_tests {
    use super::*;

    fn make_diagnostics() -> Vec<Diagnostic> {
        vec![
            Diagnostic {
                file: PathBuf::from("src/main.py"),
                line: 10,
                column: 5,
                end_line: None,
                end_column: None,
                severity: DiagSeverity::Error,
                message: "Undefined variable 'x'".to_string(),
                code: Some("E0001".to_string()),
                source: "pyright".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("src/main.py"),
                line: 15,
                column: 1,
                end_line: None,
                end_column: None,
                severity: DiagSeverity::Warning,
                message: "Unused import 'os'".to_string(),
                code: Some("W0611".to_string()),
                source: "ruff".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("src/utils.py"),
                line: 3,
                column: 1,
                end_line: None,
                end_column: None,
                severity: DiagSeverity::Information,
                message: "Consider using f-string".to_string(),
                code: Some("UP031".to_string()),
                source: "ruff".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("src/utils.py"),
                line: 7,
                column: 1,
                end_line: None,
                end_column: None,
                severity: DiagSeverity::Hint,
                message: "Add type annotation".to_string(),
                code: None,
                source: "pyright".to_string(),
                url: None,
            },
        ]
    }

    #[test]
    fn test_diagnostics_compute_summary() {
        let diags = make_diagnostics();
        let summary = compute_summary(&diags);

        assert_eq!(summary.errors, 1);
        assert_eq!(summary.warnings, 1);
        assert_eq!(summary.info, 1);
        assert_eq!(summary.hints, 1);
        assert_eq!(summary.total, 4);
    }

    #[test]
    fn test_diagnostics_filter_by_severity_error_only() {
        let diags = make_diagnostics();
        let filtered = filter_diagnostics_by_severity(&diags, DiagSeverity::Error);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].severity, DiagSeverity::Error);
    }

    #[test]
    fn test_diagnostics_filter_by_severity_warning() {
        let diags = make_diagnostics();
        let filtered = filter_diagnostics_by_severity(&diags, DiagSeverity::Warning);

        // Error < Warning, so Error + Warning pass
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|d| d.severity <= DiagSeverity::Warning));
    }

    #[test]
    fn test_diagnostics_filter_by_severity_all() {
        let diags = make_diagnostics();
        let filtered = filter_diagnostics_by_severity(&diags, DiagSeverity::Hint);

        assert_eq!(filtered.len(), 4);
    }

    #[test]
    fn test_diagnostics_dedupe() {
        let mut diags = make_diagnostics();
        // Add a duplicate of the first diagnostic
        diags.push(Diagnostic {
            file: PathBuf::from("src/main.py"),
            line: 10,
            column: 5,
            end_line: None,
            end_column: None,
            severity: DiagSeverity::Error,
            message: "Undefined variable 'x'".to_string(),
            code: Some("E0001".to_string()),
            source: "pyright".to_string(),
            url: None,
        });

        assert_eq!(diags.len(), 5);
        let deduped = dedupe_diagnostics(diags);
        assert_eq!(deduped.len(), 4, "expected duplicate to be removed");
    }

    #[test]
    fn test_diagnostics_exit_code_no_errors() {
        let summary = DiagnosticsSummary {
            errors: 0,
            warnings: 2,
            info: 1,
            hints: 0,
            total: 3,
        };

        assert_eq!(compute_exit_code(&summary, false), 0);
    }

    #[test]
    fn test_diagnostics_exit_code_with_errors() {
        let summary = DiagnosticsSummary {
            errors: 1,
            warnings: 0,
            info: 0,
            hints: 0,
            total: 1,
        };

        assert_eq!(compute_exit_code(&summary, false), 1);
    }

    #[test]
    fn test_diagnostics_exit_code_strict_mode() {
        let summary = DiagnosticsSummary {
            errors: 0,
            warnings: 1,
            info: 0,
            hints: 0,
            total: 1,
        };

        // Without strict: warnings don't cause failure
        assert_eq!(compute_exit_code(&summary, false), 0);
        // With strict: warnings cause failure
        assert_eq!(compute_exit_code(&summary, true), 1);
    }

    #[test]
    fn test_diagnostics_report_construction() {
        let diags = make_diagnostics();
        let summary = compute_summary(&diags);

        let report = DiagnosticsReport {
            diagnostics: diags.clone(),
            summary,
            tools_run: vec![ToolResult {
                name: "pyright".to_string(),
                version: Some("1.1.0".to_string()),
                success: true,
                duration_ms: 1500,
                diagnostic_count: 2,
                error: None,
            }],
            files_analyzed: 2,
        };

        assert_eq!(report.diagnostics.len(), 4);
        assert_eq!(report.files_analyzed, 2);
        assert_eq!(report.tools_run.len(), 1);
        assert!(report.tools_run[0].success);
    }

    #[test]
    fn test_diagnostic_dedupe_key() {
        let d1 = &make_diagnostics()[0];
        let d2 = &make_diagnostics()[0];

        // Same diagnostic should produce same dedupe key
        assert_eq!(d1.dedupe_key(), d2.dedupe_key());

        // Different diagnostics should produce different keys
        let d3 = &make_diagnostics()[1];
        assert_ne!(d1.dedupe_key(), d3.dedupe_key());
    }

    #[test]
    fn test_diagnostics_severity_ordering() {
        // Error=1 < Warning=2 < Information=3 < Hint=4
        assert!(DiagSeverity::Error < DiagSeverity::Warning);
        assert!(DiagSeverity::Warning < DiagSeverity::Information);
        assert!(DiagSeverity::Information < DiagSeverity::Hint);
    }

    #[test]
    fn test_diagnostics_severity_display() {
        assert_eq!(format!("{}", DiagSeverity::Error), "error");
        assert_eq!(format!("{}", DiagSeverity::Warning), "warning");
        assert_eq!(format!("{}", DiagSeverity::Information), "info");
        assert_eq!(format!("{}", DiagSeverity::Hint), "hint");
    }
}

// =============================================================================
// CONTEXT tests -- get_relevant_context
// =============================================================================

mod context_tests {
    use super::*;

    fn context_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "main.py",
            r#"def main():
    """Main entry point."""
    result = process([1, 2, 3])
    print(result)

def process(items):
    """Process items by transforming each."""
    return [transform(i) for i in items]

def transform(x):
    """Transform a single item."""
    return x * 2 + 1
"#,
        );
        dir
    }

    #[test]
    fn test_context_extracts_entry_point() {
        let dir = context_fixture();

        let result = get_relevant_context(
            dir.path(),
            "main",
            2,
            Language::Python,
            true, // include docstrings
            None,
        );

        assert!(
            result.is_ok(),
            "get_relevant_context failed: {:?}",
            result.err()
        );
        let ctx = result.unwrap();

        assert_eq!(ctx.entry_point, "main");
        assert_eq!(ctx.depth, 2);
        assert!(
            !ctx.functions.is_empty(),
            "expected at least the entry point function"
        );

        // Entry point should be in the functions list
        assert!(
            ctx.functions.iter().any(|f| f.name == "main"),
            "expected 'main' in context functions, got: {:?}",
            ctx.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_context_includes_callees() {
        let dir = context_fixture();

        let ctx =
            get_relevant_context(dir.path(), "main", 2, Language::Python, true, None).unwrap();

        let names: Vec<&str> = ctx.functions.iter().map(|f| f.name.as_str()).collect();

        // At depth 2, main -> process -> transform should all be included
        assert!(
            names.contains(&"main"),
            "expected 'main' in context, got: {:?}",
            names
        );
        assert!(
            names.contains(&"process"),
            "expected 'process' (direct callee) in context, got: {:?}",
            names
        );
    }

    #[test]
    fn test_context_function_has_signature() {
        let dir = context_fixture();

        let ctx =
            get_relevant_context(dir.path(), "main", 1, Language::Python, true, None).unwrap();

        for func in &ctx.functions {
            assert!(
                !func.signature.is_empty(),
                "expected non-empty signature for function {}",
                func.name
            );
        }
    }

    #[test]
    fn test_context_includes_docstrings() {
        let dir = context_fixture();

        let ctx =
            get_relevant_context(dir.path(), "main", 1, Language::Python, true, None).unwrap();

        let main_func = ctx.functions.iter().find(|f| f.name == "main");
        assert!(main_func.is_some(), "expected 'main' in context");

        let main_func = main_func.unwrap();
        assert!(
            main_func.docstring.is_some(),
            "expected docstring for main when include_docstrings=true"
        );
    }

    #[test]
    fn test_context_to_llm_string() {
        let dir = context_fixture();

        let ctx =
            get_relevant_context(dir.path(), "main", 1, Language::Python, true, None).unwrap();

        let llm_str = ctx.to_llm_string();

        assert!(!llm_str.is_empty(), "expected non-empty LLM string");
        assert!(
            llm_str.contains("Code Context"),
            "expected 'Code Context' header in LLM output"
        );
        assert!(llm_str.contains("main"), "expected 'main' in LLM output");
    }

    #[test]
    fn test_context_depth_zero_returns_entry_only() {
        let dir = context_fixture();

        let ctx =
            get_relevant_context(dir.path(), "main", 0, Language::Python, true, None).unwrap();

        // With depth=0, only the entry point function should be included
        assert_eq!(
            ctx.functions.len(),
            1,
            "expected exactly 1 function at depth=0, got: {:?}",
            ctx.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert_eq!(ctx.functions[0].name, "main");
    }

    #[test]
    fn test_context_nonexistent_entry_point_errors() {
        let dir = context_fixture();

        let result = get_relevant_context(
            dir.path(),
            "nonexistent_function_xyz",
            1,
            Language::Python,
            true,
            None,
        );

        assert!(
            result.is_err(),
            "expected error for nonexistent entry point"
        );
    }
}

// =============================================================================
// TODO tests -- run_todo
// =============================================================================

mod todo_tests {
    use super::*;

    fn todo_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Create a file with some complexity and dead code
        write_fixture(
            &dir,
            "complex.py",
            r#"
def very_complex_function(x, y, z):
    """A function with high cyclomatic complexity."""
    if x > 0:
        if y > 0:
            if z > 0:
                return x + y + z
            elif z < -10:
                return x - y
            else:
                return y - z
        elif y < -5:
            if z == 0:
                return -x
            else:
                return x * z
        else:
            return 0
    elif x < 0:
        if y > 0:
            return -x + y
        elif y < 0:
            return x * y
        else:
            return z
    else:
        return z * 2

def dead_function():
    """This function is never called."""
    return 42

def another_dead():
    """Also never called."""
    return "unused"

def simple_func(a, b):
    result = a + b
    return result
"#,
        );
        dir
    }

    #[test]
    fn test_todo_returns_report() {
        let dir = todo_fixture();

        let result = run_todo(dir.path().to_str().unwrap(), Some("py"), true);

        assert!(result.is_ok(), "run_todo failed: {:?}", result.err());
        let report = result.unwrap();

        assert_eq!(report.path, dir.path().to_str().unwrap());
        // Report should have sub_results for the analyses
        assert!(
            !report.sub_results.is_empty(),
            "expected sub_results in todo report"
        );
    }

    #[test]
    fn test_todo_produces_items() {
        let dir = todo_fixture();

        let report = run_todo(dir.path().to_str().unwrap(), Some("py"), true).unwrap();

        // May or may not have items depending on thresholds, but the report
        // structure should be correct
        assert!(
            report.total_elapsed_ms >= 0.0,
            "expected non-negative total_elapsed_ms"
        );
    }

    #[test]
    fn test_todo_item_fields() {
        // Test TodoItem construction directly
        let item = TodoItem {
            category: "complexity".to_string(),
            priority: 2,
            description: "Function has CC > 20".to_string(),
            file: "src/complex.py".to_string(),
            line: 10,
            severity: "high".to_string(),
            score: 25.0,
        };

        assert_eq!(item.category, "complexity");
        assert_eq!(item.priority, 2);
        assert!(item.score > 0.0);
    }

    #[test]
    fn test_todo_quick_mode_skips_similarity() {
        let dir = todo_fixture();

        // Quick mode skips the "similar" (similarity) analysis
        let result = run_todo(dir.path().to_str().unwrap(), Some("py"), true);
        assert!(result.is_ok(), "quick mode failed: {:?}", result.err());

        let report = result.unwrap();
        // In quick mode, similarity analysis should be skipped
        assert!(
            !report.sub_results.contains_key("similar"),
            "expected 'similar' skipped in quick mode"
        );
        // But equivalence should still run
        assert!(
            report.sub_results.contains_key("equivalence"),
            "expected 'equivalence' present in quick mode"
        );
    }

    #[test]
    fn test_todo_full_mode_includes_similarity() {
        let dir = todo_fixture();

        let result = run_todo(dir.path().to_str().unwrap(), Some("py"), false);
        assert!(result.is_ok(), "full mode failed: {:?}", result.err());

        let report = result.unwrap();
        // Full mode should include similarity analysis
        assert!(
            report.sub_results.contains_key("similar"),
            "expected 'similar' present in full mode"
        );
    }
}

// =============================================================================
// Cross-language DEFINITION tests (multi-file)
// =============================================================================

mod definition_multilang {
    use super::*;

    fn multilang_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();

        // JavaScript
        write_fixture(
            &dir,
            "app.js",
            r#"function fetchData(url) {
    return fetch(url).then(r => r.json());
}

class ApiClient {
    constructor(baseUrl) {
        this.baseUrl = baseUrl;
    }
}

module.exports = { fetchData, ApiClient };
"#,
        );

        dir
    }

    #[test]
    fn test_definition_javascript() {
        let dir = multilang_fixture();

        let def = find_definition("fetchData", dir.path(), None)
            .unwrap()
            .expect("expected to find fetchData definition");

        assert!(def.file.to_string_lossy().contains("app.js"));
        assert_eq!(def.kind, DefinitionKind::Function);
        assert_eq!(def.line, 1);
    }
}

// =============================================================================
// Cross-language REFERENCES tests
// =============================================================================

mod references_multilang {
    use super::*;

    #[test]
    fn test_references_go() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "main.go",
            r#"package main

func Helper(x int) int {
	return x * 2
}

func main() {
	a := Helper(10)
	b := Helper(20)
	println(a + b)
}
"#,
        );

        let options = ReferencesOptions::new().with_language("go".to_string());
        let report = find_references("Helper", dir.path(), &options).unwrap();

        assert!(
            report.total_references >= 2,
            "expected at least 2 references to Helper in Go, got {}",
            report.total_references
        );
    }

    #[test]
    fn test_references_typescript() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "utils.ts",
            r#"export function format(s: string): string {
    return s.trim();
}

export function process(input: string): string {
    const trimmed = format(input);
    return format(trimmed + "!");
}
"#,
        );

        let options = ReferencesOptions::new();
        let report = find_references("format", dir.path(), &options).unwrap();

        // format is defined once, called twice
        assert!(
            report.total_references >= 2,
            "expected at least 2 references to format in TS, got {}",
            report.total_references
        );
    }

    #[test]
    fn test_references_rust() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "lib.rs",
            r#"pub fn compute(x: i32) -> i32 {
    x * 2
}

pub fn run() -> i32 {
    let a = compute(5);
    let b = compute(10);
    a + b
}
"#,
        );

        let options = ReferencesOptions::new().with_language("rust".to_string());
        let report = find_references("compute", dir.path(), &options).unwrap();

        assert!(
            report.total_references >= 2,
            "expected at least 2 references to compute in Rust, got {}",
            report.total_references
        );
    }

    #[test]
    fn test_references_javascript() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "app.js",
            r#"function validate(input) {
    return input != null;
}

function processForm(data) {
    if (validate(data.name)) {
        validate(data.email);
        return true;
    }
    return false;
}
"#,
        );

        let options = ReferencesOptions::new();
        let report = find_references("validate", dir.path(), &options).unwrap();

        assert!(
            report.total_references >= 2,
            "expected at least 2 references to validate in JS, got {}",
            report.total_references
        );
    }

    #[test]
    fn test_references_with_kind_filter() {
        let dir = TempDir::new().unwrap();
        write_fixture(
            &dir,
            "example.py",
            r#"def target():
    return 42

result = target()
x = target()
"#,
        );

        let options = ReferencesOptions::new()
            .with_language("python".to_string())
            .with_kinds(vec![ReferenceKind::Call]);

        let report = find_references("target", dir.path(), &options).unwrap();

        // All returned references should be calls
        for reference in &report.references {
            assert_eq!(
                reference.kind,
                ReferenceKind::Call,
                "expected only Call references with kind filter"
            );
        }
    }
}

// =============================================================================
// SEARCH across multiple languages
// =============================================================================

mod search_multilang {
    use super::*;

    fn multilang_search_fixture() -> TempDir {
        let dir = TempDir::new().unwrap();

        write_fixture(
            &dir,
            "handler.py",
            r#"
def handle_request(req):
    return process_input(req.data)

def process_input(data):
    return data.strip()
"#,
        );

        write_fixture(
            &dir,
            "handler.go",
            r#"package handler

func HandleRequest(req string) string {
	return ProcessInput(req)
}

func ProcessInput(data string) string {
	return data
}
"#,
        );

        write_fixture(
            &dir,
            "handler.ts",
            r#"
export function handleRequest(req: string): string {
    return processInput(req);
}

function processInput(data: string): string {
    return data.trim();
}
"#,
        );

        write_fixture(
            &dir,
            "handler.rs",
            r#"
pub fn handle_request(req: &str) -> String {
    process_input(req)
}

fn process_input(data: &str) -> String {
    data.trim().to_string()
}
"#,
        );

        write_fixture(
            &dir,
            "handler.js",
            r#"
function handleRequest(req) {
    return processInput(req);
}

function processInput(data) {
    return data.trim();
}

module.exports = { handleRequest };
"#,
        );

        dir
    }

    #[test]
    fn test_search_across_all_languages() {
        let dir = multilang_search_fixture();

        // Search for a pattern that appears in all files
        let results = search("handle", dir.path(), None, 0, 100, 100, None).unwrap();

        // Should find matches in multiple file types
        let file_extensions: HashSet<String> = results
            .iter()
            .filter_map(|m| {
                m.file
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_string())
            })
            .collect();

        assert!(
            file_extensions.len() >= 3,
            "expected matches across at least 3 languages, got extensions: {:?}",
            file_extensions
        );
    }

    #[test]
    fn test_search_python_specific() {
        let dir = multilang_search_fixture();
        let py_exts: HashSet<String> = [".py".to_string()].into_iter().collect();

        let results = search("def handle", dir.path(), Some(&py_exts), 0, 100, 100, None).unwrap();

        assert!(!results.is_empty(), "expected Python search results");
        assert!(results
            .iter()
            .all(|m| m.file.to_string_lossy().ends_with(".py")));
    }

    #[test]
    fn test_search_go_specific() {
        let dir = multilang_search_fixture();
        let go_exts: HashSet<String> = [".go".to_string()].into_iter().collect();

        let results = search("func Handle", dir.path(), Some(&go_exts), 0, 100, 100, None).unwrap();

        assert!(!results.is_empty(), "expected Go search results");
        assert!(results
            .iter()
            .all(|m| m.file.to_string_lossy().ends_with(".go")));
    }

    #[test]
    fn test_search_typescript_specific() {
        let dir = multilang_search_fixture();
        let ts_exts: HashSet<String> = [".ts".to_string()].into_iter().collect();

        let results = search(
            "function handleRequest",
            dir.path(),
            Some(&ts_exts),
            0,
            100,
            100,
            None,
        )
        .unwrap();

        assert!(!results.is_empty(), "expected TypeScript search results");
        assert!(results
            .iter()
            .all(|m| m.file.to_string_lossy().ends_with(".ts")));
    }

    #[test]
    fn test_search_rust_specific() {
        let dir = multilang_search_fixture();
        let rs_exts: HashSet<String> = [".rs".to_string()].into_iter().collect();

        let results = search(
            "fn handle_request",
            dir.path(),
            Some(&rs_exts),
            0,
            100,
            100,
            None,
        )
        .unwrap();

        assert!(!results.is_empty(), "expected Rust search results");
        assert!(results
            .iter()
            .all(|m| m.file.to_string_lossy().ends_with(".rs")));
    }
}
