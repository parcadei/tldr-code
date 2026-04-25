//! Coupling command - Cross-module coupling analysis
//!
//! Analyzes coupling between two source modules by tracking cross-module
//! function calls. Computes a coupling score (0.0-1.0) and provides a verdict.
//!
//! Supports all languages with tree-sitter grammars: Python, Go, Rust,
//! TypeScript, JavaScript, Java, C, C++, Ruby, C#, PHP, Scala, Elixir,
//! Lua, Luau, and OCaml.
//!
//! # Example Usage
//!
//! ```bash
//! tldr coupling src/auth.py src/user.py
//! tldr coupling src/gin.go src/context.go --format text
//! tldr coupling src/lib.rs src/utils.rs --timeout 30
//! ```
//!
//! # TIGER Mitigations
//!
//! - **E02**: `--timeout` flag with default 30 seconds
//! - **T02**: All path validation through `validation.rs`

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;
use colored::Colorize;
use tree_sitter::{Node, Parser};

use tldr_core::analysis::clones::is_test_file;
use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
use tldr_core::ast::parser::ParserPool;
use tldr_core::quality::coupling::{
    analyze_coupling as core_analyze_coupling, compute_martin_metrics_from_deps,
    CouplingReport as CoreCouplingReport, CouplingVerdict as CoreVerdict, MartinMetricsReport,
    MartinOptions,
};
use tldr_core::types::Language as TldrLanguage;

use super::error::{PatternsError, PatternsResult};
use super::types::{CouplingReport, CouplingVerdict, CrossCall, CrossCalls};
use super::validation::{read_file_safe, validate_file_path, validate_file_path_in_project};
use crate::output::{common_path_prefix, strip_prefix_display, OutputFormat};

// =============================================================================
// CLI Arguments
// =============================================================================

/// Analyze coupling between source modules.
///
/// Two modes:
/// - **Pair mode** (2 args): `tldr coupling file_a file_b` -- compare two files
/// - **Project-wide mode** (1 arg): `tldr coupling directory/` -- scan all pairs
///
/// Measures cross-module function calls and computes a coupling score.
/// A lower score indicates looser coupling (good), while a higher score
/// indicates tighter coupling (may need refactoring).
///
/// Supports: Python, Go, Rust, TypeScript, JavaScript, Java, C, C++,
/// Ruby, C#, PHP, Scala, Elixir, Lua, Luau, OCaml.
#[derive(Debug, Clone, Args)]
pub struct CouplingArgs {
    /// First source module (pair mode) or directory to scan (project-wide mode)
    pub path_a: PathBuf,

    /// Second source module (pair mode). Omit for project-wide scan.
    pub path_b: Option<PathBuf>,

    /// Timeout in seconds (TIGER E02 mitigation)
    #[arg(long, default_value = "30")]
    pub timeout: u64,

    /// Project root for path validation (optional)
    #[arg(long)]
    pub project_root: Option<PathBuf>,

    /// Maximum number of pairs to show in project-wide mode (default: 20)
    #[arg(long, short = 'n', default_value = "20")]
    pub max_pairs: usize,

    /// Limit output to top N modules ranked by instability (project-wide mode only). 0 = show all.
    #[arg(long, default_value = "0")]
    pub top: usize,

    /// Only show modules involved in dependency cycles (project-wide mode only)
    #[arg(long)]
    pub cycles_only: bool,

    /// Include test files in analysis (excluded by default)
    #[arg(long)]
    pub include_tests: bool,

    /// Language filter (auto-detected if omitted)
    #[arg(long, short = 'l')]
    pub lang: Option<TldrLanguage>,
}

// =============================================================================
// Module Information
// =============================================================================

/// Information extracted from a source module for coupling analysis.
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Path to the module
    pub path: PathBuf,
    /// Names defined at module level (functions, classes)
    pub defined_names: HashSet<String>,
    /// Imports: alias/name -> source module
    pub imports: HashMap<String, String>,
    /// Call sites: (caller_func, callee_name, line)
    pub calls: Vec<(String, String, u32)>,
    /// Total function count for normalization
    pub function_count: u32,
}

impl ModuleInfo {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            defined_names: HashSet::new(),
            imports: HashMap::new(),
            calls: Vec::new(),
            function_count: 0,
        }
    }
}

// =============================================================================
// Language Configuration
// =============================================================================

/// Language-specific AST node kind configuration for coupling analysis.
struct LangConfig {
    /// Node kinds that represent function/method definitions
    function_kinds: &'static [&'static str],
    /// Node kinds that represent class/type definitions
    class_kinds: &'static [&'static str],
    /// Node kinds that represent import statements
    import_kinds: &'static [&'static str],
    /// Node kinds that represent function/method calls
    call_kinds: &'static [&'static str],
    /// Field name to get the function name (e.g. "name")
    func_name_field: &'static str,
    /// Whether to look for the name child by field or first identifier
    use_name_field: bool,
    /// Whether to recurse into class bodies for method definitions
    recurse_into_classes: bool,
}

fn lang_config_for(lang: TldrLanguage) -> LangConfig {
    match lang {
        TldrLanguage::Python => LangConfig {
            function_kinds: &["function_definition", "async_function_definition"],
            class_kinds: &["class_definition"],
            import_kinds: &["import_statement", "import_from_statement"],
            call_kinds: &["call"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: false,
        },
        TldrLanguage::Go => LangConfig {
            function_kinds: &["function_declaration", "method_declaration"],
            class_kinds: &["type_declaration"],
            import_kinds: &["import_declaration"],
            call_kinds: &["call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: false,
        },
        TldrLanguage::Rust => LangConfig {
            function_kinds: &["function_item"],
            class_kinds: &["struct_item", "enum_item", "trait_item", "impl_item"],
            import_kinds: &["use_declaration"],
            call_kinds: &["call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::TypeScript | TldrLanguage::JavaScript => LangConfig {
            function_kinds: &[
                "function_declaration",
                "method_definition",
                "arrow_function",
            ],
            class_kinds: &["class_declaration"],
            import_kinds: &["import_statement"],
            call_kinds: &["call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: false,
        },
        TldrLanguage::Java => LangConfig {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &["class_declaration", "interface_declaration"],
            import_kinds: &["import_declaration"],
            call_kinds: &["method_invocation"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::C => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["struct_specifier", "enum_specifier"],
            import_kinds: &["preproc_include"],
            call_kinds: &["call_expression"],
            func_name_field: "declarator",
            use_name_field: true,
            recurse_into_classes: false,
        },
        TldrLanguage::Cpp => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["class_specifier", "struct_specifier", "enum_specifier"],
            import_kinds: &["preproc_include"],
            call_kinds: &["call_expression"],
            func_name_field: "declarator",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::Ruby => LangConfig {
            function_kinds: &["method", "singleton_method"],
            class_kinds: &["class", "module"],
            import_kinds: &[], // Ruby uses require/require_relative as function calls
            call_kinds: &["call", "command"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::CSharp => LangConfig {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &[
                "class_declaration",
                "interface_declaration",
                "struct_declaration",
            ],
            import_kinds: &["using_directive"],
            call_kinds: &["invocation_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::Php => LangConfig {
            function_kinds: &["function_definition", "method_declaration"],
            class_kinds: &["class_declaration", "interface_declaration"],
            import_kinds: &["namespace_use_declaration"],
            call_kinds: &["function_call_expression", "member_call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::Scala => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["class_definition", "object_definition", "trait_definition"],
            import_kinds: &["import_declaration"],
            call_kinds: &["call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: true,
        },
        TldrLanguage::Elixir => LangConfig {
            function_kinds: &["call"], // def/defp are calls in Elixir AST
            class_kinds: &[],
            import_kinds: &[], // import/use/require are calls in Elixir AST
            call_kinds: &["call"],
            func_name_field: "",
            use_name_field: false,
            recurse_into_classes: false,
        },
        TldrLanguage::Lua | TldrLanguage::Luau => LangConfig {
            function_kinds: &[
                "function_declaration",
                "local_function_declaration_statement",
            ],
            class_kinds: &[],
            import_kinds: &[], // Lua uses require() as a function call
            call_kinds: &["function_call"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: false,
        },
        TldrLanguage::Ocaml => LangConfig {
            function_kinds: &["let_binding", "value_definition"],
            class_kinds: &["type_definition", "module_definition"],
            import_kinds: &["open_statement"],
            call_kinds: &["application"],
            func_name_field: "",
            use_name_field: false,
            recurse_into_classes: false,
        },
        // Kotlin and Swift are not yet supported by the parser pool
        _ => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["class_definition"],
            import_kinds: &["import_statement"],
            call_kinds: &["call_expression"],
            func_name_field: "name",
            use_name_field: true,
            recurse_into_classes: false,
        },
    }
}

/// Detect the language from a file path, returning a PatternsError if unsupported.
fn detect_language(path: &Path) -> PatternsResult<TldrLanguage> {
    TldrLanguage::from_path(path).ok_or_else(|| {
        PatternsError::parse_error(
            path,
            format!(
                "Unsupported file extension: {}",
                path.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("(none)")
            ),
        )
    })
}

// =============================================================================
// Module Extraction
// =============================================================================

/// Extract module information from source code.
///
/// Detects language from file extension, parses the source using tree-sitter,
/// and extracts:
/// - Top-level function and class definitions
/// - Import statements (language-specific)
/// - Function call sites within function bodies
pub fn extract_module_info(path: &PathBuf, source: &str) -> PatternsResult<ModuleInfo> {
    let lang = detect_language(path)?;

    let ts_lang = ParserPool::get_ts_language(lang).ok_or_else(|| {
        PatternsError::parse_error(path, format!("No tree-sitter grammar for {:?}", lang))
    })?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| PatternsError::parse_error(path, format!("Failed to set language: {}", e)))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| PatternsError::parse_error(path, "Failed to parse source"))?;

    let root = tree.root_node();
    let config = lang_config_for(lang);
    let mut info = ModuleInfo::new(path.clone());

    // Extract top-level definitions and imports
    extract_top_level_generic(&root, source, &mut info, &config, lang)?;

    // Post-processing: for package-based languages (Go, Java, C#, PHP, etc.),
    // when we see calls like `pkg.Func()`, we need to also register `Func` as
    // an import so that cross-call detection works. This handles languages where
    // you import a package/namespace and call functions through it (not by name).
    if matches!(
        lang,
        TldrLanguage::Go
            | TldrLanguage::Java
            | TldrLanguage::CSharp
            | TldrLanguage::Php
            | TldrLanguage::Scala
    ) {
        enrich_imports_from_qualified_calls(&mut info);
    }

    Ok(info)
}

/// For package-based languages, add function names from qualified calls to the imports map.
///
/// When source has `import "pkg"` and calls `pkg.Func()`, the callee is extracted as "Func"
/// but the import key is "pkg". This function adds "Func" -> "pkg" to the imports so that
/// `find_cross_calls` can detect it.
fn enrich_imports_from_qualified_calls(info: &mut ModuleInfo) {
    // Collect new import entries to avoid borrowing conflicts
    let mut new_imports: Vec<(String, String)> = Vec::new();

    for (_caller, callee, _line) in &info.calls {
        // If the callee is already in imports, no need to add
        if info.imports.contains_key(callee) {
            continue;
        }
        // Add it as an import reference (the callee name maps to itself as module)
        // This enables cross-call detection: if the other module defines this function,
        // it will be detected as a cross-call.
        new_imports.push((callee.clone(), callee.clone()));
    }

    for (name, module) in new_imports {
        info.imports.entry(name).or_insert(module);
    }
}

/// Extract top-level definitions and imports from the AST root (generic, multi-language).
fn extract_top_level_generic(
    root: &Node,
    source: &str,
    info: &mut ModuleInfo,
    config: &LangConfig,
    lang: TldrLanguage,
) -> PatternsResult<()> {
    extract_definitions_recursive(root, source, info, config, lang, 0);
    Ok(())
}

/// Recursively extract definitions, imports, and calls from the AST.
///
/// `depth` controls recursion into class/module bodies.
fn extract_definitions_recursive(
    node: &Node,
    source: &str,
    info: &mut ModuleInfo,
    config: &LangConfig,
    lang: TldrLanguage,
    depth: u32,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Check for function definitions
        if config.function_kinds.contains(&kind) {
            // Special case: Elixir's def/defp are call nodes
            if lang == TldrLanguage::Elixir && kind == "call" {
                if let Some(name) = extract_elixir_def_name(&child, source) {
                    info.defined_names.insert(name.clone());
                    info.function_count += 1;
                    extract_calls_generic(&child, source, &name, &mut info.calls, config, lang);
                }
                continue;
            }

            if let Some(name) = get_name_generic(&child, source, config, lang) {
                info.defined_names.insert(name.clone());
                info.function_count += 1;
                extract_calls_generic(&child, source, &name, &mut info.calls, config, lang);
            }
        }
        // Check for class/type definitions
        else if config.class_kinds.contains(&kind) {
            if let Some(name) = get_name_generic(&child, source, config, lang) {
                info.defined_names.insert(name);
            }
            // Recurse into class bodies to find methods
            if config.recurse_into_classes && depth < 3 {
                extract_definitions_recursive(&child, source, info, config, lang, depth + 1);
            }
        }
        // Check for import statements
        else if config.import_kinds.contains(&kind) {
            extract_imports_generic(&child, source, &mut info.imports, lang);
        }
        // Ruby: detect require/require_relative calls at top level
        else if lang == TldrLanguage::Ruby && (kind == "call" || kind == "command") {
            extract_ruby_require(&child, source, &mut info.imports);
        }
        // For languages where module body is nested (Java class_body, C# namespace, etc.)
        else if is_body_container(kind, lang) {
            extract_definitions_recursive(&child, source, info, config, lang, depth + 1);
        }
    }
}

/// Check if a node kind is a body container that should be recursed into.
fn is_body_container(kind: &str, lang: TldrLanguage) -> bool {
    match lang {
        TldrLanguage::Java => matches!(kind, "class_body" | "program"),
        TldrLanguage::CSharp => matches!(
            kind,
            "namespace_declaration"
                | "file_scoped_namespace_declaration"
                | "declaration_list"
                | "class_body"
        ),
        TldrLanguage::Php => matches!(kind, "declaration_list" | "class_body" | "program"),
        TldrLanguage::Scala => matches!(kind, "template_body"),
        TldrLanguage::Cpp => matches!(kind, "declaration_list"),
        TldrLanguage::Ruby => matches!(kind, "body_statement" | "program"),
        _ => false,
    }
}

/// Get the name of a function/class/type definition node (generic).
fn get_name_generic(
    node: &Node,
    source: &str,
    config: &LangConfig,
    _lang: TldrLanguage,
) -> Option<String> {
    // Try field-based lookup first (most languages)
    if config.use_name_field && !config.func_name_field.is_empty() {
        if let Some(name_node) = node.child_by_field_name(config.func_name_field) {
            // For C/C++, the declarator may be a function_declarator wrapping an identifier
            return Some(extract_leaf_identifier(&name_node, source));
        }
    }

    // Fallback: find the first identifier child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "name" {
            return Some(node_text(&child, source));
        }
    }

    None
}

/// Extract the leaf identifier from a node that might be a complex declarator.
///
/// Handles C/C++ patterns like `function_declarator -> identifier`.
fn extract_leaf_identifier(node: &Node, source: &str) -> String {
    if node.kind() == "identifier" || node.kind() == "name" || node.child_count() == 0 {
        return node_text(node, source);
    }

    // Recurse to find the first identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "name" {
            return node_text(&child, source);
        }
        // Recurse into function_declarator, pointer_declarator, etc.
        let result = extract_leaf_identifier(&child, source);
        if !result.is_empty() {
            return result;
        }
    }

    node_text(node, source)
}

/// Extract Elixir def/defp function name from a call node.
fn extract_elixir_def_name(node: &Node, source: &str) -> Option<String> {
    // In Elixir AST, `def foo(args)` is a call where the target is "def"
    // and the first argument contains the function name
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let text = node_text(&child, source);
        if text == "def" || text == "defp" {
            // Next sibling should have the function name
            if let Some(args) = child.next_sibling() {
                return get_first_identifier(&args, source);
            }
        }
    }
    None
}

/// Get the first identifier in a subtree.
fn get_first_identifier(node: &Node, source: &str) -> Option<String> {
    if node.kind() == "identifier" || node.kind() == "atom" {
        return Some(node_text(node, source));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(id) = get_first_identifier(&child, source) {
            return Some(id);
        }
    }
    None
}

// =============================================================================
// Import Extraction (Generic)
// =============================================================================

/// Extract imports from an import node (generic, multi-language).
fn extract_imports_generic(
    node: &Node,
    source: &str,
    imports: &mut HashMap<String, String>,
    lang: TldrLanguage,
) {
    match lang {
        TldrLanguage::Python => extract_python_imports(node, source, imports),
        TldrLanguage::Go => extract_go_imports(node, source, imports),
        TldrLanguage::Rust => extract_rust_imports(node, source, imports),
        TldrLanguage::TypeScript | TldrLanguage::JavaScript => {
            extract_ts_imports(node, source, imports)
        }
        TldrLanguage::Java => extract_java_imports(node, source, imports),
        TldrLanguage::C | TldrLanguage::Cpp => extract_c_imports(node, source, imports),
        TldrLanguage::CSharp => extract_csharp_imports(node, source, imports),
        TldrLanguage::Php => extract_php_imports(node, source, imports),
        TldrLanguage::Scala => extract_scala_imports(node, source, imports),
        TldrLanguage::Ocaml => extract_ocaml_imports(node, source, imports),
        // Languages using function-call-based imports (Ruby, Lua, Elixir)
        // are handled separately in the caller
        _ => extract_fallback_imports(node, source, imports),
    }
}

/// Python: import X, from X import Y
fn extract_python_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let kind = node.kind();
    if kind == "import_statement" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dotted_name" {
                let module_name = node_text(&child, source);
                imports.insert(module_name.clone(), module_name);
            } else if child.kind() == "aliased_import" {
                if let (Some(name), Some(alias)) = extract_aliased_import(&child, source) {
                    imports.insert(alias, name);
                }
            }
        }
    } else if kind == "import_from_statement" {
        let mut module_name = String::new();
        let mut found_import_keyword = false;

        // First pass: find the module name
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import" {
                found_import_keyword = true;
                continue;
            }
            if !found_import_keyword {
                match child.kind() {
                    "dotted_name" | "relative_import" | "import_prefix" => {
                        module_name = node_text(&child, source);
                    }
                    _ => {}
                }
            }
        }

        // Second pass: find all imported names
        let mut cursor2 = node.walk();
        found_import_keyword = false;
        for child in node.children(&mut cursor2) {
            if child.kind() == "import" {
                found_import_keyword = true;
                continue;
            }
            if !found_import_keyword {
                continue;
            }
            match child.kind() {
                "dotted_name" | "identifier" => {
                    let name = node_text(&child, source);
                    imports.insert(name, module_name.clone());
                }
                "aliased_import" => {
                    if let (Some(name), Some(alias)) = extract_aliased_import(&child, source) {
                        imports.insert(alias, module_name.clone());
                        imports.insert(name, module_name.clone());
                    }
                }
                "wildcard_import" => {
                    imports.insert("*".to_string(), module_name.clone());
                }
                _ => {
                    extract_import_names_recursive(&child, source, &module_name, imports);
                }
            }
        }
    }
}

/// Recursively extract imported names from a node subtree (Python).
fn extract_import_names_recursive(
    node: &Node,
    source: &str,
    module_name: &str,
    imports: &mut HashMap<String, String>,
) {
    match node.kind() {
        "dotted_name" | "identifier" => {
            let name = node_text(node, source);
            imports.insert(name, module_name.to_string());
        }
        "aliased_import" => {
            if let (Some(name), Some(alias)) = extract_aliased_import(node, source) {
                imports.insert(alias, module_name.to_string());
                imports.insert(name, module_name.to_string());
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_import_names_recursive(&child, source, module_name, imports);
            }
        }
    }
}

/// Extract name and alias from an aliased_import node (Python).
fn extract_aliased_import(node: &Node, source: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut alias = None;
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" | "identifier" => {
                if name.is_none() {
                    name = Some(node_text(&child, source));
                } else {
                    alias = Some(node_text(&child, source));
                }
            }
            _ => {}
        }
    }

    (name, alias)
}

/// Go: import "pkg" or import ( "pkg1"; "pkg2" )
fn extract_go_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // import_declaration can contain import_spec or import_spec_list
    let mut stack = vec![*node];
    while let Some(n) = stack.pop() {
        if n.kind() == "import_spec" {
            // import_spec has optional name (alias) and path (string literal)
            let path_node = n.child_by_field_name("path");
            let name_node = n.child_by_field_name("name");

            if let Some(path) = path_node {
                let raw = node_text(&path, source);
                let module_path = raw.trim_matches('"').to_string();
                // Use the last component as the key (e.g., "fmt" from "fmt", "render" from "gin/render")
                let short_name = if let Some(alias) = name_node {
                    node_text(&alias, source)
                } else {
                    module_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&module_path)
                        .to_string()
                };
                imports.insert(short_name, module_path);
            }
        } else {
            let mut cursor = n.walk();
            for child in n.children(&mut cursor) {
                stack.push(child);
            }
        }
    }
}

/// Rust: use crate::module::item;
fn extract_rust_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // use_declaration contains a scoped_identifier or use_wildcard
    let text = node_text(node, source);
    // Strip "use " prefix and ";" suffix
    let trimmed = text.trim_start_matches("use ").trim_end_matches(';').trim();

    // Handle use a::b::{c, d} or use a::b::c
    if let Some(last) = trimmed.rsplit("::").next() {
        if last.starts_with('{') {
            // Grouped imports: use a::b::{c, d}
            let base = trimmed.rsplit_once("::").map(|x| x.0).unwrap_or("");
            let items = last.trim_matches(|c| c == '{' || c == '}');
            for item in items.split(',') {
                let item = item.trim();
                if !item.is_empty() {
                    imports.insert(item.to_string(), base.to_string());
                }
            }
        } else {
            imports.insert(last.to_string(), trimmed.to_string());
        }
    }
}

/// TypeScript/JavaScript: import { x } from 'y'; import * as x from 'y';
fn extract_ts_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // Find the source string (the module path)
    let mut module_path = String::new();
    let mut cursor = node.walk();

    // Find the source/from clause
    if let Some(src) = node.child_by_field_name("source") {
        let raw = node_text(&src, source);
        module_path = raw.trim_matches(|c| c == '\'' || c == '"').to_string();
    } else {
        // Fallback: look for string children
        for child in node.children(&mut cursor) {
            if child.kind() == "string" {
                let raw = node_text(&child, source);
                module_path = raw.trim_matches(|c| c == '\'' || c == '"').to_string();
            }
        }
    }

    // Extract imported names
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        match child.kind() {
            "import_clause" | "named_imports" | "import_specifier" => {
                collect_identifiers_recursive(&child, source, &module_path, imports);
            }
            "namespace_import" => {
                // import * as name
                if let Some(name) = child.child_by_field_name("name") {
                    imports.insert(node_text(&name, source), module_path.clone());
                } else {
                    // Fallback: get last identifier
                    let mut inner = child.walk();
                    let mut last_id = None;
                    for c in child.children(&mut inner) {
                        if c.kind() == "identifier" {
                            last_id = Some(node_text(&c, source));
                        }
                    }
                    if let Some(id) = last_id {
                        imports.insert(id, module_path.clone());
                    }
                }
            }
            "identifier" => {
                imports.insert(node_text(&child, source), module_path.clone());
            }
            _ => {}
        }
    }
}

/// Java: import com.example.Class;
fn extract_java_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // import_declaration has a scoped_identifier child
    let text = node_text(node, source);
    let trimmed = text
        .trim_start_matches("import ")
        .trim_start_matches("static ")
        .trim_end_matches(';')
        .trim();

    if let Some(last) = trimmed.rsplit('.').next() {
        imports.insert(last.to_string(), trimmed.to_string());
    }
}

/// C/C++: #include <header.h> or #include "header.h"
fn extract_c_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // preproc_include has a path child (system_lib_string or string_literal)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "system_lib_string" || kind == "string_literal" || kind == "string_content" {
            let raw = node_text(&child, source);
            let header = raw
                .trim_matches(|c| c == '<' || c == '>' || c == '"')
                .to_string();
            // Use the filename without path as key
            let short = header.rsplit('/').next().unwrap_or(&header).to_string();
            imports.insert(short, header);
        }
    }

    // Fallback: if the path child is wrapped
    if let Some(path) = node.child_by_field_name("path") {
        let raw = node_text(&path, source);
        let header = raw
            .trim_matches(|c| c == '<' || c == '>' || c == '"')
            .to_string();
        let short = header.rsplit('/').next().unwrap_or(&header).to_string();
        imports.insert(short, header);
    }
}

/// C#: using System.Collections;
fn extract_csharp_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let text = node_text(node, source);
    let trimmed = text
        .trim_start_matches("using ")
        .trim_start_matches("static ")
        .trim_end_matches(';')
        .trim();

    if let Some(last) = trimmed.rsplit('.').next() {
        imports.insert(last.to_string(), trimmed.to_string());
    }
    // Also add the full path
    if !trimmed.is_empty() {
        imports.insert(trimmed.to_string(), trimmed.to_string());
    }
}

/// PHP: use App\Utils\Helper;
fn extract_php_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let text = node_text(node, source);
    let trimmed = text.trim_start_matches("use ").trim_end_matches(';').trim();

    // Handle grouped: use App\{A, B}
    if trimmed.contains('{') {
        if let Some((base, group)) = trimmed.split_once('{') {
            let base = base.trim_end_matches('\\');
            let items = group.trim_end_matches('}');
            for item in items.split(',') {
                let item = item.trim();
                if !item.is_empty() {
                    imports.insert(item.to_string(), format!("{}\\{}", base, item));
                }
            }
        }
    } else if let Some(last) = trimmed.rsplit('\\').next() {
        imports.insert(last.to_string(), trimmed.to_string());
    }
}

/// Scala: import com.example.Class
fn extract_scala_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let text = node_text(node, source);
    let trimmed = text.trim_start_matches("import ").trim();

    if let Some(last) = trimmed.rsplit('.').next() {
        imports.insert(last.to_string(), trimmed.to_string());
    }
}

/// OCaml: open Module
fn extract_ocaml_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let text = node_text(node, source);
    let trimmed = text.trim_start_matches("open ").trim();
    if !trimmed.is_empty() {
        imports.insert(trimmed.to_string(), trimmed.to_string());
    }
}

/// Fallback import extraction: just record the text.
fn extract_fallback_imports(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    let text = node_text(node, source).trim().to_string();
    if !text.is_empty() {
        imports.insert(text.clone(), text);
    }
}

/// Ruby: detect require/require_relative calls
fn extract_ruby_require(node: &Node, source: &str, imports: &mut HashMap<String, String>) {
    // In Ruby, require is a method call: require 'json' or require_relative 'helper'
    let mut cursor = node.walk();
    let mut method_name = String::new();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "constant" => {
                let text = node_text(&child, source);
                if text == "require" || text == "require_relative" {
                    method_name = text;
                }
            }
            "argument_list" | "string" | "string_content" => {
                if !method_name.is_empty() {
                    let raw = node_text(&child, source);
                    let module = raw
                        .trim_matches(|c: char| c == '\'' || c == '"' || c == '(' || c == ')')
                        .to_string();
                    if !module.is_empty() {
                        let short = module.rsplit('/').next().unwrap_or(&module).to_string();
                        imports.insert(short, module);
                    }
                    return;
                }
            }
            _ => {
                // Recurse into argument list
                if !method_name.is_empty() {
                    let mut inner = child.walk();
                    for grandchild in child.children(&mut inner) {
                        if grandchild.kind() == "string" || grandchild.kind() == "string_content" {
                            let raw = node_text(&grandchild, source);
                            let module = raw
                                .trim_matches(|c: char| c == '\'' || c == '"')
                                .to_string();
                            if !module.is_empty() {
                                let short =
                                    module.rsplit('/').next().unwrap_or(&module).to_string();
                                imports.insert(short, module);
                            }
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Collect identifiers from a subtree and add them as imports.
fn collect_identifiers_recursive(
    node: &Node,
    source: &str,
    module_path: &str,
    imports: &mut HashMap<String, String>,
) {
    if node.kind() == "identifier" {
        imports.insert(node_text(node, source), module_path.to_string());
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers_recursive(&child, source, module_path, imports);
    }
}

// =============================================================================
// Call Extraction (Generic)
// =============================================================================

/// Extract call sites within a function body (generic, multi-language).
fn extract_calls_generic(
    func_node: &Node,
    source: &str,
    caller_name: &str,
    calls: &mut Vec<(String, String, u32)>,
    config: &LangConfig,
    lang: TldrLanguage,
) {
    let mut stack = vec![*func_node];

    while let Some(node) = stack.pop() {
        if config.call_kinds.contains(&node.kind()) {
            if let Some(callee) = extract_callee_generic(&node, source, lang) {
                let line = node.start_position().row as u32 + 1;
                calls.push((caller_name.to_string(), callee, line));
            }
        }

        // Push children to stack (depth-first traversal)
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
}

/// Extract the callee name from a call node (generic, multi-language).
///
/// Handles:
/// - Simple calls: `func()` -> "func"
/// - Attribute/method calls: `obj.method()` -> "method"
/// - Selector calls (Go): `pkg.Func()` -> "Func"
/// - Java method invocation: `obj.method()` -> "method"
fn extract_callee_generic(call_node: &Node, source: &str, lang: TldrLanguage) -> Option<String> {
    match lang {
        TldrLanguage::Java => {
            // Java method_invocation: child_by_field_name("name") gives the method name
            if let Some(name) = call_node.child_by_field_name("name") {
                return Some(node_text(&name, source));
            }
        }
        TldrLanguage::Go => {
            // Go call_expression: function field is the callee
            if let Some(func) = call_node.child_by_field_name("function") {
                match func.kind() {
                    "identifier" => return Some(node_text(&func, source)),
                    "selector_expression" => {
                        // pkg.Func() -> extract "Func"
                        if let Some(field) = func.child_by_field_name("field") {
                            return Some(node_text(&field, source));
                        }
                    }
                    _ => return Some(node_text(&func, source)),
                }
            }
        }
        TldrLanguage::Php => {
            // PHP function_call_expression or member_call_expression
            if let Some(func) = call_node.child_by_field_name("function") {
                return Some(extract_leaf_identifier(&func, source));
            }
            if let Some(name) = call_node.child_by_field_name("name") {
                return Some(node_text(&name, source));
            }
        }
        TldrLanguage::CSharp => {
            // C# invocation_expression: function is first child
            if let Some(func) = call_node.child_by_field_name("function") {
                return Some(extract_last_identifier(&func, source));
            }
            // Fallback
            let mut cursor = call_node.walk();
            for child in call_node.children(&mut cursor) {
                if child.kind() == "member_access_expression" {
                    if let Some(name) = child.child_by_field_name("name") {
                        return Some(node_text(&name, source));
                    }
                }
                if child.kind() == "identifier" {
                    return Some(node_text(&child, source));
                }
            }
        }
        _ => {}
    }

    // Generic fallback: works for Python, Rust, TypeScript, C, C++, Ruby, etc.
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "name" => {
                return Some(node_text(&child, source));
            }
            "attribute" | "member_expression" | "field_expression" | "selector_expression" => {
                // Get the method/field name (after the dot)
                return Some(extract_last_identifier(&child, source));
            }
            "scoped_identifier" | "qualified_identifier" => {
                // Rust/C++ path::func()
                return Some(extract_last_identifier(&child, source));
            }
            _ => {}
        }
    }
    None
}

/// Extract the last identifier from a dotted/scoped expression.
fn extract_last_identifier(node: &Node, source: &str) -> String {
    let mut last_id = node_text(node, source);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier"
            || child.kind() == "name"
            || child.kind() == "field_identifier"
            || child.kind() == "property_identifier"
        {
            last_id = node_text(&child, source);
        }
    }
    last_id
}

/// Get text content of a node.
fn node_text(node: &Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

// =============================================================================
// Cross-Call Detection
// =============================================================================

/// Find cross-module calls from caller module to callee module.
///
/// A cross-call is detected when:
/// 1. The caller module imports a name from the callee module
/// 2. The caller module calls that imported name
/// 3. The callee module defines that name
pub fn find_cross_calls(caller: &ModuleInfo, callee: &ModuleInfo) -> CrossCalls {
    let mut calls = Vec::new();

    for (caller_func, callee_name, line) in &caller.calls {
        // Check if the callee name is:
        // 1. Imported by the caller module
        // 2. Defined in the callee module
        if caller.imports.contains_key(callee_name) && callee.defined_names.contains(callee_name) {
            calls.push(CrossCall {
                caller: caller_func.clone(),
                callee: callee_name.clone(),
                line: *line,
            });
        }
    }

    let count = calls.len() as u32;
    CrossCalls { calls, count }
}

// =============================================================================
// Coupling Score Computation
// =============================================================================

/// Compute coupling score between two modules.
///
/// The score is computed as:
/// `cross_calls / (total_functions * 2)`
///
/// Where:
/// - `cross_calls` = calls from A to B + calls from B to A
/// - `total_functions` = functions in A + functions in B
///
/// The score is clamped to [0.0, 1.0].
pub fn compute_coupling_score(a_to_b: u32, b_to_a: u32, funcs_a: u32, funcs_b: u32) -> f64 {
    let total_funcs = funcs_a.saturating_add(funcs_b);
    if total_funcs == 0 {
        return 0.0;
    }

    let cross_calls = a_to_b.saturating_add(b_to_a);
    let denominator = (total_funcs as f64) * 2.0;

    (cross_calls as f64 / denominator).min(1.0)
}

// =============================================================================
// Text Formatting
// =============================================================================

/// Format Martin metrics report as human-readable text.
///
/// Renders a table of per-module Ca, Ce, Instability, and cycle membership,
/// followed by a summary line and (if applicable) a list of detected cycles.
pub fn format_martin_text(report: &tldr_core::quality::coupling::MartinMetricsReport) -> String {
    let mut output = String::new();

    output.push_str("Martin Coupling Metrics (project-wide)\n\n");

    if report.metrics.is_empty() {
        output.push_str("No modules found.\n");
        return output;
    }

    // Compute column width for module path (min 6 for "Module", max 40)
    let max_path_len = report
        .metrics
        .iter()
        .map(|m| m.module.to_string_lossy().len())
        .max()
        .unwrap_or(6)
        .clamp(6, 40);

    // Header
    output.push_str(&format!(
        " {:<width$} | {:>2} | {:>2} | {:>6} | Cycle?\n",
        "Module",
        "Ca",
        "Ce",
        "I",
        width = max_path_len,
    ));
    output.push_str(&format!(
        "-{}-+----+----+--------+-------\n",
        "-".repeat(max_path_len),
    ));

    // Rows
    for m in &report.metrics {
        let path_display = m.module.to_string_lossy();
        let truncated_path = if path_display.len() > max_path_len {
            format!(
                "...{}",
                &path_display[path_display.len() - (max_path_len - 3)..]
            )
        } else {
            path_display.to_string()
        };

        let cycle_str = if m.in_cycle { "yes" } else { "--" };

        output.push_str(&format!(
            " {:<width$} | {:>2} | {:>2} |  {:.2}  |   {}\n",
            truncated_path,
            m.ca,
            m.ce,
            m.instability,
            cycle_str,
            width = max_path_len,
        ));
    }

    // Summary line
    output.push_str(&format!(
        "\nSummary: {} modules, {} cycles detected, avg instability: {:.2}\n",
        report.modules_analyzed, report.summary.total_cycles, report.summary.avg_instability,
    ));

    // Cycles section (only if cycles exist)
    if !report.cycles.is_empty() {
        output.push_str("\nCycles:\n");
        for (i, cycle) in report.cycles.iter().enumerate() {
            let path_strs: Vec<String> = cycle
                .path
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            output.push_str(&format!(
                "  {}. {} (length {})\n",
                i + 1,
                path_strs.join(" -> "),
                cycle.length,
            ));
        }
    }

    output
}

/// Format coupling report as human-readable text.
pub fn format_coupling_text(report: &CouplingReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!(
        "Coupling Analysis: {} <-> {}",
        report.path_a, report.path_b
    ));
    lines.push(String::new());
    lines.push(format!(
        "Score: {:.2} ({})",
        report.coupling_score, report.verdict
    ));
    lines.push(format!("Total cross-module calls: {}", report.total_calls));
    lines.push(String::new());

    // A -> B calls
    lines.push(format!(
        "Calls from {} to {}:",
        report.path_a, report.path_b
    ));
    if report.a_to_b.calls.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for call in &report.a_to_b.calls {
            lines.push(format!(
                "  {} -> {} (line {})",
                call.caller, call.callee, call.line
            ));
        }
    }
    lines.push(String::new());

    // B -> A calls
    lines.push(format!(
        "Calls from {} to {}:",
        report.path_b, report.path_a
    ));
    if report.b_to_a.calls.is_empty() {
        lines.push("  (none)".to_string());
    } else {
        for call in &report.b_to_a.calls {
            lines.push(format!(
                "  {} -> {} (line {})",
                call.caller, call.callee, call.line
            ));
        }
    }

    lines.join("\n")
}

// =============================================================================
// Entry Point
// =============================================================================

/// Run the coupling command.
///
/// Two modes:
/// - **Pair mode**: `path_b` is `Some(...)` -- compare two files (original behavior)
/// - **Project-wide mode**: `path_b` is `None` and `path_a` is a directory -- scan all pairs
///
/// Language is auto-detected from file extensions.
pub fn run(args: CouplingArgs, format: OutputFormat) -> Result<()> {
    // Determine mode based on arguments
    match args.path_b {
        Some(ref _path_b) => run_pair_mode(&args, format),
        None if args.path_a.is_dir() => run_project_mode(&args, format),
        None => {
            // path_a is a file but no path_b -- ambiguous
            Err(anyhow::anyhow!(
                "For pair mode, provide two file paths: tldr coupling <file_a> <file_b>\n\
                 For project-wide mode, provide a directory: tldr coupling <directory>"
            ))
        }
    }
}

/// Run pair mode: compare two specific files (original behavior).
fn run_pair_mode(args: &CouplingArgs, format: OutputFormat) -> Result<()> {
    let start = Instant::now();
    let timeout = Duration::from_secs(args.timeout);

    let path_b_ref = args.path_b.as_ref().expect("pair mode requires path_b");

    // Validate paths (TIGER T02 mitigation)
    let path_a = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(&args.path_a, root)?
    } else {
        validate_file_path(&args.path_a)?
    };

    let path_b = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(path_b_ref, root)?
    } else {
        validate_file_path(path_b_ref)?
    };

    // Check timeout after path validation
    if start.elapsed() > timeout {
        return Err(PatternsError::Timeout {
            timeout_secs: args.timeout,
        }
        .into());
    }

    // Read source files
    let source_a = read_file_safe(&path_a)?;
    let source_b = read_file_safe(&path_b)?;

    // Check timeout after file read
    if start.elapsed() > timeout {
        return Err(PatternsError::Timeout {
            timeout_secs: args.timeout,
        }
        .into());
    }

    // Handle self-coupling case
    if path_a == path_b {
        let report = CouplingReport {
            path_a: path_a.to_string_lossy().to_string(),
            path_b: path_b.to_string_lossy().to_string(),
            a_to_b: CrossCalls::default(),
            b_to_a: CrossCalls::default(),
            total_calls: 0,
            coupling_score: 1.0,
            verdict: CouplingVerdict::VeryHigh,
        };

        output_pair_report(&report, format)?;
        return Ok(());
    }

    // Extract module information
    let info_a = extract_module_info(&path_a, &source_a)?;
    let info_b = extract_module_info(&path_b, &source_b)?;

    // Check timeout after parsing
    if start.elapsed() > timeout {
        return Err(PatternsError::Timeout {
            timeout_secs: args.timeout,
        }
        .into());
    }

    // Find cross-module calls
    let a_to_b = find_cross_calls(&info_a, &info_b);
    let b_to_a = find_cross_calls(&info_b, &info_a);

    // Compute coupling score
    let total_calls = a_to_b.count.saturating_add(b_to_a.count);
    let coupling_score = compute_coupling_score(
        a_to_b.count,
        b_to_a.count,
        info_a.function_count,
        info_b.function_count,
    );
    let verdict = CouplingVerdict::from_score(coupling_score);

    // Build report
    let report = CouplingReport {
        path_a: path_a.to_string_lossy().to_string(),
        path_b: path_b.to_string_lossy().to_string(),
        a_to_b,
        b_to_a,
        total_calls,
        coupling_score,
        verdict,
    };

    output_pair_report(&report, format)?;

    Ok(())
}

/// Run project-wide mode: scan a directory for all coupling pairs.
fn run_project_mode(args: &CouplingArgs, format: OutputFormat) -> Result<()> {
    // Existing pairwise coupling analysis
    let mut pairwise_report = core_analyze_coupling(&args.path_a, None, Some(args.max_pairs))
        .map_err(|e| anyhow::anyhow!("coupling analysis failed: {}", e))?;

    // Filter test files from pairwise by default
    if !args.include_tests {
        pairwise_report
            .top_pairs
            .retain(|pair| !is_test_file(&pair.source) && !is_test_file(&pair.target));
    }

    // Martin metrics: compute from dependency graph
    let martin_options = MartinOptions {
        top: args.top,
        cycles_only: args.cycles_only,
    };
    let mut martin_report = match analyze_dependencies(&args.path_a, &DepsOptions::default()) {
        Ok(deps_report) => compute_martin_metrics_from_deps(&deps_report, &martin_options),
        Err(_) => MartinMetricsReport::default(), // no source files or unsupported language
    };

    // Filter test files by default (--include-tests to keep them)
    if !args.include_tests {
        let pre_count = martin_report.metrics.len();
        martin_report.metrics.retain(|m| !is_test_file(&m.module));
        martin_report.modules_analyzed = martin_report.metrics.len();

        // Recalculate summary if we filtered anything
        if martin_report.metrics.len() < pre_count {
            if martin_report.metrics.is_empty() {
                martin_report.summary.avg_instability = 0.0;
                martin_report.summary.most_stable = None;
                martin_report.summary.most_unstable = None;
            } else {
                let sum: f64 = martin_report.metrics.iter().map(|m| m.instability).sum();
                martin_report.summary.avg_instability = sum / martin_report.metrics.len() as f64;
                martin_report.summary.most_stable = martin_report
                    .metrics
                    .iter()
                    .min_by(|a, b| a.instability.partial_cmp(&b.instability).unwrap())
                    .map(|m| m.module.clone());
                martin_report.summary.most_unstable = martin_report
                    .metrics
                    .iter()
                    .max_by(|a, b| a.instability.partial_cmp(&b.instability).unwrap())
                    .map(|m| m.module.clone());
            }
            // Filter cycles to only include non-test modules
            martin_report
                .cycles
                .retain(|cycle| cycle.path.iter().all(|m| !is_test_file(m)));
            martin_report.summary.total_cycles = martin_report.cycles.len();
        }
    }

    output_project_report_with_martin(&pairwise_report, &martin_report, format)?;
    Ok(())
}

/// Output the project-wide report with Martin metrics in the specified format.
fn output_project_report_with_martin(
    pairwise_report: &CoreCouplingReport,
    martin_report: &MartinMetricsReport,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Text => {
            // Martin metrics first, then pairwise coupling
            println!("{}", format_martin_text(martin_report));
            if !pairwise_report.top_pairs.is_empty() {
                println!("{}", format_coupling_project_text(pairwise_report));
            }
        }
        OutputFormat::Compact => {
            let combined = serde_json::json!({
                "martin_metrics": serde_json::to_value(martin_report)?,
                "pairwise_coupling": serde_json::to_value(pairwise_report)?,
            });
            let json = serde_json::to_string(&combined)?;
            println!("{}", json);
        }
        _ => {
            let combined = serde_json::json!({
                "martin_metrics": serde_json::to_value(martin_report)?,
                "pairwise_coupling": serde_json::to_value(pairwise_report)?,
            });
            let json = serde_json::to_string_pretty(&combined)?;
            println!("{}", json);
        }
    }
    Ok(())
}

/// Output the pair-mode report in the specified format.
fn output_pair_report(report: &CouplingReport, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Text => {
            println!("{}", format_coupling_text(report));
        }
        OutputFormat::Compact => {
            let json = serde_json::to_string(report)?;
            println!("{}", json);
        }
        _ => {
            let json = serde_json::to_string_pretty(report)?;
            println!("{}", json);
        }
    }
    Ok(())
}

/// Format a project-wide coupling report as human-readable text.
///
/// Renders a ranked table of the highest-coupling module pairs with color coding:
/// - Tight (>= 0.6): red bold
/// - Moderate (0.3-0.6): yellow
/// - Loose (< 0.3): green
pub fn format_coupling_project_text(report: &CoreCouplingReport) -> String {
    let mut output = String::new();

    output.push_str(&format!(
        "{}\n\n",
        "Coupling Analysis (project-wide)".bold()
    ));

    if report.top_pairs.is_empty() {
        output.push_str(&format!(
            "Summary: {} modules, 0 pairs analyzed\n",
            report.modules_analyzed,
        ));
        return output;
    }

    // Compute common path prefix for relative display
    let all_paths: Vec<&Path> = report
        .top_pairs
        .iter()
        .flat_map(|p| [p.source.as_path(), p.target.as_path()])
        .collect();
    let prefix = common_path_prefix(&all_paths);

    // Header
    output.push_str(&format!(
        " {:>5}  {:>5}  {:>7}  {:>10}  {}\n",
        "Score", "Calls", "Imports", "Verdict", "Source -> Target"
    ));

    // Rows
    for pair in &report.top_pairs {
        let source_rel = strip_prefix_display(&pair.source, &prefix);
        let target_rel = strip_prefix_display(&pair.target, &prefix);

        let verdict_str = match pair.verdict {
            CoreVerdict::Tight => "tight".red().bold().to_string(),
            CoreVerdict::Moderate => "moderate".yellow().to_string(),
            CoreVerdict::Loose => "loose".green().to_string(),
        };

        let score_str = format!("{:.2}", pair.score);
        let score_colored = match pair.verdict {
            CoreVerdict::Tight => score_str.red().bold().to_string(),
            CoreVerdict::Moderate => score_str.yellow().to_string(),
            CoreVerdict::Loose => score_str.green().to_string(),
        };

        output.push_str(&format!(
            " {:>5}  {:>5}  {:>7}  {:>10}  {} -> {}\n",
            score_colored, pair.call_count, pair.import_count, verdict_str, source_rel, target_rel,
        ));
    }

    // Summary line
    let avg_str = report
        .avg_coupling_score
        .map(|s| format!("{:.2}", s))
        .unwrap_or_else(|| "N/A".to_string());

    output.push_str(&format!(
        "\nSummary: {} modules, {} pairs analyzed, {} tight, avg score: {}\n",
        report.modules_analyzed, report.pairs_analyzed, report.tight_coupling_count, avg_str,
    ));

    if report.truncated == Some(true) {
        if let Some(total) = report.total_pairs {
            output.push_str(&format!(
                "  (showing top {} of {} pairs)\n",
                report.top_pairs.len(),
                total,
            ));
        }
    }

    output
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a test file in a temp directory.
    fn create_test_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    // -------------------------------------------------------------------------
    // compute_coupling_score Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_compute_coupling_score_no_calls() {
        let score = compute_coupling_score(0, 0, 5, 5);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_compute_coupling_score_unidirectional() {
        // 2 calls from A to B, 5 functions in A, 5 in B
        // score = 2 / (10 * 2) = 2/20 = 0.1
        let score = compute_coupling_score(2, 0, 5, 5);
        assert!((score - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_compute_coupling_score_bidirectional() {
        // 3 calls A->B, 2 calls B->A, 5 functions each
        // score = 5 / (10 * 2) = 5/20 = 0.25
        let score = compute_coupling_score(3, 2, 5, 5);
        assert!((score - 0.25).abs() < 0.001);
    }

    #[test]
    fn test_compute_coupling_score_no_functions() {
        let score = compute_coupling_score(5, 5, 0, 0);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_compute_coupling_score_clamped() {
        // Many calls, few functions -> clamped to 1.0
        let score = compute_coupling_score(100, 100, 1, 1);
        assert_eq!(score, 1.0);
    }

    // -------------------------------------------------------------------------
    // CouplingVerdict Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_verdict_low() {
        assert_eq!(CouplingVerdict::from_score(0.0), CouplingVerdict::Low);
        assert_eq!(CouplingVerdict::from_score(0.1), CouplingVerdict::Low);
        assert_eq!(CouplingVerdict::from_score(0.19), CouplingVerdict::Low);
    }

    #[test]
    fn test_verdict_moderate() {
        assert_eq!(CouplingVerdict::from_score(0.2), CouplingVerdict::Moderate);
        assert_eq!(CouplingVerdict::from_score(0.3), CouplingVerdict::Moderate);
        assert_eq!(CouplingVerdict::from_score(0.39), CouplingVerdict::Moderate);
    }

    #[test]
    fn test_verdict_high() {
        assert_eq!(CouplingVerdict::from_score(0.4), CouplingVerdict::High);
        assert_eq!(CouplingVerdict::from_score(0.5), CouplingVerdict::High);
        assert_eq!(CouplingVerdict::from_score(0.59), CouplingVerdict::High);
    }

    #[test]
    fn test_verdict_very_high() {
        assert_eq!(CouplingVerdict::from_score(0.6), CouplingVerdict::VeryHigh);
        assert_eq!(CouplingVerdict::from_score(0.8), CouplingVerdict::VeryHigh);
        assert_eq!(CouplingVerdict::from_score(1.0), CouplingVerdict::VeryHigh);
    }

    // -------------------------------------------------------------------------
    // extract_module_info Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_defined_names() {
        let source = r#"
def func_a():
    pass

async def func_b():
    pass

class MyClass:
    pass
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "test.py", source);
        let info = extract_module_info(&path, source).unwrap();

        assert!(info.defined_names.contains("func_a"));
        assert!(info.defined_names.contains("func_b"));
        assert!(info.defined_names.contains("MyClass"));
        assert_eq!(info.function_count, 2);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
import os
import sys as system
from pathlib import Path
from collections import defaultdict, Counter
from typing import List as L
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "test.py", source);
        let info = extract_module_info(&path, source).unwrap();

        assert!(info.imports.contains_key("os"));
        assert!(info.imports.contains_key("system"));
        assert!(info.imports.contains_key("Path"));
        assert!(info.imports.contains_key("defaultdict"));
        assert!(info.imports.contains_key("Counter"));
    }

    #[test]
    fn test_extract_calls() {
        let source = r#"
def caller():
    result = helper()
    obj.method()
    other_func(1, 2, 3)
    return result
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "test.py", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should find calls to helper, method, other_func
        let callees: Vec<&str> = info
            .calls
            .iter()
            .map(|(_, callee, _)| callee.as_str())
            .collect();
        assert!(callees.contains(&"helper"));
        assert!(callees.contains(&"method"));
        assert!(callees.contains(&"other_func"));
    }

    // -------------------------------------------------------------------------
    // find_cross_calls Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_cross_calls_simple() {
        let temp = TempDir::new().unwrap();

        // Module A imports and calls helper from module B
        let source_a = r#"
from module_b import helper

def caller():
    return helper()
"#;
        let path_a = create_test_file(&temp, "module_a.py", source_a);
        let info_a = extract_module_info(&path_a, source_a).unwrap();

        // Module B defines helper
        let source_b = r#"
def helper():
    return 42
"#;
        let path_b = create_test_file(&temp, "module_b.py", source_b);
        let info_b = extract_module_info(&path_b, source_b).unwrap();

        let cross_calls = find_cross_calls(&info_a, &info_b);

        assert_eq!(cross_calls.count, 1);
        assert_eq!(cross_calls.calls[0].caller, "caller");
        assert_eq!(cross_calls.calls[0].callee, "helper");
    }

    #[test]
    fn test_find_cross_calls_no_import() {
        let temp = TempDir::new().unwrap();

        // Module A calls helper but doesn't import it
        let source_a = r#"
def caller():
    return helper()
"#;
        let path_a = create_test_file(&temp, "module_a.py", source_a);
        let info_a = extract_module_info(&path_a, source_a).unwrap();

        // Module B defines helper
        let source_b = r#"
def helper():
    return 42
"#;
        let path_b = create_test_file(&temp, "module_b.py", source_b);
        let info_b = extract_module_info(&path_b, source_b).unwrap();

        let cross_calls = find_cross_calls(&info_a, &info_b);

        // No cross-calls since helper wasn't imported
        assert_eq!(cross_calls.count, 0);
    }

    #[test]
    fn test_find_cross_calls_bidirectional() {
        let temp = TempDir::new().unwrap();

        // Module A imports and calls helper from B
        let source_a = r#"
from module_b import helper_b

def func_a():
    return helper_b()
"#;
        let path_a = create_test_file(&temp, "module_a.py", source_a);
        let info_a = extract_module_info(&path_a, source_a).unwrap();

        // Module B imports and calls func_a from A
        let source_b = r#"
from module_a import func_a

def helper_b():
    return 42

def caller_b():
    return func_a()
"#;
        let path_b = create_test_file(&temp, "module_b.py", source_b);
        let info_b = extract_module_info(&path_b, source_b).unwrap();

        let a_to_b = find_cross_calls(&info_a, &info_b);
        let b_to_a = find_cross_calls(&info_b, &info_a);

        assert_eq!(a_to_b.count, 1);
        assert_eq!(b_to_a.count, 1);
    }

    // -------------------------------------------------------------------------
    // format_coupling_text Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_format_coupling_text() {
        let report = CouplingReport {
            path_a: "src/auth.py".to_string(),
            path_b: "src/user.py".to_string(),
            a_to_b: CrossCalls {
                calls: vec![CrossCall {
                    caller: "login".to_string(),
                    callee: "get_user".to_string(),
                    line: 10,
                }],
                count: 1,
            },
            b_to_a: CrossCalls::default(),
            total_calls: 1,
            coupling_score: 0.15,
            verdict: CouplingVerdict::Low,
        };

        let text = format_coupling_text(&report);

        assert!(text.contains("src/auth.py"));
        assert!(text.contains("src/user.py"));
        assert!(text.contains("0.15"));
        assert!(text.contains("low"));
        assert!(text.contains("login"));
        assert!(text.contains("get_user"));
        assert!(text.contains("line 10"));
    }

    // -------------------------------------------------------------------------
    // Integration Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_run_no_coupling() {
        let temp = TempDir::new().unwrap();

        let source_a = r#"
def standalone_a():
    return 1
"#;
        let source_b = r#"
def standalone_b():
    return 2
"#;

        let path_a = create_test_file(&temp, "a.py", source_a);
        let path_b = create_test_file(&temp, "b.py", source_b);

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        // Just verify it runs without error
        let result = run(args, OutputFormat::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_with_coupling() {
        let temp = TempDir::new().unwrap();

        let source_a = r#"
from b import helper

def caller():
    return helper()
"#;
        let source_b = r#"
def helper():
    return 42
"#;

        let path_a = create_test_file(&temp, "a.py", source_a);
        let path_b = create_test_file(&temp, "b.py", source_b);

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        let result = run(args, OutputFormat::Json);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Multi-language Tests
    // =========================================================================

    #[test]
    fn test_go_extract_module_info() {
        let source = r#"
package main

import (
    "fmt"
    "myapp/utils"
)

func Caller() {
    utils.Helper()
    fmt.Println("hello")
}

func Standalone() int {
    return 42
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.go", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect functions
        assert!(info.defined_names.contains("Caller"), "missing Caller");
        assert!(
            info.defined_names.contains("Standalone"),
            "missing Standalone"
        );
        assert_eq!(info.function_count, 2);

        // Should detect imports
        assert!(
            info.imports.contains_key("fmt") || info.imports.values().any(|v| v.contains("fmt")),
            "missing fmt import: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_go_cross_calls() {
        let temp = TempDir::new().unwrap();

        let source_a = r#"
package main

import "myapp/pkg_b"

func CallerA() {
    pkg_b.HelperB()
}
"#;
        let source_b = r#"
package pkg_b

func HelperB() int {
    return 42
}
"#;
        let path_a = create_test_file(&temp, "a.go", source_a);
        let path_b = create_test_file(&temp, "b.go", source_b);

        let info_a = extract_module_info(&path_a, source_a).unwrap();
        let info_b = extract_module_info(&path_b, source_b).unwrap();

        // Should find the cross-call from A to B
        let a_to_b = find_cross_calls(&info_a, &info_b);
        assert!(
            a_to_b.count >= 1,
            "expected cross-calls from A to B, got {}",
            a_to_b.count
        );
    }

    #[test]
    fn test_rust_extract_module_info() {
        let source = r#"
use std::collections::HashMap;
use crate::module_b::helper;

pub fn caller() {
    let _ = helper();
}

fn standalone() -> i32 {
    42
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "lib.rs", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect functions
        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);

        // Should detect imports
        assert!(
            !info.imports.is_empty(),
            "should have imports: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_typescript_extract_module_info() {
        let source = r#"
import { helper } from './module_b';
import * as utils from './utils';

function caller(): void {
    helper();
    utils.doStuff();
}

function standalone(): number {
    return 42;
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.ts", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect functions
        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);

        // Should detect imports
        assert!(
            !info.imports.is_empty(),
            "should have imports: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_java_extract_module_info() {
        let source = r#"
import com.example.utils.Helper;
import java.util.List;

public class Main {
    public void caller() {
        Helper.doWork();
    }

    public int standalone() {
        return 42;
    }
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "Main.java", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect methods
        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );

        // Should detect imports
        assert!(
            !info.imports.is_empty(),
            "should have imports: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_c_extract_module_info() {
        let source = r#"
#include <stdio.h>
#include "mylib.h"

void caller() {
    helper();
    printf("hello\n");
}

int standalone() {
    return 42;
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.c", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect functions
        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);

        // Should detect includes as imports
        assert!(
            !info.imports.is_empty(),
            "should have imports from #include: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_ruby_extract_module_info() {
        let source = r#"
require 'json'
require_relative 'helper'

def caller
  helper_method
  JSON.parse("{}")
end

def standalone
  42
end
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.rb", source);
        let info = extract_module_info(&path, source).unwrap();

        // Should detect methods
        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);

        // Should detect requires as imports
        assert!(
            !info.imports.is_empty(),
            "should have imports from require: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_cpp_extract_module_info() {
        let source = r#"
#include <iostream>
#include "mylib.hpp"

void caller() {
    helper();
    std::cout << "hello" << std::endl;
}

int standalone() {
    return 42;
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.cpp", source);
        let info = extract_module_info(&path, source).unwrap();

        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);
        assert!(
            !info.imports.is_empty(),
            "should have imports from #include: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_php_extract_module_info() {
        let source = r#"<?php
use App\Utils\Helper;
use Symfony\Component\Console\Command;

function caller() {
    Helper::doWork();
}

function standalone() {
    return 42;
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "main.php", source);
        let info = extract_module_info(&path, source).unwrap();

        assert!(info.defined_names.contains("caller"), "missing caller");
        assert!(
            info.defined_names.contains("standalone"),
            "missing standalone"
        );
        assert_eq!(info.function_count, 2);
        assert!(
            !info.imports.is_empty(),
            "should have imports from use: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_csharp_extract_module_info() {
        let source = r#"
using System;
using MyApp.Utils;

public class Main {
    public void Caller() {
        Helper.DoWork();
    }

    public int Standalone() {
        return 42;
    }
}
"#;
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "Main.cs", source);
        let info = extract_module_info(&path, source).unwrap();

        assert!(info.defined_names.contains("Caller"), "missing Caller");
        assert!(
            info.defined_names.contains("Standalone"),
            "missing Standalone"
        );
        assert!(
            !info.imports.is_empty(),
            "should have imports from using: {:?}",
            info.imports
        );
    }

    #[test]
    fn test_run_go_coupling() {
        let temp = TempDir::new().unwrap();

        let source_a = r#"
package main

func standalone_a() int {
    return 1
}
"#;
        let source_b = r#"
package main

func standalone_b() int {
    return 2
}
"#;

        let path_a = create_test_file(&temp, "a.go", source_a);
        let path_b = create_test_file(&temp, "b.go", source_b);

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        let result = run(args, OutputFormat::Json);
        assert!(
            result.is_ok(),
            "coupling should work for Go files: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_run_rust_coupling() {
        let temp = TempDir::new().unwrap();

        let source_a = r#"
fn standalone_a() -> i32 {
    1
}
"#;
        let source_b = r#"
fn standalone_b() -> i32 {
    2
}
"#;

        let path_a = create_test_file(&temp, "a.rs", source_a);
        let path_b = create_test_file(&temp, "b.rs", source_b);

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        let result = run(args, OutputFormat::Json);
        assert!(
            result.is_ok(),
            "coupling should work for Rust files: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_unsupported_extension_returns_error() {
        let temp = TempDir::new().unwrap();
        let path = create_test_file(&temp, "data.xyz", "some content");
        let result = extract_module_info(&path, "some content");
        assert!(
            result.is_err(),
            "unsupported file extension should return error"
        );
    }

    // =========================================================================
    // Project-Wide Scan Mode Tests
    // =========================================================================

    #[test]
    fn test_coupling_args_pair_mode_backward_compat() {
        // Pair mode: path_a and path_b both set
        let args = CouplingArgs {
            path_a: PathBuf::from("src/a.py"),
            path_b: Some(PathBuf::from("src/b.py")),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert!(args.path_b.is_some());
    }

    #[test]
    fn test_coupling_args_project_wide_mode() {
        // Project-wide mode: only path_a, no path_b
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert!(args.path_b.is_none());
    }

    #[test]
    fn test_coupling_args_max_pairs_default() {
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert_eq!(args.max_pairs, 20);
    }

    #[test]
    fn test_coupling_args_max_pairs_custom() {
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 5,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert_eq!(args.max_pairs, 5);
    }

    #[test]
    fn test_run_project_wide_mode() {
        let temp = TempDir::new().unwrap();

        // Create a small project with multiple Python files
        let source_a = r#"
from b import helper

def caller():
    return helper()
"#;
        let source_b = r#"
def helper():
    return 42
"#;
        let source_c = r#"
def standalone():
    return 99
"#;

        create_test_file(&temp, "a.py", source_a);
        create_test_file(&temp, "b.py", source_b);
        create_test_file(&temp, "c.py", source_c);

        // Project-wide: pass directory as path_a, no path_b
        let args = CouplingArgs {
            path_a: temp.path().to_path_buf(),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        let result = run(args, OutputFormat::Json);
        assert!(
            result.is_ok(),
            "project-wide coupling should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_run_pair_mode_still_works() {
        // Backward compatibility: pair mode must still work
        let temp = TempDir::new().unwrap();

        let source_a = r#"
from b import helper

def caller():
    return helper()
"#;
        let source_b = r#"
def helper():
    return 42
"#;

        let path_a = create_test_file(&temp, "a.py", source_a);
        let path_b = create_test_file(&temp, "b.py", source_b);

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        let result = run(args, OutputFormat::Json);
        assert!(
            result.is_ok(),
            "pair mode should still work: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_format_coupling_project_text_basic() {
        use tldr_core::quality::coupling::{
            CouplingReport as CoreCouplingReport, CouplingVerdict as CoreVerdict,
            ModuleCoupling as CoreModuleCoupling,
        };

        let report = CoreCouplingReport {
            modules_analyzed: 10,
            pairs_analyzed: 45,
            total_cross_file_pairs: 8,
            avg_coupling_score: Some(0.25),
            tight_coupling_count: 2,
            top_pairs: vec![
                CoreModuleCoupling {
                    source: PathBuf::from("src/services/auth.rs"),
                    target: PathBuf::from("src/db/users.rs"),
                    import_count: 8,
                    call_count: 12,
                    calls_source_to_target: vec![],
                    calls_target_to_source: vec![],
                    shared_imports: vec![],
                    score: 0.72,
                    verdict: CoreVerdict::Tight,
                },
                CoreModuleCoupling {
                    source: PathBuf::from("src/api/routes.rs"),
                    target: PathBuf::from("src/services/auth.rs"),
                    import_count: 5,
                    call_count: 7,
                    calls_source_to_target: vec![],
                    calls_target_to_source: vec![],
                    shared_imports: vec![],
                    score: 0.55,
                    verdict: CoreVerdict::Moderate,
                },
                CoreModuleCoupling {
                    source: PathBuf::from("src/handlers/web.rs"),
                    target: PathBuf::from("src/api/routes.rs"),
                    import_count: 3,
                    call_count: 5,
                    calls_source_to_target: vec![],
                    calls_target_to_source: vec![],
                    shared_imports: vec![],
                    score: 0.15,
                    verdict: CoreVerdict::Loose,
                },
            ],
            truncated: None,
            total_pairs: None,
            shown_pairs: None,
        };

        let text = format_coupling_project_text(&report);

        // Header
        assert!(
            text.contains("project-wide"),
            "should contain 'project-wide': {}",
            text
        );
        // Table header columns
        assert!(
            text.contains("Score"),
            "should contain Score header: {}",
            text
        );
        assert!(
            text.contains("Calls"),
            "should contain Calls header: {}",
            text
        );
        assert!(
            text.contains("Imports"),
            "should contain Imports header: {}",
            text
        );
        assert!(
            text.contains("Verdict"),
            "should contain Verdict header: {}",
            text
        );
        // Data rows
        assert!(
            text.contains("0.72"),
            "should contain tight score: {}",
            text
        );
        assert!(
            text.contains("0.55"),
            "should contain moderate score: {}",
            text
        );
        assert!(
            text.contains("0.15"),
            "should contain loose score: {}",
            text
        );
        // Verdict labels
        assert!(
            text.contains("tight"),
            "should contain tight verdict: {}",
            text
        );
        assert!(
            text.contains("moderate"),
            "should contain moderate verdict: {}",
            text
        );
        assert!(
            text.contains("loose"),
            "should contain loose verdict: {}",
            text
        );
        // Summary line
        assert!(
            text.contains("10 modules"),
            "should contain module count: {}",
            text
        );
        assert!(
            text.contains("45 pairs"),
            "should contain pair count: {}",
            text
        );
        assert!(
            text.contains("2 tight"),
            "should contain tight count: {}",
            text
        );
    }

    #[test]
    fn test_format_coupling_project_text_empty() {
        use tldr_core::quality::coupling::CouplingReport as CoreCouplingReport;

        let report = CoreCouplingReport::default();

        let text = format_coupling_project_text(&report);

        assert!(
            text.contains("project-wide"),
            "should contain 'project-wide': {}",
            text
        );
        assert!(
            text.contains("0 modules"),
            "should contain zero modules: {}",
            text
        );
    }

    // =========================================================================
    // Martin Metrics Text Formatter Tests
    // =========================================================================

    #[test]
    fn test_format_martin_text_basic() {
        use tldr_core::quality::coupling::{
            MartinMetricsReport, MartinModuleMetrics, MartinSummary,
        };

        let report = MartinMetricsReport {
            schema_version: "1.0".to_string(),
            modules_analyzed: 2,
            metrics: vec![
                MartinModuleMetrics {
                    module: PathBuf::from("src/api.py"),
                    ca: 0,
                    ce: 3,
                    instability: 1.0,
                    in_cycle: false,
                },
                MartinModuleMetrics {
                    module: PathBuf::from("src/db.py"),
                    ca: 2,
                    ce: 0,
                    instability: 0.0,
                    in_cycle: false,
                },
            ],
            cycles: vec![],
            summary: MartinSummary {
                avg_instability: 0.5,
                total_cycles: 0,
                most_stable: Some(PathBuf::from("src/db.py")),
                most_unstable: Some(PathBuf::from("src/api.py")),
            },
        };

        let text = format_martin_text(&report);
        assert!(
            text.contains("Module"),
            "should contain Module header: {}",
            text
        );
        assert!(text.contains("Ca"), "should contain Ca header: {}", text);
        assert!(text.contains("Ce"), "should contain Ce header: {}", text);
        assert!(
            text.contains("Cycle?"),
            "should contain Cycle? header: {}",
            text
        );
    }

    #[test]
    fn test_format_martin_text_empty() {
        use tldr_core::quality::coupling::MartinMetricsReport;

        let report = MartinMetricsReport::default();
        let text = format_martin_text(&report);
        assert!(
            text.contains("No modules found"),
            "empty report should say 'No modules found': {}",
            text
        );
    }

    #[test]
    fn test_format_martin_text_with_cycles() {
        use tldr_core::analysis::deps::DepCycle;
        use tldr_core::quality::coupling::{
            MartinMetricsReport, MartinModuleMetrics, MartinSummary,
        };

        let cycle = DepCycle::new(vec![PathBuf::from("a.py"), PathBuf::from("b.py")]);
        let report = MartinMetricsReport {
            schema_version: "1.0".to_string(),
            modules_analyzed: 2,
            metrics: vec![
                MartinModuleMetrics {
                    module: PathBuf::from("a.py"),
                    ca: 1,
                    ce: 1,
                    instability: 0.5,
                    in_cycle: true,
                },
                MartinModuleMetrics {
                    module: PathBuf::from("b.py"),
                    ca: 1,
                    ce: 1,
                    instability: 0.5,
                    in_cycle: true,
                },
            ],
            cycles: vec![cycle],
            summary: MartinSummary {
                avg_instability: 0.5,
                total_cycles: 1,
                most_stable: Some(PathBuf::from("a.py")),
                most_unstable: Some(PathBuf::from("a.py")),
            },
        };

        let text = format_martin_text(&report);
        assert!(
            text.contains("Cycles:"),
            "should contain 'Cycles:' section: {}",
            text
        );
        assert!(
            text.contains("->"),
            "should contain '->' in cycle display: {}",
            text
        );
    }

    #[test]
    fn test_format_martin_text_no_cycles() {
        use tldr_core::quality::coupling::{
            MartinMetricsReport, MartinModuleMetrics, MartinSummary,
        };

        let report = MartinMetricsReport {
            schema_version: "1.0".to_string(),
            modules_analyzed: 1,
            metrics: vec![MartinModuleMetrics {
                module: PathBuf::from("a.py"),
                ca: 0,
                ce: 0,
                instability: 0.0,
                in_cycle: false,
            }],
            cycles: vec![],
            summary: MartinSummary {
                avg_instability: 0.0,
                total_cycles: 0,
                most_stable: Some(PathBuf::from("a.py")),
                most_unstable: Some(PathBuf::from("a.py")),
            },
        };

        let text = format_martin_text(&report);
        assert!(
            !text.contains("Cycles:"),
            "should NOT contain 'Cycles:' section when no cycles: {}",
            text
        );
    }

    #[test]
    fn test_format_martin_text_summary_line() {
        use tldr_core::quality::coupling::{
            MartinMetricsReport, MartinModuleMetrics, MartinSummary,
        };

        let report = MartinMetricsReport {
            schema_version: "1.0".to_string(),
            modules_analyzed: 3,
            metrics: vec![MartinModuleMetrics {
                module: PathBuf::from("a.py"),
                ca: 0,
                ce: 1,
                instability: 1.0,
                in_cycle: false,
            }],
            cycles: vec![],
            summary: MartinSummary {
                avg_instability: 0.5,
                total_cycles: 0,
                most_stable: Some(PathBuf::from("c.py")),
                most_unstable: Some(PathBuf::from("a.py")),
            },
        };

        let text = format_martin_text(&report);
        assert!(
            text.contains("modules"),
            "should contain 'modules' in summary: {}",
            text
        );
        assert!(
            text.contains("avg instability"),
            "should contain 'avg instability' in summary: {}",
            text
        );
    }

    #[test]
    fn test_format_coupling_project_text_path_stripping() {
        use tldr_core::quality::coupling::{
            CouplingReport as CoreCouplingReport, CouplingVerdict as CoreVerdict,
            ModuleCoupling as CoreModuleCoupling,
        };

        let report = CoreCouplingReport {
            modules_analyzed: 2,
            pairs_analyzed: 1,
            total_cross_file_pairs: 1,
            avg_coupling_score: Some(0.50),
            tight_coupling_count: 0,
            top_pairs: vec![CoreModuleCoupling {
                source: PathBuf::from("/home/user/project/src/auth.rs"),
                target: PathBuf::from("/home/user/project/src/db.rs"),
                import_count: 3,
                call_count: 4,
                calls_source_to_target: vec![],
                calls_target_to_source: vec![],
                shared_imports: vec![],
                score: 0.50,
                verdict: CoreVerdict::Moderate,
            }],
            truncated: None,
            total_pairs: None,
            shown_pairs: None,
        };

        let text = format_coupling_project_text(&report);

        // Should strip common prefix and show relative paths
        assert!(
            text.contains("auth.rs"),
            "should show relative path auth.rs: {}",
            text
        );
        assert!(
            text.contains("db.rs"),
            "should show relative path db.rs: {}",
            text
        );
        // Should NOT contain the full absolute path
        assert!(
            !text.contains("/home/user/project/src/auth.rs"),
            "should strip common prefix from paths: {}",
            text
        );
    }

    // =========================================================================
    // Phase 3: CLI Args + Project-Mode Martin Integration Tests
    // =========================================================================

    #[test]
    fn test_coupling_args_top_flag() {
        // Verify CouplingArgs can be constructed with top == 5
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 5,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert_eq!(args.top, 5);
    }

    #[test]
    fn test_coupling_args_cycles_only_flag() {
        // Verify CouplingArgs can be constructed with cycles_only == true
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: true,
            lang: None,
            include_tests: false,
        };
        assert!(args.cycles_only);
    }

    #[test]
    fn test_coupling_args_defaults() {
        // Verify default values: top == 0, cycles_only == false
        let args = CouplingArgs {
            path_a: PathBuf::from("src/"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert_eq!(args.top, 0);
        assert!(!args.cycles_only);
    }

    #[test]
    fn test_project_mode_produces_martin_output() {
        // Create tempdir with 3 Python files: a imports b, b imports c, c standalone
        let temp = TempDir::new().unwrap();

        create_test_file(
            &temp,
            "a.py",
            "from b import helper_b\n\ndef func_a():\n    return helper_b()\n",
        );
        create_test_file(
            &temp,
            "b.py",
            "from c import helper_c\n\ndef helper_b():\n    return helper_c()\n",
        );
        create_test_file(&temp, "c.py", "def helper_c():\n    return 42\n");

        let args = CouplingArgs {
            path_a: temp.path().to_path_buf(),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };

        // Run project mode and capture output
        let result = run(args, OutputFormat::Text);
        assert!(
            result.is_ok(),
            "project mode should succeed: {:?}",
            result.err()
        );
        // The text output goes to stdout; we verify it doesn't fail.
        // For deeper content check, call the internal function directly.
    }

    #[test]
    fn test_project_mode_json_has_martin_fields() {
        use serde_json::Value;

        let temp = TempDir::new().unwrap();

        create_test_file(
            &temp,
            "a.py",
            "from b import helper_b\n\ndef func_a():\n    return helper_b()\n",
        );
        create_test_file(
            &temp,
            "b.py",
            "from c import helper_c\n\ndef helper_b():\n    return helper_c()\n",
        );
        create_test_file(&temp, "c.py", "def helper_c():\n    return 42\n");

        // Call the internal functions directly to get the martin report
        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::{compute_martin_metrics_from_deps, MartinOptions};

        let deps_report = analyze_dependencies(temp.path(), &DepsOptions::default()).unwrap();
        let martin_report = compute_martin_metrics_from_deps(
            &deps_report,
            &MartinOptions {
                top: 0,
                cycles_only: false,
            },
        );

        let json = serde_json::to_string_pretty(&martin_report).unwrap();
        let parsed: Value = serde_json::from_str(&json).unwrap();

        assert!(
            parsed.get("modules_analyzed").is_some(),
            "JSON should have 'modules_analyzed': {}",
            json
        );
        assert!(
            parsed.get("metrics").is_some(),
            "JSON should have 'metrics': {}",
            json
        );
        assert!(
            parsed.get("summary").is_some(),
            "JSON should have 'summary': {}",
            json
        );
    }

    #[test]
    fn test_project_mode_cycles_only_filter() {
        // Create A->B->A cycle + C->D no-cycle
        let temp = TempDir::new().unwrap();

        create_test_file(
            &temp,
            "a.py",
            "from b import func_b\n\ndef func_a():\n    return func_b()\n",
        );
        create_test_file(
            &temp,
            "b.py",
            "from a import func_a\n\ndef func_b():\n    return func_a()\n",
        );
        create_test_file(
            &temp,
            "c.py",
            "from d import func_d\n\ndef func_c():\n    return func_d()\n",
        );
        create_test_file(&temp, "d.py", "def func_d():\n    return 42\n");

        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::{compute_martin_metrics_from_deps, MartinOptions};

        let deps_report = analyze_dependencies(temp.path(), &DepsOptions::default()).unwrap();
        let martin_report = compute_martin_metrics_from_deps(
            &deps_report,
            &MartinOptions {
                top: 0,
                cycles_only: true,
            },
        );

        // With cycles_only, only modules in cycles should appear in metrics
        for m in &martin_report.metrics {
            assert!(
                m.in_cycle,
                "cycles_only filter should only include cycle modules, got: {:?}",
                m.module
            );
        }
    }

    #[test]
    fn test_project_mode_top_n_limits() {
        // Create 5+ modules
        let temp = TempDir::new().unwrap();

        create_test_file(
            &temp,
            "a.py",
            "from b import fb\n\ndef fa():\n    return fb()\n",
        );
        create_test_file(
            &temp,
            "b.py",
            "from c import fc\n\ndef fb():\n    return fc()\n",
        );
        create_test_file(
            &temp,
            "c.py",
            "from d import fd\n\ndef fc():\n    return fd()\n",
        );
        create_test_file(
            &temp,
            "d.py",
            "from e import fe\n\ndef fd():\n    return fe()\n",
        );
        create_test_file(&temp, "e.py", "def fe():\n    return 42\n");

        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::{compute_martin_metrics_from_deps, MartinOptions};

        let deps_report = analyze_dependencies(temp.path(), &DepsOptions::default()).unwrap();
        let martin_report = compute_martin_metrics_from_deps(
            &deps_report,
            &MartinOptions {
                top: 2,
                cycles_only: false,
            },
        );

        assert!(
            martin_report.metrics.len() <= 2,
            "top 2 should limit metrics to at most 2, got {}",
            martin_report.metrics.len()
        );
        // modules_analyzed should still reflect the total count
        assert!(
            martin_report.modules_analyzed >= 3,
            "modules_analyzed should reflect total (not filtered), got {}",
            martin_report.modules_analyzed
        );
    }

    #[test]
    fn test_pair_mode_unchanged() {
        // Pair mode should still work with the new fields present
        let temp = TempDir::new().unwrap();

        let path_a = create_test_file(&temp, "a.py", "def standalone_a():\n    return 1\n");
        let path_b = create_test_file(&temp, "b.py", "def standalone_b():\n    return 2\n");

        let args = CouplingArgs {
            path_a: path_a.clone(),
            path_b: Some(path_b.clone()),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 3,
            cycles_only: true,
            lang: None,
            include_tests: false,
        };

        // Pair mode should ignore top and cycles_only, and succeed
        let result = run(args, OutputFormat::Json);
        assert!(
            result.is_ok(),
            "pair mode with new flags should still work: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_project_mode_empty_dir() {
        // analyze_dependencies errors on a directory with no source files
        // (it can't auto-detect a language). Verify we handle this gracefully.
        let temp = TempDir::new().unwrap();

        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::MartinMetricsReport;

        let deps_result = analyze_dependencies(temp.path(), &DepsOptions::default());
        // Empty dir → error is expected from analyze_dependencies
        // The empty MartinMetricsReport should format as "No modules found"
        match deps_result {
            Err(_) => {
                let empty_report = MartinMetricsReport::default();
                let text = format_martin_text(&empty_report);
                assert!(
                    text.contains("No modules found"),
                    "empty report should say 'No modules found': {}",
                    text
                );
            }
            Ok(deps_report) => {
                // If analyze_dependencies somehow succeeds with 0 modules, verify that too
                use tldr_core::quality::coupling::{
                    compute_martin_metrics_from_deps, MartinOptions,
                };
                let martin_report = compute_martin_metrics_from_deps(
                    &deps_report,
                    &MartinOptions {
                        top: 0,
                        cycles_only: false,
                    },
                );
                assert_eq!(
                    martin_report.modules_analyzed, 0,
                    "empty dir should have 0 modules"
                );
                let text = format_martin_text(&martin_report);
                assert!(
                    text.contains("No modules found"),
                    "empty dir text should say 'No modules found': {}",
                    text
                );
            }
        }
    }

    #[test]
    fn test_project_mode_single_file() {
        let temp = TempDir::new().unwrap();

        create_test_file(&temp, "only.py", "def lonely():\n    return 1\n");

        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::{compute_martin_metrics_from_deps, MartinOptions};

        let deps_report = analyze_dependencies(temp.path(), &DepsOptions::default()).unwrap();
        let martin_report = compute_martin_metrics_from_deps(
            &deps_report,
            &MartinOptions {
                top: 0,
                cycles_only: false,
            },
        );

        // Single file should show in output (at least 1 module analyzed)
        assert!(
            martin_report.modules_analyzed >= 1,
            "single file should produce at least 1 module, got {}",
            martin_report.modules_analyzed
        );
    }

    // =========================================================================
    // Phase 4: Edge Case Tests
    // =========================================================================

    #[test]
    fn test_format_martin_json_schema() {
        // JSON output should include schema_version field
        use serde_json::Value;
        use tldr_core::quality::coupling::{
            MartinMetricsReport, MartinModuleMetrics, MartinSummary,
        };

        let report = MartinMetricsReport {
            schema_version: "1.0".to_string(),
            modules_analyzed: 1,
            metrics: vec![MartinModuleMetrics {
                module: PathBuf::from("a.py"),
                ca: 0,
                ce: 0,
                instability: 0.0,
                in_cycle: false,
            }],
            cycles: vec![],
            summary: MartinSummary {
                avg_instability: 0.0,
                total_cycles: 0,
                most_stable: Some(PathBuf::from("a.py")),
                most_unstable: Some(PathBuf::from("a.py")),
            },
        };

        let json_str = serde_json::to_string_pretty(&report).unwrap();
        let parsed: Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(
            parsed["schema_version"].as_str(),
            Some("1.0"),
            "JSON should contain schema_version=1.0, got: {}",
            json_str
        );
    }

    #[test]
    fn test_project_mode_top_and_cycles_combined() {
        // --top 2 --cycles-only should show max 2 cycle-participating modules
        let temp = TempDir::new().unwrap();

        // Create 4 modules: A<->B cycle, B<->C cycle, D standalone
        // This gives us A, B, C in cycles
        create_test_file(
            &temp,
            "a.py",
            "from b import fb\n\ndef fa():\n    return fb()\n",
        );
        create_test_file(
            &temp,
            "b.py",
            "from a import fa\nfrom c import fc\n\ndef fb():\n    return fa() + fc()\n",
        );
        create_test_file(
            &temp,
            "c.py",
            "from b import fb\n\ndef fc():\n    return fb()\n",
        );
        create_test_file(&temp, "d.py", "def fd():\n    return 42\n");

        use tldr_core::analysis::deps::{analyze_dependencies, DepsOptions};
        use tldr_core::quality::coupling::{compute_martin_metrics_from_deps, MartinOptions};

        let deps_report = analyze_dependencies(temp.path(), &DepsOptions::default()).unwrap();
        let martin_report = compute_martin_metrics_from_deps(
            &deps_report,
            &MartinOptions {
                top: 2,
                cycles_only: true,
            },
        );

        // With both filters: should show at most 2 modules, and all must be in_cycle
        assert!(
            martin_report.metrics.len() <= 2,
            "top 2 + cycles_only should limit to at most 2 modules, got {}",
            martin_report.metrics.len()
        );
        for m in &martin_report.metrics {
            assert!(
                m.in_cycle,
                "all returned modules should be in_cycle, but {:?} is not",
                m.module
            );
        }
    }

    #[test]
    fn test_coupling_args_lang_flag() {
        // Verify CouplingArgs has a lang field of type Option<Language>
        let args = CouplingArgs {
            path_a: PathBuf::from("src/a.ts"),
            path_b: Some(PathBuf::from("src/b.ts")),
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: Some(TldrLanguage::TypeScript),
            include_tests: false,
        };
        assert_eq!(args.lang, Some(TldrLanguage::TypeScript));

        // Also test None case (auto-detect)
        let args_auto = CouplingArgs {
            path_a: PathBuf::from("src/a.py"),
            path_b: None,
            timeout: 30,
            project_root: None,
            max_pairs: 20,
            top: 0,
            cycles_only: false,
            lang: None,
            include_tests: false,
        };
        assert_eq!(args_auto.lang, None);
    }
}
