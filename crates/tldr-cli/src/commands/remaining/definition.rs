//! Definition command - Go-to-definition functionality
//!
//! Finds where a symbol is defined in the codebase.
//! Supports both position-based and name-based lookup.
//!
//! # Example
//!
//! ```bash
//! # Position-based: find definition of symbol at line 10, column 5
//! tldr definition src/main.py 10 5
//!
//! # Name-based: find definition by symbol name
//! tldr definition --symbol MyClass --file src/main.py
//!
//! # Cross-file resolution with project context
//! tldr definition --symbol helper --file src/main.py --project .
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use tree_sitter::{Node, Parser};
use tree_sitter_python::LANGUAGE as PYTHON_LANGUAGE;

use super::error::{RemainingError, RemainingResult};
use super::types::{DefinitionResult, Location, SymbolInfo, SymbolKind};
use crate::output::OutputWriter;

use tldr_core::Language;

// =============================================================================
// Constants
// =============================================================================

/// Maximum depth for import resolution to prevent cycles
const MAX_IMPORT_DEPTH: usize = 10;

/// Python built-in functions
const PYTHON_BUILTINS: &[&str] = &[
    "abs",
    "aiter",
    "all",
    "any",
    "anext",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
];

// =============================================================================
// Graph Utils (TIGER-02 Mitigation)
// =============================================================================

/// Tracks visited nodes to detect cycles during import resolution
pub struct DefinitionCycleDetector {
    visited: HashSet<(PathBuf, String)>,
}

impl DefinitionCycleDetector {
    /// Create a new cycle detector
    pub fn new() -> Self {
        Self {
            visited: HashSet::new(),
        }
    }

    /// Visit a (file, symbol) pair. Returns true if already visited (cycle detected).
    pub fn visit(&mut self, file: &Path, symbol: &str) -> bool {
        let key = (file.to_path_buf(), symbol.to_string());
        !self.visited.insert(key)
    }
}

impl Default for DefinitionCycleDetector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Find symbol definition (go-to-definition)
///
/// Supports two modes:
/// 1. Position-based: Find symbol at file:line:column and jump to its definition
/// 2. Name-based: Find definition of a named symbol using --symbol and --file
///
/// # Example
///
/// ```bash
/// # Position mode
/// tldr definition src/main.py 10 5
///
/// # Name mode
/// tldr definition --symbol MyClass --file src/main.py
/// ```
#[derive(Debug, Args)]
pub struct DefinitionArgs {
    /// Source file (positional, for position-based lookup)
    pub file: Option<PathBuf>,

    /// line number (1-indexed, for position-based lookup)
    pub line: Option<u32>,

    /// column number (0-indexed, for position-based lookup)
    pub column: Option<u32>,

    /// Find symbol by name instead of position
    #[arg(long)]
    pub symbol: Option<String>,

    /// File to search in (used with --symbol)
    #[arg(long = "file", name = "target_file")]
    pub target_file: Option<PathBuf>,

    /// Project root for cross-file resolution
    #[arg(long)]
    pub project: Option<PathBuf>,

    /// Output file (optional, stdout if not specified)
    #[arg(long, short = 'O')]
    pub output: Option<PathBuf>,
}

impl DefinitionArgs {
    /// Run the definition command
    pub fn run(
        &self,
        format: crate::output::OutputFormat,
        quiet: bool,
        lang: Option<Language>,
    ) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Convert language option to string hint
        let lang_hint = match lang {
            Some(l) => format!("{:?}", l).to_lowercase(),
            None => "auto".to_string(),
        };

        // Determine which mode we're in
        let result = if let Some(ref symbol_name) = self.symbol {
            // Name-based mode - require --file
            let file = self.target_file.as_ref().ok_or_else(|| {
                RemainingError::invalid_argument("--file is required with --symbol")
            })?;

            writer.progress(&format!(
                "Finding definition of '{}' in {}...",
                symbol_name,
                file.display()
            ));

            find_definition_by_name(symbol_name, file, self.project.as_deref(), &lang_hint)?
        } else {
            // Position-based mode
            let file = self
                .file
                .as_ref()
                .ok_or_else(|| RemainingError::invalid_argument("file argument is required"))?;
            let line = self
                .line
                .ok_or_else(|| RemainingError::invalid_argument("line argument is required"))?;
            let column = self
                .column
                .ok_or_else(|| RemainingError::invalid_argument("column argument is required"))?;

            writer.progress(&format!(
                "Finding definition at {}:{}:{}...",
                file.display(),
                line,
                column
            ));

            match find_definition_by_position(
                file,
                line,
                column,
                self.project.as_deref(),
                &lang_hint,
            ) {
                Ok(result) => result,
                Err(_) => {
                    // Return a graceful "not found" result instead of failing
                    DefinitionResult {
                        symbol: SymbolInfo {
                            name: format!("<unknown at {}:{}:{}>", file.display(), line, column),
                            kind: SymbolKind::Variable,
                            location: Some(Location::with_column(
                                file.display().to_string(),
                                line,
                                column,
                            )),
                            type_annotation: None,
                            docstring: None,
                            is_builtin: false,
                            module: None,
                        },
                        definition: None,
                        type_definition: None,
                    }
                }
            }
        };

        // Determine output format
        let use_text = format == crate::output::OutputFormat::Text;

        // Write output
        if let Some(ref output_path) = self.output {
            if use_text {
                let text = format_definition_text(&result);
                fs::write(output_path, text)?;
            } else {
                let json = serde_json::to_string_pretty(&result)?;
                fs::write(output_path, json)?;
            }
        } else if use_text {
            let text = format_definition_text(&result);
            writer.write_text(&text)?;
        } else {
            writer.write(&result)?;
        }

        Ok(())
    }
}

// =============================================================================
// Core Functions
// =============================================================================

/// Find definition by symbol name
pub fn find_definition_by_name(
    symbol: &str,
    file: &Path,
    project: Option<&Path>,
    lang_hint: &str,
) -> RemainingResult<DefinitionResult> {
    // Validate file exists
    if !file.exists() {
        return Err(RemainingError::file_not_found(file));
    }

    // Detect language
    let language = detect_language(file, lang_hint)?;

    // Only Python is supported currently
    if language != Language::Python {
        return Err(RemainingError::unsupported_language(format!(
            "{:?}",
            language
        )));
    }

    // Check if it's a builtin
    if is_builtin(symbol, &language) {
        return Ok(DefinitionResult {
            symbol: SymbolInfo {
                name: symbol.to_string(),
                kind: SymbolKind::Function,
                location: None,
                type_annotation: None,
                docstring: None,
                is_builtin: true,
                module: Some("builtins".to_string()),
            },
            definition: None,
            type_definition: None,
        });
    }

    // Read and parse file
    let source = fs::read_to_string(file).map_err(RemainingError::Io)?;

    // Try to find the symbol in this file first
    if let Some(result) = find_symbol_in_file(symbol, file, &source)? {
        return Ok(result);
    }

    // If not found and we have a project context, try cross-file resolution
    if let Some(project_root) = project {
        let mut detector = DefinitionCycleDetector::new();
        if let Some(result) = resolve_cross_file(symbol, file, project_root, &mut detector, 0)? {
            return Ok(result);
        }
    }

    Err(RemainingError::symbol_not_found(symbol, file))
}

/// Find definition by position (line, column)
pub fn find_definition_by_position(
    file: &Path,
    line: u32,
    column: u32,
    project: Option<&Path>,
    lang_hint: &str,
) -> RemainingResult<DefinitionResult> {
    // Validate file exists
    if !file.exists() {
        return Err(RemainingError::file_not_found(file));
    }

    // Detect language
    let language = detect_language(file, lang_hint)?;

    // Only Python is supported currently
    if language != Language::Python {
        return Err(RemainingError::unsupported_language(format!(
            "{:?}",
            language
        )));
    }

    // Read and parse file
    let source = fs::read_to_string(file).map_err(RemainingError::Io)?;

    // Find symbol at position
    let symbol_name = find_symbol_at_position(&source, line, column)?;

    // Now find definition of that symbol
    find_definition_by_name(&symbol_name, file, project, lang_hint)
}

/// Find symbol name at a given position
fn find_symbol_at_position(source: &str, line: u32, column: u32) -> RemainingResult<String> {
    let mut parser = Parser::new();
    parser
        .set_language(&PYTHON_LANGUAGE.into())
        .map_err(|e| RemainingError::parse_error(PathBuf::from("<input>"), e.to_string()))?;

    let tree = parser.parse(source, None).ok_or_else(|| {
        RemainingError::parse_error(PathBuf::from("<input>"), "Failed to parse".to_string())
    })?;

    // Convert 1-indexed line to 0-indexed
    let target_line = line.saturating_sub(1) as usize;
    let target_col = column as usize;

    // Find the node at the position
    let root = tree.root_node();
    let point = tree_sitter::Point::new(target_line, target_col);

    let node = root
        .descendant_for_point_range(point, point)
        .ok_or_else(|| {
            RemainingError::invalid_argument(format!(
                "No symbol found at line {}, column {}",
                line, column
            ))
        })?;

    // Get the identifier
    let text = node.utf8_text(source.as_bytes()).map_err(|_| {
        RemainingError::parse_error(PathBuf::from("<input>"), "Invalid UTF-8".to_string())
    })?;

    // If this is an identifier, return it
    if node.kind() == "identifier" || node.kind() == "property_identifier" {
        return Ok(text.to_string());
    }

    // Try parent nodes to find an identifier
    let mut current = Some(node);
    while let Some(n) = current {
        if n.kind() == "identifier" || n.kind() == "property_identifier" {
            let text = n.utf8_text(source.as_bytes()).map_err(|_| {
                RemainingError::parse_error(PathBuf::from("<input>"), "Invalid UTF-8".to_string())
            })?;
            return Ok(text.to_string());
        }
        current = n.parent();
    }

    // Return what we found
    Ok(text.to_string())
}

/// Find a symbol definition within a single file
fn find_symbol_in_file(
    symbol: &str,
    file: &Path,
    source: &str,
) -> RemainingResult<Option<DefinitionResult>> {
    let mut parser = Parser::new();
    parser
        .set_language(&PYTHON_LANGUAGE.into())
        .map_err(|e| RemainingError::parse_error(file.to_path_buf(), e.to_string()))?;

    let tree = parser.parse(source, None).ok_or_else(|| {
        RemainingError::parse_error(file.to_path_buf(), "Failed to parse".to_string())
    })?;

    let root = tree.root_node();

    // Search for function/class/method definitions using tree traversal
    if let Some((kind, location)) = find_definition_recursive(root, source, symbol, file) {
        return Ok(Some(DefinitionResult {
            symbol: SymbolInfo {
                name: symbol.to_string(),
                kind,
                location: Some(location.clone()),
                type_annotation: None,
                docstring: None,
                is_builtin: false,
                module: None,
            },
            definition: Some(location),
            type_definition: None,
        }));
    }

    Ok(None)
}

/// Recursively search the AST for a definition
fn find_definition_recursive(
    node: Node,
    source: &str,
    target_name: &str,
    file: &Path,
) -> Option<(SymbolKind, Location)> {
    match node.kind() {
        "function_definition" => {
            // Get the name child
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    if name == target_name {
                        // Check if inside a class by looking at parents
                        let in_class = is_inside_class(node);
                        let kind = if in_class {
                            SymbolKind::Method
                        } else {
                            SymbolKind::Function
                        };
                        let location = Location::with_column(
                            file.display().to_string(),
                            name_node.start_position().row as u32 + 1,
                            name_node.start_position().column as u32,
                        );
                        return Some((kind, location));
                    }
                }
            }
        }
        "class_definition" => {
            // Get the name child
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    if name == target_name {
                        let location = Location::with_column(
                            file.display().to_string(),
                            name_node.start_position().row as u32 + 1,
                            name_node.start_position().column as u32,
                        );
                        return Some((SymbolKind::Class, location));
                    }
                }
            }
        }
        "assignment" => {
            // Check for variable assignments at module level
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "identifier" {
                    if let Ok(name) = left.utf8_text(source.as_bytes()) {
                        if name == target_name {
                            let location = Location::with_column(
                                file.display().to_string(),
                                left.start_position().row as u32 + 1,
                                left.start_position().column as u32,
                            );
                            return Some((SymbolKind::Variable, location));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    // Search children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(result) = find_definition_recursive(child, source, target_name, file) {
                return Some(result);
            }
        }
    }

    None
}

/// Check if a node is inside a class definition
fn is_inside_class(node: Node) -> bool {
    let mut current = node.parent();
    while let Some(n) = current {
        if n.kind() == "class_definition" {
            return true;
        }
        current = n.parent();
    }
    false
}

/// Resolve symbol across files via imports
fn resolve_cross_file(
    symbol: &str,
    current_file: &Path,
    project_root: &Path,
    detector: &mut DefinitionCycleDetector,
    depth: usize,
) -> RemainingResult<Option<DefinitionResult>> {
    // Prevent infinite recursion
    if depth >= MAX_IMPORT_DEPTH {
        return Ok(None);
    }

    // Check for cycle
    if detector.visit(current_file, symbol) {
        return Ok(None);
    }

    // Read current file
    let source = fs::read_to_string(current_file).map_err(RemainingError::Io)?;

    // Find imports in the current file and check if symbol is imported
    let imports = extract_imports(&source);

    for (module_path, imported_names) in imports {
        // Check if our symbol is imported from this module
        let is_imported = imported_names.is_empty() // Star import or module import
            || imported_names.contains(&symbol.to_string());

        if is_imported {
            // Resolve module path to file path
            if let Some(resolved_path) =
                resolve_module_path(&module_path, current_file, project_root)
            {
                if resolved_path.exists() {
                    let module_source =
                        fs::read_to_string(&resolved_path).map_err(RemainingError::Io)?;

                    if let Some(result) =
                        find_symbol_in_file(symbol, &resolved_path, &module_source)?
                    {
                        return Ok(Some(result));
                    }

                    // Recursively check imports in that file
                    if let Some(result) = resolve_cross_file(
                        symbol,
                        &resolved_path,
                        project_root,
                        detector,
                        depth + 1,
                    )? {
                        return Ok(Some(result));
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Extract import statements from source code
fn extract_imports(source: &str) -> Vec<(String, Vec<String>)> {
    let mut imports = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("from ") {
            if let Some(import_idx) = line.find(" import ") {
                let module = &line[5..import_idx];
                let names_str = &line[import_idx + 8..];
                let names: Vec<String> = names_str
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .split(" as ")
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty() && s != "*")
                    .collect();
                imports.push((module.trim().to_string(), names));
            }
        } else if let Some(module) = line.strip_prefix("import ") {
            let module = module.split(" as ").next().unwrap_or(module).trim();
            imports.push((module.to_string(), Vec::new()));
        }
    }

    imports
}

/// Resolve a module path to a file path
///
/// Handles both absolute imports (`os.path`) and relative imports (`.utils`, `..pkg.mod`).
/// For relative imports, leading dots indicate the number of parent directories to traverse
/// from the current file's location (1 dot = same package, 2 dots = parent, etc.).
fn resolve_module_path(module: &str, current_file: &Path, project_root: &Path) -> Option<PathBuf> {
    let current_dir = current_file.parent()?;

    // Count leading dots for relative imports
    let dot_count = module.chars().take_while(|&c| c == '.').count();

    if dot_count > 0 {
        // Relative import: strip the leading dots and resolve relative to current package
        let remainder = &module[dot_count..];

        // Navigate up (dot_count - 1) directories from the current file's directory.
        // 1 dot  = same directory as current file
        // 2 dots = parent directory
        // 3 dots = grandparent directory, etc.
        let mut base = current_dir.to_path_buf();
        for _ in 1..dot_count {
            base = base.parent()?.to_path_buf();
        }

        if remainder.is_empty() {
            // "from . import X" - resolve to __init__.py in current package
            let pkg_candidate = base.join("__init__.py");
            if pkg_candidate.exists() {
                return Some(pkg_candidate);
            }
            return None;
        }

        // Convert remaining dotted path to filesystem path
        let rel_path = remainder.replace('.', "/");

        // Try as a module file
        let candidate = base.join(&rel_path).with_extension("py");
        if candidate.exists() {
            return Some(candidate);
        }

        // Try as a package directory
        let pkg_candidate = base.join(&rel_path).join("__init__.py");
        if pkg_candidate.exists() {
            return Some(pkg_candidate);
        }

        return None;
    }

    // Absolute import: try relative to current directory first, then project root
    let rel_path = module.replace('.', "/");

    // Try relative to current file's directory
    let candidate = current_dir.join(&rel_path).with_extension("py");
    if candidate.exists() {
        return Some(candidate);
    }

    // Try as package
    let pkg_candidate = current_dir.join(&rel_path).join("__init__.py");
    if pkg_candidate.exists() {
        return Some(pkg_candidate);
    }

    // Try relative to project root
    let candidate = project_root.join(&rel_path).with_extension("py");
    if candidate.exists() {
        return Some(candidate);
    }

    let pkg_candidate = project_root.join(&rel_path).join("__init__.py");
    if pkg_candidate.exists() {
        return Some(pkg_candidate);
    }

    None
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Check if a symbol is a language builtin
pub fn is_builtin(name: &str, language: &Language) -> bool {
    match language {
        Language::Python => PYTHON_BUILTINS.contains(&name),
        _ => false,
    }
}

/// Detect language from file extension or hint
fn detect_language(file: &Path, hint: &str) -> RemainingResult<Language> {
    if hint != "auto" {
        return match hint.to_lowercase().as_str() {
            "python" | "py" => Ok(Language::Python),
            "typescript" | "ts" => Ok(Language::TypeScript),
            "javascript" | "js" => Ok(Language::JavaScript),
            "rust" | "rs" => Ok(Language::Rust),
            "go" | "golang" => Ok(Language::Go),
            _ => Err(RemainingError::unsupported_language(hint)),
        };
    }

    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "py" => Ok(Language::Python),
        "ts" | "tsx" => Ok(Language::TypeScript),
        "js" | "jsx" => Ok(Language::JavaScript),
        "rs" => Ok(Language::Rust),
        "go" => Ok(Language::Go),
        _ => Err(RemainingError::unsupported_language(ext)),
    }
}

/// Format definition result as text
fn format_definition_text(result: &DefinitionResult) -> String {
    let mut output = String::new();

    output.push_str("=== Definition Result ===\n\n");
    output.push_str(&format!("Symbol: {}\n", result.symbol.name));
    output.push_str(&format!("Kind: {:?}\n", result.symbol.kind));

    if result.symbol.is_builtin {
        output.push_str("Type: Built-in\n");
        if let Some(ref module) = result.symbol.module {
            output.push_str(&format!("Module: {}\n", module));
        }
    } else if let Some(ref location) = result.definition {
        output.push_str("\nDefinition Location:\n");
        output.push_str(&format!("  File: {}\n", location.file));
        output.push_str(&format!("  Line: {}\n", location.line));
        if location.column > 0 {
            output.push_str(&format!("  Column: {}\n", location.column));
        }
    } else {
        output.push_str("\nDefinition: Not found\n");
    }

    if let Some(ref type_def) = result.type_definition {
        output.push_str("\nType Definition:\n");
        output.push_str(&format!("  File: {}\n", type_def.file));
        output.push_str(&format!("  Line: {}\n", type_def.line));
    }

    if let Some(ref docstring) = result.symbol.docstring {
        output.push_str(&format!("\nDocstring:\n  {}\n", docstring));
    }

    output
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_builtin_python() {
        assert!(is_builtin("len", &Language::Python));
        assert!(is_builtin("print", &Language::Python));
        assert!(is_builtin("range", &Language::Python));
        assert!(!is_builtin("my_func", &Language::Python));
    }

    #[test]
    fn test_cycle_detector() {
        let mut detector = DefinitionCycleDetector::new();

        // First visit should return false (not a cycle)
        assert!(!detector.visit(Path::new("file.py"), "symbol"));

        // Second visit to same location should return true (cycle)
        assert!(detector.visit(Path::new("file.py"), "symbol"));

        // Different location should return false
        assert!(!detector.visit(Path::new("other.py"), "symbol"));
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(
            detect_language(Path::new("test.py"), "auto").unwrap(),
            Language::Python
        );
    }

    #[test]
    fn test_detect_language_with_hint() {
        assert_eq!(
            detect_language(Path::new("test.txt"), "python").unwrap(),
            Language::Python
        );
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
from os import path, getcwd
from sys import argv
import json
import re as regex
"#;
        let imports = extract_imports(source);

        assert_eq!(imports.len(), 4);
        assert_eq!(imports[0].0, "os");
        assert!(imports[0].1.contains(&"path".to_string()));
        assert!(imports[0].1.contains(&"getcwd".to_string()));
        assert_eq!(imports[1].0, "sys");
        assert!(imports[1].1.contains(&"argv".to_string()));
        assert_eq!(imports[2].0, "json");
        assert_eq!(imports[3].0, "re");
    }

    #[test]
    fn test_extract_imports_relative() {
        let source = r#"
from .utils import echo, make_str
from .exceptions import Abort
from ._utils import FLAG_NEEDS_VALUE
from . import types
"#;
        let imports = extract_imports(source);

        assert_eq!(imports.len(), 4);
        // Relative imports should preserve the dot prefix
        assert_eq!(imports[0].0, ".utils");
        assert!(imports[0].1.contains(&"echo".to_string()));
        assert!(imports[0].1.contains(&"make_str".to_string()));
        assert_eq!(imports[1].0, ".exceptions");
        assert!(imports[1].1.contains(&"Abort".to_string()));
        assert_eq!(imports[2].0, "._utils");
        assert!(imports[2].1.contains(&"FLAG_NEEDS_VALUE".to_string()));
        assert_eq!(imports[3].0, ".");
        assert!(imports[3].1.contains(&"types".to_string()));
    }

    #[test]
    fn test_resolve_module_path_relative_import() {
        // Create a temp directory structure simulating a Python package
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("mypkg");
        fs::create_dir_all(&pkg).unwrap();

        // Create files
        fs::write(pkg.join("__init__.py"), "").unwrap();
        fs::write(pkg.join("core.py"), "from .utils import helper\n").unwrap();
        fs::write(pkg.join("utils.py"), "def helper(): pass\n").unwrap();

        let current_file = pkg.join("core.py");
        let project_root = dir.path();

        // Relative import ".utils" from core.py should resolve to utils.py in the same directory
        let resolved = resolve_module_path(".utils", &current_file, project_root);
        assert!(
            resolved.is_some(),
            "resolve_module_path should find .utils relative to core.py"
        );
        assert_eq!(
            resolved.unwrap(),
            pkg.join("utils.py"),
            "Should resolve to sibling utils.py"
        );
    }

    #[test]
    fn test_resolve_module_path_relative_import_subpackage() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("mypkg");
        let sub = pkg.join("sub");
        fs::create_dir_all(&sub).unwrap();

        fs::write(pkg.join("__init__.py"), "").unwrap();
        fs::write(sub.join("__init__.py"), "").unwrap();
        fs::write(pkg.join("core.py"), "").unwrap();
        fs::write(sub.join("helpers.py"), "def helper(): pass\n").unwrap();

        let current_file = pkg.join("core.py");
        let project_root = dir.path();

        // ".sub.helpers" from core.py should resolve to sub/helpers.py
        let resolved = resolve_module_path(".sub.helpers", &current_file, project_root);
        assert!(
            resolved.is_some(),
            "resolve_module_path should find .sub.helpers relative to core.py"
        );
        assert_eq!(
            resolved.unwrap(),
            sub.join("helpers.py"),
            "Should resolve to sub/helpers.py"
        );
    }

    #[test]
    fn test_cross_file_definition_via_relative_import() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("mypkg");
        fs::create_dir_all(&pkg).unwrap();

        fs::write(pkg.join("__init__.py"), "").unwrap();
        fs::write(
            pkg.join("core.py"),
            "from .utils import echo\n\ndef main():\n    echo('hello')\n",
        )
        .unwrap();
        fs::write(pkg.join("utils.py"), "def echo(msg):\n    print(msg)\n").unwrap();

        // Look for 'echo' starting from core.py with project context
        let result =
            find_definition_by_name("echo", &pkg.join("core.py"), Some(dir.path()), "python");

        assert!(
            result.is_ok(),
            "Should find echo via cross-file resolution: {:?}",
            result.err()
        );
        let result = result.unwrap();
        assert_eq!(result.symbol.name, "echo");
        assert_eq!(result.symbol.kind, SymbolKind::Function);
        assert!(
            result.definition.is_some(),
            "Should have a definition location"
        );
        let def_loc = result.definition.unwrap();
        assert!(
            def_loc.file.contains("utils.py"),
            "Definition should be in utils.py, got: {}",
            def_loc.file
        );
        assert_eq!(def_loc.line, 1, "echo is defined on line 1 of utils.py");
    }
}
