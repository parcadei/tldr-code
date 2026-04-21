//! Go-specific API surface extraction.
//!
//! Extracts the complete public API surface from a Go package by:
//! 1. Walking all `.go` files in the source directory
//! 2. Using tree-sitter to parse each file and extract functions, structs,
//!    interfaces, constants, and methods
//! 3. Filtering to exported names only (uppercase first letter convention)
//! 4. Building method sets to track interface satisfaction
//! 5. Generating example usage strings from type signatures

use std::path::{Path, PathBuf};

use crate::ast::extract::extract_from_tree;
use crate::ast::parser::parse;
use crate::types::{ClassInfo, Language};
use crate::TldrResult;

use super::triggers::extract_triggers;
use super::types::{ApiEntry, ApiKind, ApiSurface, Location, Param, ResolvedPackage, Signature};

/// Extract the complete API surface from a Go package directory.
///
/// # Arguments
/// * `resolved` - The resolved package with root directory
/// * `include_private` - Whether to include unexported (lowercase) names
/// * `limit` - Optional maximum number of APIs
///
/// # Returns
/// * `ApiSurface` with all extracted API entries
pub fn extract_go_api_surface(
    resolved: &ResolvedPackage,
    include_private: bool,
    limit: Option<usize>,
) -> TldrResult<ApiSurface> {
    let mut apis = Vec::new();

    // Find all Go source files
    let go_files = find_go_files(&resolved.root_dir);

    // Extract from each file
    for file_path in &go_files {
        let file_apis = extract_from_go_file(
            file_path,
            &resolved.root_dir,
            &resolved.package_name,
            include_private,
        )?;
        apis.extend(file_apis);
    }

    // Apply limit if specified
    if let Some(max) = limit {
        apis.truncate(max);
    }

    let total = apis.len();
    Ok(ApiSurface {
        package: resolved.package_name.clone(),
        language: "go".to_string(),
        total,
        apis,
    })
}

/// Find all `.go` files in the given path (non-recursive for Go packages).
///
/// If `dir` is a single `.go` file, returns just that file (single-file mode).
/// If `dir` is a directory, walks it collecting all `.go` files.
/// Excludes `_test.go` files and `vendor/` directories.
fn find_go_files(dir: &Path) -> Vec<PathBuf> {
    // Single-file mode: dir IS a .go file itself
    if dir.is_file() {
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".go") && !name.ends_with("_test.go") {
                return vec![dir.to_path_buf()];
            }
        }
        return vec![];
    }

    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.ends_with(".go") && !name.ends_with("_test.go") {
                        files.push(path);
                    }
                }
            } else if path.is_dir() {
                // Recurse into subdirectories (for multi-package modules)
                // but skip vendor/ and testdata/
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name != "vendor" && name != "testdata" && !name.starts_with('.') {
                        files.extend(find_go_files(&path));
                    }
                }
            }
        }
    }
    files.sort();
    files
}

/// Check if a Go identifier is exported (starts with uppercase).
fn is_exported(name: &str) -> bool {
    name.chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

/// Extract API entries from a single Go file.
fn extract_from_go_file(
    file_path: &Path,
    root_dir: &Path,
    package_name: &str,
    include_private: bool,
) -> TldrResult<Vec<ApiEntry>> {
    let source = std::fs::read_to_string(file_path).map_err(|e| {
        crate::error::TldrError::parse_error(
            file_path.to_path_buf(),
            None,
            format!("Cannot read: {}", e),
        )
    })?;

    let tree = parse(&source, Language::Go)?;

    // Use extract_from_tree to get module info
    let module_info = extract_from_tree(&tree, &source, Language::Go, file_path, Some(root_dir))?;

    // Compute package path
    let module_path = compute_go_package_path(file_path, root_dir, package_name);
    let relative_path = file_path
        .strip_prefix(root_dir)
        .unwrap_or(file_path)
        .to_path_buf();

    let mut apis = Vec::new();

    // Extract top-level functions (non-method functions)
    for func in &module_info.functions {
        if !include_private && !is_exported(&func.name) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, func.name);
        let params = convert_go_params(&func.params);
        let return_type = func.return_type.clone();
        let signature = Some(Signature {
            params: params.clone(),
            return_type: return_type.clone(),
            is_async: false,
            is_generator: false,
        });

        let example =
            generate_go_function_example(&module_path, &func.name, &params, return_type.as_deref());
        let triggers = extract_triggers(&func.name, func.docstring.as_deref());

        apis.push(ApiEntry {
            qualified_name,
            kind: ApiKind::Function,
            module: module_path.clone(),
            signature,
            docstring: func.docstring.clone().map(|d| truncate_docstring(&d)),
            example,
            triggers,
            is_property: false,
            return_type,
            location: Some(Location {
                file: relative_path.clone(),
                line: func.line_number as usize,
                column: None,
            }),
        });
    }

    // Extract structs and interfaces with their methods
    for class in &module_info.classes {
        if !include_private && !is_exported(&class.name) {
            continue;
        }

        let kind = determine_go_type_kind(class, &source);
        let qualified_name = format!("{}.{}", module_path, class.name);
        let triggers = extract_triggers(&class.name, class.docstring.as_deref());

        // Add the type itself
        apis.push(ApiEntry {
            qualified_name: qualified_name.clone(),
            kind,
            module: module_path.clone(),
            signature: None,
            docstring: class.docstring.clone().map(|d| truncate_docstring(&d)),
            example: generate_go_type_example(&module_path, &class.name, kind),
            triggers,
            is_property: false,
            return_type: None,
            location: Some(Location {
                file: relative_path.clone(),
                line: class.line_number as usize,
                column: None,
            }),
        });

        // Add methods
        for method in &class.methods {
            if !include_private && !is_exported(&method.name) {
                continue;
            }

            let method_qualified = format!("{}.{}", qualified_name, method.name);
            let params = convert_go_params(&method.params);
            let return_type = method.return_type.clone();

            let signature = Some(Signature {
                params: params.clone(),
                return_type: return_type.clone(),
                is_async: false,
                is_generator: false,
            });

            let example = generate_go_method_example(
                &class.name,
                &method.name,
                &params,
                return_type.as_deref(),
            );
            let triggers = extract_triggers(&method.name, method.docstring.as_deref());

            apis.push(ApiEntry {
                qualified_name: method_qualified,
                kind: ApiKind::Method,
                module: module_path.clone(),
                signature,
                docstring: method.docstring.clone().map(|d| truncate_docstring(&d)),
                example,
                triggers,
                is_property: false,
                return_type,
                location: Some(Location {
                    file: relative_path.clone(),
                    line: method.line_number as usize,
                    column: None,
                }),
            });
        }
    }

    // Extract module-level constants
    for field in &module_info.constants {
        if !include_private && !is_exported(&field.name) {
            continue;
        }

        let qualified_name = format!("{}.{}", module_path, field.name);
        let triggers = extract_triggers(&field.name, None);

        apis.push(ApiEntry {
            qualified_name,
            kind: ApiKind::Constant,
            module: module_path.clone(),
            signature: None,
            docstring: None,
            example: Some(format!("{}.{}", module_path, field.name)),
            triggers,
            is_property: false,
            return_type: field.field_type.clone(),
            location: Some(Location {
                file: relative_path.clone(),
                line: field.line_number as usize,
                column: None,
            }),
        });
    }

    Ok(apis)
}

// ============================================================================
// Helpers
// ============================================================================

/// Compute the Go package path from a file path.
///
/// Examples:
/// - `pkg.go` in root -> `<package>`
/// - `sub/pkg.go` -> `<package>/sub`
fn compute_go_package_path(file_path: &Path, root_dir: &Path, package_name: &str) -> String {
    let relative = file_path.strip_prefix(root_dir).unwrap_or(file_path);
    let parent = relative.parent();

    match parent {
        Some(p) if !p.as_os_str().is_empty() => {
            let sub_path = p.to_string_lossy().replace('\\', "/");
            format!("{}/{}", package_name, sub_path)
        }
        _ => package_name.to_string(),
    }
}

/// Determine the kind of a Go type (struct, interface, or enum-like).
fn determine_go_type_kind(class: &ClassInfo, source: &str) -> ApiKind {
    let lines: Vec<&str> = source.lines().collect();
    let line_idx = (class.line_number as usize).saturating_sub(1);

    if line_idx < lines.len() {
        let line = lines[line_idx];
        if line.contains("interface") {
            return ApiKind::Interface;
        }
    }

    // Check if it looks like a struct by having fields
    ApiKind::Struct
}

/// Convert Go param strings to Param structs.
///
/// Go params are typically "name type" pairs. The extractor returns them
/// in various formats.
fn convert_go_params(params: &[String]) -> Vec<Param> {
    params
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            let parts: Vec<&str> = p.splitn(2, ' ').collect();
            if parts.len() == 2 {
                Param {
                    name: parts[0].to_string(),
                    type_annotation: Some(parts[1].to_string()),
                    default: None,
                    is_variadic: parts[1].starts_with("..."),
                    is_keyword: false,
                }
            } else {
                Param {
                    name: p.to_string(),
                    type_annotation: None,
                    default: None,
                    is_variadic: false,
                    is_keyword: false,
                }
            }
        })
        .collect()
}

/// Truncate a docstring to ~200 characters, taking only the first paragraph.
fn truncate_docstring(doc: &str) -> String {
    let first_para = doc.split("\n\n").next().unwrap_or(doc);
    let cleaned: String = first_para
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<&str>>()
        .join(" ");

    if cleaned.len() <= 200 {
        cleaned
    } else {
        format!("{}...", &cleaned[..197])
    }
}

/// Generate an example usage string for a Go function.
fn generate_go_function_example(
    module_path: &str,
    func_name: &str,
    params: &[Param],
    return_type: Option<&str>,
) -> Option<String> {
    let args: Vec<String> = params
        .iter()
        .map(|p| go_example_value(p.type_annotation.as_deref()))
        .collect();
    let call = format!("{}.{}({})", module_path, func_name, args.join(", "));

    match return_type {
        Some(rt) if !rt.is_empty() => Some(format!("result := {}", call)),
        _ => Some(call),
    }
}

/// Generate an example usage string for a Go type.
fn generate_go_type_example(module_path: &str, type_name: &str, kind: ApiKind) -> Option<String> {
    match kind {
        ApiKind::Struct => Some(format!("obj := {}.{}{{}}", module_path, type_name)),
        ApiKind::Interface => Some(format!("var iface {}.{}", module_path, type_name)),
        _ => Some(format!("{}.{}", module_path, type_name)),
    }
}

/// Generate an example usage string for a Go method.
fn generate_go_method_example(
    type_name: &str,
    method_name: &str,
    params: &[Param],
    return_type: Option<&str>,
) -> Option<String> {
    let receiver_var = type_name
        .chars()
        .next()
        .map(|c| c.to_lowercase().to_string())
        .unwrap_or_else(|| "v".to_string());

    let args: Vec<String> = params
        .iter()
        .map(|p| go_example_value(p.type_annotation.as_deref()))
        .collect();
    let call = format!("{}.{}({})", receiver_var, method_name, args.join(", "));

    match return_type {
        Some(rt) if !rt.is_empty() => Some(format!("result := {}", call)),
        _ => Some(call),
    }
}

/// Generate a Go example value for a type annotation.
fn go_example_value(type_annotation: Option<&str>) -> String {
    match type_annotation {
        Some("string") => "\"example\"".to_string(),
        Some("int") | Some("int8") | Some("int16") | Some("int32") | Some("int64") => {
            "42".to_string()
        }
        Some("uint") | Some("uint8") | Some("uint16") | Some("uint32") | Some("uint64") => {
            "42".to_string()
        }
        Some("float32") | Some("float64") => "3.14".to_string(),
        Some("bool") => "true".to_string(),
        Some("byte") => "0".to_string(),
        Some("rune") => "'a'".to_string(),
        Some("error") => "nil".to_string(),
        Some(t) if t.starts_with("[]") => "nil".to_string(),
        Some(t) if t.starts_with("map[") => "nil".to_string(),
        Some(t) if t.starts_with("*") => "nil".to_string(),
        Some(t) if t.starts_with("...") => {
            // Variadic: use an example of the element type
            let elem = &t[3..];
            go_example_value(Some(elem))
        }
        _ => "nil".to_string(),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ---- is_exported ----

    #[test]
    fn test_is_exported_uppercase() {
        assert!(is_exported("Println"));
        assert!(is_exported("Handler"));
        assert!(is_exported("New"));
        assert!(is_exported("HTTPClient"));
    }

    #[test]
    fn test_is_exported_lowercase() {
        assert!(!is_exported("println"));
        assert!(!is_exported("handler"));
        assert!(!is_exported("new"));
        assert!(!is_exported("_private"));
    }

    #[test]
    fn test_is_exported_empty() {
        assert!(!is_exported(""));
    }

    // ---- compute_go_package_path ----

    #[test]
    fn test_compute_package_path_root() {
        let root = Path::new("/project/pkg");
        let file = Path::new("/project/pkg/main.go");
        assert_eq!(compute_go_package_path(file, root, "mypkg"), "mypkg");
    }

    #[test]
    fn test_compute_package_path_subdir() {
        let root = Path::new("/project/pkg");
        let file = Path::new("/project/pkg/sub/helper.go");
        assert_eq!(compute_go_package_path(file, root, "mypkg"), "mypkg/sub");
    }

    // ---- determine_go_type_kind ----

    #[test]
    fn test_determine_type_kind_struct() {
        let class = ClassInfo {
            name: "Server".to_string(),
            line_number: 3,
            methods: vec![],
            fields: vec![],
            bases: vec![],
            decorators: vec![],
            docstring: None,
        };
        let source = "package main\n\ntype Server struct {\n\tAddr string\n\tPort int\n}\n";
        assert_eq!(determine_go_type_kind(&class, source), ApiKind::Struct);
    }

    #[test]
    fn test_determine_type_kind_interface() {
        let class = ClassInfo {
            name: "Handler".to_string(),
            line_number: 3,
            methods: vec![],
            fields: vec![],
            bases: vec![],
            decorators: vec![],
            docstring: None,
        };
        let source = "package main\n\ntype Handler interface {\n\tServeHTTP(w ResponseWriter, r *Request)\n}\n";
        assert_eq!(determine_go_type_kind(&class, source), ApiKind::Interface);
    }

    // ---- convert_go_params ----

    #[test]
    fn test_convert_params_typed() {
        let params = vec!["name string".to_string(), "count int".to_string()];
        let converted = convert_go_params(&params);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].name, "name");
        assert_eq!(converted[0].type_annotation, Some("string".to_string()));
        assert_eq!(converted[1].name, "count");
        assert_eq!(converted[1].type_annotation, Some("int".to_string()));
    }

    #[test]
    fn test_convert_params_variadic() {
        let params = vec!["args ...string".to_string()];
        let converted = convert_go_params(&params);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].name, "args");
        assert!(converted[0].is_variadic);
    }

    #[test]
    fn test_convert_params_empty() {
        let params: Vec<String> = vec![];
        let converted = convert_go_params(&params);
        assert!(converted.is_empty());
    }

    // ---- example generation ----

    #[test]
    fn test_generate_function_example() {
        let params = vec![Param {
            name: "s".to_string(),
            type_annotation: Some("string".to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        }];
        let example = generate_go_function_example("fmt", "Println", &params, None);
        assert!(example.is_some());
        assert!(example.unwrap().contains("fmt.Println(\"example\")"));
    }

    #[test]
    fn test_generate_function_example_with_return() {
        let params = vec![Param {
            name: "s".to_string(),
            type_annotation: Some("string".to_string()),
            default: None,
            is_variadic: false,
            is_keyword: false,
        }];
        let example = generate_go_function_example("strconv", "Atoi", &params, Some("int"));
        assert!(example.is_some());
        let ex = example.unwrap();
        assert!(ex.starts_with("result := "));
    }

    #[test]
    fn test_generate_type_example_struct() {
        let example = generate_go_type_example("http", "Server", ApiKind::Struct);
        assert!(example.is_some());
        assert_eq!(example.unwrap(), "obj := http.Server{}");
    }

    #[test]
    fn test_generate_type_example_interface() {
        let example = generate_go_type_example("io", "Reader", ApiKind::Interface);
        assert!(example.is_some());
        assert_eq!(example.unwrap(), "var iface io.Reader");
    }

    // ---- go_example_value ----

    #[test]
    fn test_go_example_value_types() {
        assert_eq!(go_example_value(Some("string")), "\"example\"");
        assert_eq!(go_example_value(Some("int")), "42");
        assert_eq!(go_example_value(Some("bool")), "true");
        assert_eq!(go_example_value(Some("float64")), "3.14");
        assert_eq!(go_example_value(Some("error")), "nil");
        assert_eq!(go_example_value(Some("[]byte")), "nil");
        assert_eq!(go_example_value(Some("*Foo")), "nil");
        assert_eq!(go_example_value(None), "nil");
    }

    // ---- truncate_docstring ----

    #[test]
    fn test_truncate_docstring_short() {
        assert_eq!(truncate_docstring("Short doc."), "Short doc.");
    }

    #[test]
    fn test_truncate_docstring_multiline() {
        let doc = "First paragraph line 1.\nFirst paragraph line 2.\n\nSecond paragraph.";
        let result = truncate_docstring(doc);
        assert!(result.contains("First paragraph line 1."));
        assert!(result.contains("First paragraph line 2."));
        assert!(!result.contains("Second paragraph"));
    }

    // ---- Integration: extract from Go source ----

    #[test]
    fn test_extract_go_api_surface_minimal() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("math.go");
        fs::write(
            &go_file,
            r#"package mathpkg

// Add returns the sum of two integers.
func Add(a int, b int) int {
	return a + b
}

// multiply is unexported -- should be excluded by default.
func multiply(a, b int) int {
	return a * b
}

// Pi is a constant.
const Pi = 3.14159

// internal is an unexported constant.
const internal = 42

// Calculator performs arithmetic.
type Calculator struct {
	Precision int
}

// Compute runs a calculation.
func (c *Calculator) Compute(expr string) (float64, error) {
	return 0.0, nil
}

// reset is unexported.
func (c *Calculator) reset() {
	c.Precision = 0
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mathpkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        assert_eq!(surface.language, "go");
        assert_eq!(surface.package, "mathpkg");

        // Should have: Add, Pi, Calculator, Calculator.Compute
        // Should NOT have: multiply, internal, Calculator.reset
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        assert!(
            names.contains(&"mathpkg.Add"),
            "Should contain exported function Add, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("multiply")),
            "Should NOT contain unexported function multiply, got: {:?}",
            names
        );
        assert!(
            names.contains(&"mathpkg.Calculator"),
            "Should contain exported struct Calculator, got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("Compute")),
            "Should contain exported method Compute, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("reset")),
            "Should NOT contain unexported method reset, got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_go_api_surface_include_private() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("pkg.go");
        fs::write(
            &go_file,
            r#"package mypkg

func Public() {}
func private() {}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, true, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"mypkg.Public"));
        assert!(names.contains(&"mypkg.private"));
    }

    #[test]
    fn test_extract_go_api_surface_with_limit() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("pkg.go");
        fs::write(
            &go_file,
            r#"package mypkg

func A() {}
func B() {}
func C() {}
func D() {}
func E() {}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, Some(2)).unwrap();
        assert_eq!(surface.apis.len(), 2);
        assert_eq!(surface.total, 2);
    }

    #[test]
    fn test_extract_go_api_surface_interface() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("iface.go");
        fs::write(
            &go_file,
            r#"package mypkg

// Reader is a test interface.
type Reader interface {
	Read(p []byte) (int, error)
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let reader_api = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.Reader");
        assert!(reader_api.is_some(), "Should find Reader interface");
        assert_eq!(reader_api.unwrap().kind, ApiKind::Interface);
    }

    // ---- Bug 1: Docstrings ----

    #[test]
    fn test_extract_go_function_docstring() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("doc.go");
        fs::write(
            &go_file,
            r#"package mypkg

// Transfer moves funds between accounts
// and returns the new balance.
func Transfer(from string, to string, amount float64) (float64, error) {
	return 0, nil
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let transfer = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.Transfer")
            .expect("Should find Transfer");

        assert!(
            transfer.docstring.is_some(),
            "Transfer should have a docstring extracted from // comments"
        );
        let doc = transfer.docstring.as_ref().unwrap();
        assert!(
            doc.contains("Transfer moves funds"),
            "Docstring should contain first line, got: {}",
            doc
        );
        assert!(
            doc.contains("new balance"),
            "Docstring should contain second line, got: {}",
            doc
        );
    }

    #[test]
    fn test_extract_go_struct_docstring() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("doc.go");
        fs::write(
            &go_file,
            r#"package mypkg

// Account represents a bank account.
type Account struct {
	ID      string
	Balance float64
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let account = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.Account")
            .expect("Should find Account");

        assert!(
            account.docstring.is_some(),
            "Account struct should have a docstring extracted from // comments"
        );
        let doc = account.docstring.as_ref().unwrap();
        assert!(
            doc.contains("bank account"),
            "Docstring should contain content, got: {}",
            doc
        );
    }

    #[test]
    fn test_extract_go_method_docstring() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("doc.go");
        fs::write(
            &go_file,
            r#"package mypkg

type Server struct{}

// Start begins listening on the given address.
func (s *Server) Start(addr string) error {
	return nil
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let start = surface
            .apis
            .iter()
            .find(|a| a.qualified_name.contains("Start"))
            .expect("Should find Start method");

        assert!(
            start.docstring.is_some(),
            "Start method should have a docstring, got None"
        );
        let doc = start.docstring.as_ref().unwrap();
        assert!(
            doc.contains("begins listening"),
            "Docstring should contain content, got: {}",
            doc
        );
    }

    #[test]
    fn test_extract_go_no_docstring_when_no_comment() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("nodoc.go");
        fs::write(
            &go_file,
            r#"package mypkg

func NoDoc() {}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let nodoc = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.NoDoc")
            .expect("Should find NoDoc");

        assert!(nodoc.docstring.is_none(), "NoDoc should have no docstring");
    }

    // ---- Bug 2: Grouped parameters ----

    #[test]
    fn test_extract_go_grouped_params() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("params.go");
        fs::write(
            &go_file,
            r#"package mypkg

func Transfer(fromID, toID string, amount float64) (float64, error) {
	return 0, nil
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let transfer = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.Transfer")
            .expect("Should find Transfer");

        let sig = transfer.signature.as_ref().expect("Should have signature");
        let param_names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();

        assert_eq!(
            param_names,
            vec!["fromID", "toID", "amount"],
            "All three params should be extracted (including grouped toID)"
        );
    }

    #[test]
    fn test_extract_go_grouped_params_three_names() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("params.go");
        fs::write(
            &go_file,
            r#"package mypkg

func ThreeWay(a, b, c int) int {
	return a + b + c
}
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let func = surface
            .apis
            .iter()
            .find(|a| a.qualified_name == "mypkg.ThreeWay")
            .expect("Should find ThreeWay");

        let sig = func.signature.as_ref().expect("Should have signature");
        let param_names: Vec<&str> = sig.params.iter().map(|p| p.name.as_str()).collect();

        assert_eq!(
            param_names,
            vec!["a", "b", "c"],
            "All three grouped params should be extracted"
        );
    }

    // ---- Bug 3: var declarations ----

    #[test]
    fn test_extract_go_var_declarations() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("errors.go");
        fs::write(
            &go_file,
            r#"package mypkg

import "errors"

var ErrInsufficientFunds = errors.New("insufficient funds")

var ErrNotFound = errors.New("not found")

var internalErr = errors.New("internal")
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"mypkg.ErrInsufficientFunds"),
            "Should extract exported var ErrInsufficientFunds, got: {:?}",
            names
        );
        assert!(
            names.contains(&"mypkg.ErrNotFound"),
            "Should extract exported var ErrNotFound, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("internalErr")),
            "Should NOT extract unexported var internalErr, got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_go_var_with_type() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("vars.go");
        fs::write(
            &go_file,
            r#"package mypkg

var DefaultTimeout int = 30

var MaxRetries = 5
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"mypkg.DefaultTimeout"),
            "Should extract exported var DefaultTimeout, got: {:?}",
            names
        );
        assert!(
            names.contains(&"mypkg.MaxRetries"),
            "Should extract exported var MaxRetries, got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_go_var_grouped_block() {
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("vars.go");
        fs::write(
            &go_file,
            r#"package mypkg

import "errors"

var (
	ErrTimeout  = errors.New("timeout")
	ErrCanceled = errors.New("canceled")
	internal    = errors.New("internal")
)
"#,
        )
        .unwrap();

        let resolved = ResolvedPackage {
            root_dir: dir.path().to_path_buf(),
            package_name: "mypkg".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.contains(&"mypkg.ErrTimeout"),
            "Should extract exported var ErrTimeout from grouped block, got: {:?}",
            names
        );
        assert!(
            names.contains(&"mypkg.ErrCanceled"),
            "Should extract exported var ErrCanceled from grouped block, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("internal")),
            "Should NOT extract unexported var internal, got: {:?}",
            names
        );
    }

    #[test]
    fn test_extract_go_api_surface_single_file() {
        // When root_dir is a single .go file (not a directory), extraction
        // should process only that file and not fail with a read_dir error.
        let dir = TempDir::new().unwrap();
        let main_go = dir.path().join("main.go");
        fs::write(
            &main_go,
            r#"package main

// Run starts the application.
func Run() error {
	return nil
}

func helper() {}
"#,
        )
        .unwrap();

        // Also create a sibling that must NOT be included
        let sibling = dir.path().join("extra.go");
        fs::write(
            &sibling,
            r#"package main

func Extra() {}
"#,
        )
        .unwrap();

        // Pass the single file as root_dir (simulating what resolve_target
        // returns for a single-file target)
        let resolved = ResolvedPackage {
            root_dir: main_go.clone(),
            package_name: "main".to_string(),
            is_pure_source: true,
            public_names: None,
        };

        let surface = extract_go_api_surface(&resolved, false, None).unwrap();
        let names: Vec<&str> = surface
            .apis
            .iter()
            .map(|a| a.qualified_name.as_str())
            .collect();

        assert!(
            names.iter().any(|n| n.contains("Run")),
            "Should extract Run from the single file, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("Extra")),
            "Should NOT extract Extra from sibling file, got: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n.contains("helper")),
            "Should NOT extract unexported helper, got: {:?}",
            names
        );
    }

    #[test]
    fn test_find_go_files_single_file_path() {
        // find_go_files should handle a single .go file as input,
        // returning just that file instead of failing on read_dir.
        let dir = TempDir::new().unwrap();
        let go_file = dir.path().join("server.go");
        fs::write(&go_file, "package main\n").unwrap();

        // Also create a sibling
        fs::write(dir.path().join("client.go"), "package main\n").unwrap();

        let files = find_go_files(&go_file);
        assert_eq!(files.len(), 1, "Should return exactly 1 file");
        assert_eq!(files[0], go_file, "Should return the single file passed in");
    }

    #[test]
    fn test_find_go_files_excludes_tests() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.go"), "package main\n").unwrap();
        fs::write(dir.path().join("main_test.go"), "package main\n").unwrap();
        fs::write(dir.path().join("helper.go"), "package main\n").unwrap();

        let files = find_go_files(dir.path());
        let names: Vec<String> = files
            .iter()
            .filter_map(|f| {
                f.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .collect();
        assert!(names.contains(&"main.go".to_string()));
        assert!(names.contains(&"helper.go".to_string()));
        assert!(
            !names.contains(&"main_test.go".to_string()),
            "Should exclude _test.go files"
        );
    }
}
