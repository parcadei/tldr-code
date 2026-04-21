//! Behavioral constraint extraction command
//!
//! Extracts behavioral specifications (pre/postconditions, exceptions, side effects)
//! from source code using tree-sitter for AST analysis. Supports all languages
//! with tree-sitter grammars.
//!
//! # Features
//!
//! - Guard clause detection (if x < 0: raise/throw/panic)
//! - Assertion extraction (assert, debug_assert, console.assert, etc.)
//! - Type hint/annotation extraction
//! - Docstring/doc-comment parsing (Google, NumPy, Sphinx, Javadoc, Rustdoc, Godoc)
//! - Exception/error detection (raise, throw, panic, return err)
//! - Side effect analysis (I/O, global writes, attribute mutations)
//! - LLM-ready constraint generation (--constraints flag)
//!
//! # Supported Languages
//!
//! Python, TypeScript, JavaScript, Go, Rust, Java, C, C++, Ruby, C#,
//! Scala, PHP, Lua, Luau, Elixir, OCaml, Kotlin, Swift
//!
//! # Usage
//!
//! ```bash
//! tldr behavioral file.py [function_name]
//! tldr behavioral file.go --constraints
//! tldr behavioral file.rs --format text
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::Args;
use tree_sitter::{Node, Parser};
use tldr_core::types::Language;

use super::error::{PatternsError, PatternsResult};
use super::types::{
    BehavioralReport, ClassBehavior, ClassInvariant, ConditionSource, DocstringStyle,
    EffectType, ExceptionInfo, FunctionBehavior, OutputFormat, Postcondition,
    Precondition, SideEffect, YieldInfo,
};
use super::validation::{read_file_safe, validate_file_path, validate_file_path_in_project};

// =============================================================================
// Constants
// =============================================================================

/// I/O operations that indicate impure functions (cross-language)
const IO_OPERATIONS: &[&str] = &[
    // Universal
    "print", "println", "printf", "fprintf", "sprintf", "open", "read", "write",
    "input", "readline", "readlines", "writelines", "flush", "close", "seek", "tell",
    // Go
    "fmt.Println", "fmt.Printf", "fmt.Fprintf", "os.Open", "os.Create",
    "io.ReadAll", "io.Copy", "bufio.NewReader", "bufio.NewWriter",
    // Rust
    "println!", "eprintln!", "print!", "eprint!",
    "std::fs::read", "std::fs::write", "std::fs::File::open",
    // Java/C#
    "System.out.println", "System.out.print", "Console.WriteLine", "Console.Write",
    // JS/TS
    "console.log", "console.error", "console.warn",
    "fs.readFile", "fs.writeFile", "fs.readFileSync", "fs.writeFileSync",
    // Ruby
    "puts", "p", "pp",
    // PHP
    "echo", "var_dump", "print_r", "fopen", "fclose", "fread", "fwrite",
];

/// Known impure function calls (cross-language)
const IMPURE_CALLS: &[&str] = &[
    // Python
    "random.random", "random.randint", "random.choice", "random.shuffle",
    "time.time", "time.sleep", "datetime.now", "datetime.today",
    "os.system", "os.popen", "subprocess.run", "subprocess.call",
    "requests.get", "requests.post", "urllib.request.urlopen",
    // Go
    "rand.Int", "rand.Intn", "time.Now", "time.Sleep",
    "http.Get", "http.Post", "exec.Command",
    // Rust
    "std::thread::sleep", "reqwest::get",
    // JS/TS
    "fetch", "setTimeout", "setInterval", "Math.random",
    "Date.now",
    // Java
    "Math.random", "Thread.sleep", "Runtime.exec",
    // Ruby
    "sleep", "rand", "system", "exec",
];

/// Collection mutation methods (cross-language)
const COLLECTION_MUTATIONS: &[&str] = &[
    // Python
    "append", "extend", "insert", "remove", "pop", "clear",
    "sort", "reverse", "update", "setdefault", "popitem",
    "add", "discard", "difference_update", "intersection_update",
    "symmetric_difference_update",
    // JS/TS/Java/C#
    "push", "splice", "shift", "unshift", "fill",
    // Ruby
    "push", "unshift", "delete", "reject!", "select!", "sort!", "reverse!",
    // Go (typically direct assignment, but some patterns)
    // Rust
    "push_back", "push_front", "retain", "truncate",
];

// =============================================================================
// Language Configuration
// =============================================================================

/// Language-specific AST node kind configuration for behavioral analysis
struct LangConfig {
    /// Node kinds that represent function definitions
    function_kinds: &'static [&'static str],
    /// Node kinds that represent class/struct/impl definitions
    class_kinds: &'static [&'static str],
    /// Node kinds for raise/throw/panic statements
    raise_kinds: &'static [&'static str],
    /// Node kinds for if statements
    if_kinds: &'static [&'static str],
    /// Node kinds for assert statements/calls
    assert_kinds: &'static [&'static str],
    /// Node kinds for assignment
    assignment_kinds: &'static [&'static str],
    /// Node kind for function body (field name to look up, or node kind to match)
    body_field: &'static str,
    /// Node kind for parameters
    params_field: &'static str,
    /// Keyword for self/this
    self_keyword: &'static str,
    /// Node kinds for global/module-scope state mutation
    global_kinds: &'static [&'static str],
    /// Whether the language uses async function definitions as separate node kinds
    async_function_kinds: &'static [&'static str],
    /// Node kinds for yield/generator expressions
    yield_kinds: &'static [&'static str],
    /// Node kinds for comments (doc comments)
    comment_kinds: &'static [&'static str],
}

fn get_lang_config(lang: Language) -> LangConfig {
    match lang {
        Language::Python => LangConfig {
            function_kinds: &["function_definition", "async_function_definition"],
            class_kinds: &["class_definition"],
            raise_kinds: &["raise_statement"],
            if_kinds: &["if_statement"],
            assert_kinds: &["assert_statement"],
            assignment_kinds: &["assignment", "augmented_assignment"],
            body_field: "block",
            params_field: "parameters",
            self_keyword: "self",
            global_kinds: &["global_statement"],
            async_function_kinds: &["async_function_definition"],
            yield_kinds: &["yield", "yield_statement"],
            comment_kinds: &["comment"],
        },
        Language::TypeScript | Language::JavaScript => LangConfig {
            function_kinds: &["function_declaration", "arrow_function", "method_definition", "function"],
            class_kinds: &["class_declaration"],
            raise_kinds: &["throw_statement"],
            if_kinds: &["if_statement"],
            assert_kinds: &[], // JS uses console.assert() - handled as call
            assignment_kinds: &["assignment_expression", "augmented_assignment_expression"],
            body_field: "body",
            params_field: "formal_parameters",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &["yield_expression"],
            comment_kinds: &["comment"],
        },
        Language::Go => LangConfig {
            function_kinds: &["function_declaration", "method_declaration"],
            class_kinds: &["type_declaration"],
            raise_kinds: &[], // Go uses panic() - handled as call
            if_kinds: &["if_statement"],
            assert_kinds: &[], // Go doesn't have assert keyword
            assignment_kinds: &["assignment_statement", "short_var_declaration"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
        Language::Rust => LangConfig {
            function_kinds: &["function_item"],
            class_kinds: &["struct_item", "impl_item", "enum_item"],
            raise_kinds: &[], // Rust uses panic!() macro or return Err - handled separately
            if_kinds: &["if_expression"],
            assert_kinds: &[], // assert!, debug_assert! are macro invocations
            assignment_kinds: &["assignment_expression", "compound_assignment_expr"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["line_comment", "block_comment"],
        },
        Language::Java => LangConfig {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &["class_declaration", "interface_declaration", "enum_declaration"],
            raise_kinds: &["throw_statement"],
            if_kinds: &["if_statement"],
            assert_kinds: &["assert_statement"],
            assignment_kinds: &["assignment_expression"],
            body_field: "body",
            params_field: "formal_parameters",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["line_comment", "block_comment"],
        },
        Language::C => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["struct_specifier"],
            raise_kinds: &[], // C has no exceptions
            if_kinds: &["if_statement"],
            assert_kinds: &[], // assert() is a macro call
            assignment_kinds: &["assignment_expression"],
            body_field: "body",
            params_field: "declarator", // C uses declarator -> parameter_list
            self_keyword: "",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
        Language::Cpp => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["class_specifier", "struct_specifier"],
            raise_kinds: &["throw_statement"],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            assignment_kinds: &["assignment_expression"],
            body_field: "body",
            params_field: "declarator",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &["co_yield_statement"],
            comment_kinds: &["comment"],
        },
        Language::Ruby => LangConfig {
            function_kinds: &["method", "singleton_method"],
            class_kinds: &["class", "module"],
            raise_kinds: &[], // Ruby uses raise/fail as method calls
            if_kinds: &["if", "unless"],
            assert_kinds: &[],
            assignment_kinds: &["assignment"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &["yield"],
            comment_kinds: &["comment"],
        },
        Language::CSharp => LangConfig {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &["class_declaration", "interface_declaration", "struct_declaration"],
            raise_kinds: &["throw_statement"],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            assignment_kinds: &["assignment_expression"],
            body_field: "body",
            params_field: "parameter_list",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &["yield_statement"],
            comment_kinds: &["comment"],
        },
        Language::Php => LangConfig {
            function_kinds: &["function_definition", "method_declaration"],
            class_kinds: &["class_declaration", "interface_declaration"],
            raise_kinds: &["throw_expression"],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            assignment_kinds: &["assignment_expression", "augmented_assignment_expression"],
            body_field: "body",
            params_field: "formal_parameters",
            self_keyword: "$this",
            global_kinds: &["global_declaration"],
            async_function_kinds: &[],
            yield_kinds: &["yield_expression"],
            comment_kinds: &["comment"],
        },
        Language::Scala => LangConfig {
            function_kinds: &["function_definition", "function_declaration"],
            class_kinds: &["class_definition", "object_definition", "trait_definition"],
            raise_kinds: &["throw_expression"],
            if_kinds: &["if_expression"],
            assert_kinds: &[],
            assignment_kinds: &["assignment_expression"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &["yield"],
            comment_kinds: &["comment"],
        },
        Language::Kotlin => LangConfig {
            function_kinds: &["function_declaration"],
            class_kinds: &["class_declaration", "object_declaration"],
            raise_kinds: &["throw_expression"],
            if_kinds: &["if_expression"],
            assert_kinds: &[],
            assignment_kinds: &["assignment"],
            body_field: "function_body",
            params_field: "function_value_parameters",
            self_keyword: "this",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["line_comment", "multiline_comment"],
        },
        Language::Lua | Language::Luau => LangConfig {
            function_kinds: &["function_declaration", "function_definition"],
            class_kinds: &[],
            raise_kinds: &[], // Lua uses error() call
            if_kinds: &["if_statement"],
            assert_kinds: &[], // assert() is a function call
            assignment_kinds: &["assignment_statement"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
        Language::Elixir => LangConfig {
            function_kinds: &["call"], // def/defp are calls in Elixir AST
            class_kinds: &["call"],    // defmodule is a call
            raise_kinds: &[],          // raise is a function call
            if_kinds: &["call"],       // if/unless are macros (calls)
            assert_kinds: &[],
            assignment_kinds: &["binary_operator"], // = is a binary operator
            body_field: "do_block",
            params_field: "arguments",
            self_keyword: "",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
        Language::Ocaml => LangConfig {
            function_kinds: &["let_binding", "value_definition"],
            class_kinds: &["class_definition", "module_definition"],
            raise_kinds: &[], // raise is a function application
            if_kinds: &["if_expression"],
            assert_kinds: &[],
            assignment_kinds: &[],
            body_field: "body",
            params_field: "parameter",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
        Language::Swift => LangConfig {
            function_kinds: &["function_declaration"],
            class_kinds: &["class_declaration", "struct_declaration", "protocol_declaration", "enum_declaration"],
            raise_kinds: &[], // Swift uses fatalError()/precondition() as calls, not raise statements
            if_kinds: &["if_statement", "guard_statement"],
            assert_kinds: &[], // assert() is a function call
            assignment_kinds: &["assignment", "directly_assignable_expression"],
            body_field: "body",
            params_field: "parameter",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment", "multiline_comment"],
        },
        _ => LangConfig {
            function_kinds: &["function_definition"],
            class_kinds: &["class_definition"],
            raise_kinds: &[],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            assignment_kinds: &["assignment"],
            body_field: "body",
            params_field: "parameters",
            self_keyword: "self",
            global_kinds: &[],
            async_function_kinds: &[],
            yield_kinds: &[],
            comment_kinds: &["comment"],
        },
    }
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Arguments for the behavioral command
#[derive(Debug, Clone, Args)]
pub struct BehavioralArgs {
    /// Path to source file to analyze
    #[arg(required = true)]
    pub file: PathBuf,

    /// Specific function to analyze (optional, analyzes all if not provided)
    #[arg()]
    pub function: Option<String>,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(long = "output", short = 'o', hide = true, default_value = "json", value_enum)]
    pub output_format: OutputFormat,

    /// Generate LLM-ready constraints output
    #[arg(long)]
    pub constraints: bool,

    /// Project root for path validation
    #[arg(long)]
    pub project_root: Option<PathBuf>,
}

// =============================================================================
// Tree-sitter Parser
// =============================================================================

/// Get a tree-sitter parser for Python (legacy helper for backward compatibility)
fn get_python_parser() -> PatternsResult<Parser> {
    get_parser(Language::Python)
}

/// Get a tree-sitter parser for the given language
fn get_parser(lang: Language) -> PatternsResult<Parser> {
    let mut parser = Parser::new();
    let ts_lang = get_ts_language(lang)
        .ok_or_else(|| PatternsError::UnsupportedLanguage {
            language: format!("{:?}", lang),
        })?;
    parser
        .set_language(&ts_lang)
        .map_err(|e| PatternsError::parse_error(Path::new("<parser>"), &format!("Failed to set language: {}", e)))?;
    Ok(parser)
}

/// Get tree-sitter Language for a tldr Language
fn get_ts_language(lang: Language) -> Option<tree_sitter::Language> {
    match lang {
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::JavaScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Go => Some(tree_sitter_go::LANGUAGE.into()),
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::Java => Some(tree_sitter_java::LANGUAGE.into()),
        Language::C => Some(tree_sitter_c::LANGUAGE.into()),
        Language::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
        Language::Ruby => Some(tree_sitter_ruby::LANGUAGE.into()),
        Language::CSharp => Some(tree_sitter_c_sharp::LANGUAGE.into()),
        Language::Scala => Some(tree_sitter_scala::LANGUAGE.into()),
        Language::Php => Some(tree_sitter_php::LANGUAGE_PHP.into()),
        Language::Lua => Some(tree_sitter_lua::LANGUAGE.into()),
        Language::Luau => Some(tree_sitter_luau::LANGUAGE.into()),
        Language::Elixir => Some(tree_sitter_elixir::LANGUAGE.into()),
        Language::Ocaml => Some(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
        Language::Kotlin => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
        Language::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
        _ => None,
    }
}

/// Extract text from a tree-sitter node
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Get the line number of a node (1-indexed)
fn get_line_number(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

// =============================================================================
// Docstring Style Detection
// =============================================================================

/// Detect the docstring style from source content
pub fn detect_docstring_style(source: &str) -> DocstringStyle {
    // Look for distinctive patterns

    // NumPy style: section headers with dashes underneath
    if source.contains("\n    Parameters\n    ----------")
        || source.contains("\n    Returns\n    -------")
        || source.contains("\nParameters\n----------") {
        return DocstringStyle::Numpy;
    }

    // Sphinx/reST style: :param, :returns:, :raises:
    if source.contains(":param ") || source.contains(":returns:")
        || source.contains(":raises:") || source.contains(":type ") {
        return DocstringStyle::Sphinx;
    }

    // Google style: Args:, Returns:, Raises:, Yields:
    if source.contains("\n    Args:") || source.contains("\n    Returns:")
        || source.contains("\n    Raises:") || source.contains("\n    Yields:")
        || source.contains("\nArgs:") || source.contains("\nReturns:") {
        return DocstringStyle::Google;
    }

    DocstringStyle::Plain
}

// =============================================================================
// Import Detection
// =============================================================================

/// Check if source imports icontract library
pub fn has_icontract_import(source: &str) -> bool {
    source.contains("import icontract")
        || source.contains("from icontract")
        || source.contains("@icontract.")
        || source.contains("@require")
        || source.contains("@ensure")
}

/// Check if source imports deal library
pub fn has_deal_import(source: &str) -> bool {
    source.contains("import deal")
        || source.contains("from deal")
        || source.contains("@deal.")
}

// =============================================================================
// Precondition Extraction
// =============================================================================

/// Precondition extractor using tree-sitter AST
pub struct PreconditionExtractor<'a> {
    source: &'a [u8],
    param_names: HashSet<String>,
}

impl<'a> PreconditionExtractor<'a> {
    pub fn new(source: &'a [u8], param_names: HashSet<String>) -> Self {
        Self { source, param_names }
    }

    /// Extract preconditions from guard clauses (if x < 0: raise ...)
    pub fn extract_from_guard_clauses(&self, func_node: Node) -> Vec<Precondition> {
        let mut preconditions = Vec::new();
        let mut cursor = func_node.walk();

        // Find the function body
        for child in func_node.children(&mut cursor) {
            if child.kind() == "block" {
                self.extract_guards_from_block(child, &mut preconditions);
            }
        }

        preconditions
    }

    fn extract_guards_from_block(&self, block: Node, preconditions: &mut Vec<Precondition>) {
        let mut cursor = block.walk();

        for stmt in block.children(&mut cursor) {
            // Only look at first few statements (guard clauses should be at the start)
            if stmt.kind() == "if_statement" {
                if let Some(precond) = self.extract_guard_from_if(stmt) {
                    preconditions.push(precond);
                }
            } else if stmt.kind() != "expression_statement"
                && stmt.kind() != "pass_statement"
                && stmt.kind() != "comment" {
                // Stop at first non-guard statement (excluding docstrings, pass, comments)
                // Check if it's a docstring (string literal expression)
                if stmt.kind() == "expression_statement" {
                    if let Some(child) = stmt.child(0) {
                        if child.kind() != "string" {
                            break;
                        }
                    }
                }
            }
        }
    }

    fn extract_guard_from_if(&self, if_stmt: Node) -> Option<Precondition> {
        let mut cursor = if_stmt.walk();
        let mut condition_text = None;
        let mut has_raise = false;
        let mut param_found = None;

        for child in if_stmt.children(&mut cursor) {
            if child.kind() == "comparison_operator" || child.kind() == "not_operator"
                || child.kind() == "boolean_operator" || child.kind() == "parenthesized_expression" {
                condition_text = Some(node_text(child, self.source).to_string());
                // Check if condition references a parameter
                param_found = self.find_param_in_node(child);
            }

            if child.kind() == "block" {
                // Check if block contains a raise statement
                has_raise = self.block_has_raise(child);
            }
        }

        if has_raise {
            if let (Some(condition), Some(param)) = (condition_text, param_found) {
                // Negate the condition for the precondition
                // e.g., "if x < 0: raise" becomes precondition "x >= 0"
                let expression = self.negate_condition(&condition);
                return Some(Precondition {
                    param,
                    expression: Some(expression),
                    description: None,
                    type_hint: None,
                    source: ConditionSource::Guard,
                });
            }
        }

        None
    }

    fn block_has_raise(&self, block: Node) -> bool {
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            if child.kind() == "raise_statement" {
                return true;
            }
        }
        false
    }

    fn find_param_in_node(&self, node: Node) -> Option<String> {
        let mut cursor = node.walk();

        // Check if this node is an identifier that's a param
        if node.kind() == "identifier" {
            let name = node_text(node, self.source);
            if self.param_names.contains(name) {
                return Some(name.to_string());
            }
        }

        // Recurse into children
        for child in node.children(&mut cursor) {
            if let Some(param) = self.find_param_in_node(child) {
                return Some(param);
            }
        }

        None
    }

    fn negate_condition(&self, condition: &str) -> String {
        // Simple negation heuristics
        let condition = condition.trim();

        // Handle common comparison operators
        if condition.contains(" < ") {
            return condition.replace(" < ", " >= ");
        }
        if condition.contains(" <= ") {
            return condition.replace(" <= ", " > ");
        }
        if condition.contains(" > ") {
            return condition.replace(" > ", " <= ");
        }
        if condition.contains(" >= ") {
            return condition.replace(" >= ", " < ");
        }
        if condition.contains(" == ") {
            return condition.replace(" == ", " != ");
        }
        if condition.contains(" != ") {
            return condition.replace(" != ", " == ");
        }
        if condition.contains(" is None") {
            return condition.replace(" is None", " is not None");
        }
        if condition.contains(" is not None") {
            return condition.replace(" is not None", " is None");
        }
        if condition.starts_with("not ") {
            return condition[4..].to_string();
        }

        // Default: wrap with not
        format!("not ({})", condition)
    }

    /// Extract preconditions from assert statements
    pub fn extract_from_assertions(&self, func_node: Node) -> Vec<Precondition> {
        let mut preconditions = Vec::new();
        let mut cursor = func_node.walk();

        for child in func_node.children(&mut cursor) {
            if child.kind() == "block" {
                self.extract_asserts_from_block(child, &mut preconditions);
            }
        }

        preconditions
    }

    fn extract_asserts_from_block(&self, block: Node, preconditions: &mut Vec<Precondition>) {
        let mut cursor = block.walk();
        let mut stmt_count = 0;
        const MAX_ENTRY_STATEMENTS: usize = 10;

        for stmt in block.children(&mut cursor) {
            stmt_count += 1;
            if stmt_count > MAX_ENTRY_STATEMENTS {
                break;
            }

            if stmt.kind() == "assert_statement" {
                if let Some(precond) = self.extract_assert(stmt) {
                    preconditions.push(precond);
                }
            } else if stmt.kind() == "expression_statement" {
                // Allow docstrings
                continue;
            } else if stmt.kind() == "pass_statement" || stmt.kind() == "comment" {
                continue;
            } else if stmt.kind() != "assert_statement" {
                // Stop at first non-assert executable statement
                break;
            }
        }
    }

    fn extract_assert(&self, assert_stmt: Node) -> Option<Precondition> {
        let mut cursor = assert_stmt.walk();

        for child in assert_stmt.children(&mut cursor) {
            // Skip the "assert" keyword
            if child.kind() == "assert" {
                continue;
            }

            // Get the condition
            let condition = node_text(child, self.source).to_string();
            if let Some(param) = self.find_param_in_node(child) {
                return Some(Precondition {
                    param,
                    expression: Some(condition),
                    description: None,
                    type_hint: None,
                    source: ConditionSource::Assertion,
                });
            }
            break; // Only process first child after "assert"
        }

        None
    }

    /// Extract preconditions from type hints
    pub fn extract_from_type_hints(&self, func_node: Node) -> Vec<Precondition> {
        let mut preconditions = Vec::new();

        // Find parameters node
        let mut cursor = func_node.walk();
        for child in func_node.children(&mut cursor) {
            if child.kind() == "parameters" {
                self.extract_type_hints_from_params(child, &mut preconditions);
            }
        }

        preconditions
    }

    fn extract_type_hints_from_params(&self, params: Node, preconditions: &mut Vec<Precondition>) {
        let mut cursor = params.walk();

        for param in params.children(&mut cursor) {
            if param.kind() == "typed_parameter" || param.kind() == "typed_default_parameter" {
                let mut param_cursor = param.walk();
                let mut param_name = None;
                let mut type_hint = None;

                for child in param.children(&mut param_cursor) {
                    if child.kind() == "identifier" && param_name.is_none() {
                        param_name = Some(node_text(child, self.source).to_string());
                    }
                    if child.kind() == "type" {
                        type_hint = Some(node_text(child, self.source).to_string());
                    }
                }

                if let (Some(name), Some(hint)) = (param_name, type_hint) {
                    if self.param_names.contains(&name) {
                        preconditions.push(Precondition {
                            param: name,
                            expression: None,
                            description: None,
                            type_hint: Some(hint),
                            source: ConditionSource::TypeHint,
                        });
                    }
                }
            }
        }
    }

    /// Extract preconditions from docstrings
    pub fn extract_from_docstring(&self, docstring: &str, style: DocstringStyle) -> Vec<Precondition> {
        let mut preconditions = Vec::new();

        match style {
            DocstringStyle::Google => {
                self.extract_google_docstring_preconditions(docstring, &mut preconditions);
            }
            DocstringStyle::Numpy => {
                self.extract_numpy_docstring_preconditions(docstring, &mut preconditions);
            }
            DocstringStyle::Sphinx => {
                self.extract_sphinx_docstring_preconditions(docstring, &mut preconditions);
            }
            DocstringStyle::Plain => {
                // Plain docstrings don't have structured parameter descriptions
            }
        }

        preconditions
    }

    fn extract_google_docstring_preconditions(&self, docstring: &str, preconditions: &mut Vec<Precondition>) {
        // Find Args: section
        if let Some(args_start) = docstring.find("Args:") {
            let args_section = &docstring[args_start + 5..];
            // Find end of Args section (next section or end)
            let section_end = args_section
                .find("\n    Returns:")
                .or_else(|| args_section.find("\n    Raises:"))
                .or_else(|| args_section.find("\n    Yields:"))
                .or_else(|| args_section.find("\n    Note:"))
                .or_else(|| args_section.find("\n    Example"))
                .unwrap_or(args_section.len());

            let args_text = &args_section[..section_end];

            // Parse each parameter
            for line in args_text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Google style: "param_name: Description" or "param_name (type): Description"
                if let Some(colon_pos) = trimmed.find(':') {
                    let param_part = trimmed[..colon_pos].trim();
                    let description = trimmed[colon_pos + 1..].trim();

                    // Extract param name (may have type in parens)
                    let param_name = if let Some(paren_pos) = param_part.find('(') {
                        param_part[..paren_pos].trim()
                    } else {
                        param_part
                    };

                    if self.param_names.contains(param_name) {
                        preconditions.push(Precondition {
                            param: param_name.to_string(),
                            expression: None,
                            description: if description.is_empty() { None } else { Some(description.to_string()) },
                            type_hint: None,
                            source: ConditionSource::Docstring,
                        });
                    }
                }
            }
        }
    }

    fn extract_numpy_docstring_preconditions(&self, docstring: &str, preconditions: &mut Vec<Precondition>) {
        // Find Parameters section
        if let Some(params_start) = docstring.find("Parameters\n") {
            let after_header = &docstring[params_start + 11..];
            // Skip the dashes line
            if let Some(dashes_end) = after_header.find('\n') {
                let params_section = &after_header[dashes_end + 1..];

                // Find end of section
                let section_end = params_section
                    .find("\nReturns\n")
                    .or_else(|| params_section.find("\nRaises\n"))
                    .or_else(|| params_section.find("\nYields\n"))
                    .or_else(|| params_section.find("\nNotes\n"))
                    .or_else(|| params_section.find("\nExamples\n"))
                    .unwrap_or(params_section.len());

                let params_text = &params_section[..section_end];

                // Parse each parameter (name : type format)
                let mut current_param: Option<String> = None;
                let mut current_desc = String::new();

                for line in params_text.lines() {
                    if line.starts_with("    ") && !line.starts_with("        ") {
                        // New parameter line
                        // Save previous if exists
                        if let Some(ref param) = current_param {
                            if self.param_names.contains(param) {
                                preconditions.push(Precondition {
                                    param: param.clone(),
                                    expression: None,
                                    description: if current_desc.is_empty() { None } else { Some(current_desc.trim().to_string()) },
                                    type_hint: None,
                                    source: ConditionSource::Docstring,
                                });
                            }
                        }

                        let trimmed = line.trim();
                        if let Some(colon_pos) = trimmed.find(':') {
                            current_param = Some(trimmed[..colon_pos].trim().to_string());
                            current_desc = String::new();
                        }
                    } else if line.starts_with("        ") && current_param.is_some() {
                        // Description continuation
                        current_desc.push_str(line.trim());
                        current_desc.push(' ');
                    }
                }

                // Don't forget last parameter
                if let Some(ref param) = current_param {
                    if self.param_names.contains(param) {
                        preconditions.push(Precondition {
                            param: param.clone(),
                            expression: None,
                            description: if current_desc.is_empty() { None } else { Some(current_desc.trim().to_string()) },
                            type_hint: None,
                            source: ConditionSource::Docstring,
                        });
                    }
                }
            }
        }
    }

    fn extract_sphinx_docstring_preconditions(&self, docstring: &str, preconditions: &mut Vec<Precondition>) {
        // Parse :param name: description lines
        for line in docstring.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(":param ") {
                // :param name: description
                let after_param = &trimmed[7..];
                if let Some(colon_pos) = after_param.find(':') {
                    let param_name = after_param[..colon_pos].trim();
                    let description = after_param[colon_pos + 1..].trim();

                    if self.param_names.contains(param_name) {
                        preconditions.push(Precondition {
                            param: param_name.to_string(),
                            expression: None,
                            description: if description.is_empty() { None } else { Some(description.to_string()) },
                            type_hint: None,
                            source: ConditionSource::Docstring,
                        });
                    }
                }
            }
        }
    }
}

// =============================================================================
// Postcondition Extraction
// =============================================================================

/// Postcondition extractor
pub struct PostconditionExtractor<'a> {
    source: &'a [u8],
}

impl<'a> PostconditionExtractor<'a> {
    pub fn new(source: &'a [u8]) -> Self {
        Self { source }
    }

    /// Extract postconditions from return type hints
    pub fn extract_from_return_type(&self, func_node: Node) -> Vec<Postcondition> {
        let mut postconditions = Vec::new();
        let mut cursor = func_node.walk();

        for child in func_node.children(&mut cursor) {
            if child.kind() == "type" {
                let return_type = node_text(child, self.source).to_string();
                postconditions.push(Postcondition {
                    expression: None,
                    description: None,
                    type_hint: Some(return_type),
                });
            }
        }

        postconditions
    }

    /// Extract postconditions from docstring
    pub fn extract_from_docstring(&self, docstring: &str, style: DocstringStyle) -> Vec<Postcondition> {
        let mut postconditions = Vec::new();

        match style {
            DocstringStyle::Google => {
                if let Some(returns_start) = docstring.find("Returns:") {
                    let returns_section = &docstring[returns_start + 8..];
                    let section_end = returns_section
                        .find("\n    Raises:")
                        .or_else(|| returns_section.find("\n    Yields:"))
                        .or_else(|| returns_section.find("\n    Note:"))
                        .or_else(|| returns_section.find("\n    Example"))
                        .unwrap_or(returns_section.len());

                    let desc = returns_section[..section_end].trim();
                    if !desc.is_empty() {
                        postconditions.push(Postcondition {
                            expression: None,
                            description: Some(desc.to_string()),
                            type_hint: None,
                        });
                    }
                }
            }
            DocstringStyle::Numpy => {
                if let Some(returns_start) = docstring.find("Returns\n") {
                    let after_header = &docstring[returns_start + 8..];
                    if let Some(dashes_end) = after_header.find('\n') {
                        let returns_section = &after_header[dashes_end + 1..];
                        let section_end = returns_section
                            .find("\nRaises\n")
                            .or_else(|| returns_section.find("\nYields\n"))
                            .unwrap_or(returns_section.len());

                        let desc = returns_section[..section_end].trim();
                        if !desc.is_empty() {
                            postconditions.push(Postcondition {
                                expression: None,
                                description: Some(desc.to_string()),
                                type_hint: None,
                            });
                        }
                    }
                }
            }
            DocstringStyle::Sphinx => {
                for line in docstring.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(":returns:") || trimmed.starts_with(":return:") {
                        let desc = if trimmed.starts_with(":returns:") {
                            trimmed[9..].trim()
                        } else {
                            trimmed[8..].trim()
                        };
                        if !desc.is_empty() {
                            postconditions.push(Postcondition {
                                expression: None,
                                description: Some(desc.to_string()),
                                type_hint: None,
                            });
                        }
                    }
                }
            }
            DocstringStyle::Plain => {}
        }

        postconditions
    }

    /// Extract postconditions from assertions before return
    pub fn extract_from_assertions(&self, _func_node: Node) -> Vec<Postcondition> {
        // This would require more complex analysis to find assertions
        // that occur just before return statements
        Vec::new()
    }
}

// =============================================================================
// Exception Detection
// =============================================================================

/// Detect exceptions that may be raised by a function
pub fn detect_exceptions(func_node: Node, source: &[u8], docstring: Option<&str>, style: DocstringStyle) -> Vec<ExceptionInfo> {
    let mut exceptions = Vec::new();
    let mut seen_types: HashSet<String> = HashSet::new();

    // Extract from raise statements in the AST
    extract_raise_statements(func_node, source, &mut exceptions, &mut seen_types);

    // Extract from docstring
    if let Some(doc) = docstring {
        extract_docstring_exceptions(doc, style, &mut exceptions, &mut seen_types);
    }

    exceptions
}

fn extract_raise_statements(node: Node, source: &[u8], exceptions: &mut Vec<ExceptionInfo>, seen: &mut HashSet<String>) {
    let mut cursor = node.walk();

    if node.kind() == "raise_statement" {
        // Extract the exception type
        for child in node.children(&mut cursor) {
            if child.kind() == "call" {
                // raise ExceptionType(...)
                if let Some(func) = child.child_by_field_name("function") {
                    let exc_type = node_text(func, source).to_string();
                    if !seen.contains(&exc_type) {
                        seen.insert(exc_type.clone());
                        exceptions.push(ExceptionInfo {
                            exception_type: exc_type,
                            description: None,
                        });
                    }
                }
            } else if child.kind() == "identifier" {
                // raise ExceptionType
                let exc_type = node_text(child, source).to_string();
                if exc_type != "raise" && !seen.contains(&exc_type) {
                    seen.insert(exc_type.clone());
                    exceptions.push(ExceptionInfo {
                        exception_type: exc_type,
                        description: None,
                    });
                }
            }
        }
    }

    // Recurse into children
    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        extract_raise_statements(child, source, exceptions, seen);
    }
}

fn extract_docstring_exceptions(docstring: &str, style: DocstringStyle, exceptions: &mut Vec<ExceptionInfo>, seen: &mut HashSet<String>) {
    match style {
        DocstringStyle::Google => {
            if let Some(raises_start) = docstring.find("Raises:") {
                let raises_section = &docstring[raises_start + 7..];
                let section_end = raises_section
                    .find("\n    Yields:")
                    .or_else(|| raises_section.find("\n    Note:"))
                    .or_else(|| raises_section.find("\n    Example"))
                    .unwrap_or(raises_section.len());

                let raises_text = &raises_section[..section_end];

                for line in raises_text.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    if let Some(colon_pos) = trimmed.find(':') {
                        let exc_type = trimmed[..colon_pos].trim().to_string();
                        let description = trimmed[colon_pos + 1..].trim();

                        if !seen.contains(&exc_type) {
                            seen.insert(exc_type.clone());
                            exceptions.push(ExceptionInfo {
                                exception_type: exc_type,
                                description: if description.is_empty() { None } else { Some(description.to_string()) },
                            });
                        }
                    }
                }
            }
        }
        DocstringStyle::Numpy => {
            if let Some(raises_start) = docstring.find("Raises\n") {
                let after_header = &docstring[raises_start + 7..];
                if let Some(dashes_end) = after_header.find('\n') {
                    let raises_section = &after_header[dashes_end + 1..];
                    let section_end = raises_section
                        .find("\nYields\n")
                        .or_else(|| raises_section.find("\nNotes\n"))
                        .unwrap_or(raises_section.len());

                    let raises_text = &raises_section[..section_end];
                    let mut current_exc: Option<String> = None;
                    let mut current_desc = String::new();

                    for line in raises_text.lines() {
                        if !line.starts_with("    ") || line.starts_with("        ") {
                            if let Some(ref exc) = current_exc {
                                if !seen.contains(exc) {
                                    seen.insert(exc.clone());
                                    exceptions.push(ExceptionInfo {
                                        exception_type: exc.clone(),
                                        description: if current_desc.is_empty() { None } else { Some(current_desc.trim().to_string()) },
                                    });
                                }
                            }
                            current_exc = Some(line.trim().to_string());
                            current_desc = String::new();
                        } else {
                            current_desc.push_str(line.trim());
                            current_desc.push(' ');
                        }
                    }

                    if let Some(ref exc) = current_exc {
                        if !seen.contains(exc) {
                            seen.insert(exc.clone());
                            exceptions.push(ExceptionInfo {
                                exception_type: exc.clone(),
                                description: if current_desc.is_empty() { None } else { Some(current_desc.trim().to_string()) },
                            });
                        }
                    }
                }
            }
        }
        DocstringStyle::Sphinx => {
            for line in docstring.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with(":raises ") {
                    let after_raises = &trimmed[8..];
                    if let Some(colon_pos) = after_raises.find(':') {
                        let exc_type = after_raises[..colon_pos].trim().to_string();
                        let description = after_raises[colon_pos + 1..].trim();

                        if !seen.contains(&exc_type) {
                            seen.insert(exc_type.clone());
                            exceptions.push(ExceptionInfo {
                                exception_type: exc_type,
                                description: if description.is_empty() { None } else { Some(description.to_string()) },
                            });
                        }
                    }
                }
            }
        }
        DocstringStyle::Plain => {}
    }
}

// =============================================================================
// Side Effect Detection
// =============================================================================

/// Detect side effects in a function
pub fn detect_side_effects(func_node: Node, source: &[u8]) -> Vec<SideEffect> {
    let mut effects = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    detect_effects_recursive(func_node, source, &mut effects, &mut seen, 0);

    effects
}

fn detect_effects_recursive(node: Node, source: &[u8], effects: &mut Vec<SideEffect>, seen: &mut HashSet<String>, depth: usize) {
    // Prevent stack overflow
    if depth > 100 {
        return;
    }

    let mut cursor = node.walk();

    match node.kind() {
        "global_statement" => {
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(child, source).to_string();
                    let key = format!("global:{}", name);
                    if !seen.contains(&key) {
                        seen.insert(key);
                        effects.push(SideEffect {
                            effect_type: EffectType::GlobalWrite,
                            target: Some(name),
                        });
                    }
                }
            }
        }

        "assignment" | "augmented_assignment" => {
            // Check for attribute writes (self.x = ...)
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "attribute" {
                    if let Some(obj) = left.child_by_field_name("object") {
                        let obj_text = node_text(obj, source);
                        if obj_text == "self" {
                            if let Some(attr) = left.child_by_field_name("attribute") {
                                let attr_text = node_text(attr, source);
                                let target = format!("self.{}", attr_text);
                                let key = format!("attr:{}", target);
                                if !seen.contains(&key) {
                                    seen.insert(key);
                                    effects.push(SideEffect {
                                        effect_type: EffectType::AttributeWrite,
                                        target: Some(target),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        "call" => {
            let call_name = extract_call_name(node, source);
            if let Some(name) = call_name {
                // Check for I/O operations
                for &io_op in IO_OPERATIONS {
                    if name == io_op || name.ends_with(&format!(".{}", io_op)) {
                        let key = format!("io:{}", name);
                        if !seen.contains(&key) {
                            seen.insert(key);
                            effects.push(SideEffect {
                                effect_type: EffectType::Io,
                                target: Some(name.clone()),
                            });
                        }
                        break;
                    }
                }

                // Check for impure calls
                for &impure in IMPURE_CALLS {
                    if name == impure || name.ends_with(impure) {
                        let key = format!("call:{}", name);
                        if !seen.contains(&key) {
                            seen.insert(key);
                            effects.push(SideEffect {
                                effect_type: EffectType::Call,
                                target: Some(name.clone()),
                            });
                        }
                        break;
                    }
                }

                // Check for collection mutations
                let method_name = name.split('.').last().unwrap_or("");
                for &mutation in COLLECTION_MUTATIONS {
                    if method_name == mutation {
                        let key = format!("collection:{}", name);
                        if !seen.contains(&key) {
                            seen.insert(key);
                            effects.push(SideEffect {
                                effect_type: EffectType::CollectionModify,
                                target: Some(name.clone()),
                            });
                        }
                        break;
                    }
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    let mut child_cursor = node.walk();
    for child in node.children(&mut child_cursor) {
        detect_effects_recursive(child, source, effects, seen, depth + 1);
    }
}

fn extract_call_name(node: Node, source: &[u8]) -> Option<String> {
    if let Some(func) = node.child_by_field_name("function") {
        return Some(extract_name_from_expr(func, source));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => return Some(node_text(child, source).to_string()),
            "attribute" => return Some(extract_name_from_expr(child, source)),
            _ => continue,
        }
    }
    None
}

fn extract_name_from_expr(node: Node, source: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, source).to_string(),
        "attribute" => {
            let mut parts = Vec::new();
            let mut current = node;

            loop {
                if let Some(attr) = current.child_by_field_name("attribute") {
                    parts.push(node_text(attr, source).to_string());
                }

                if let Some(obj) = current.child_by_field_name("object") {
                    if obj.kind() == "attribute" {
                        current = obj;
                    } else if obj.kind() == "identifier" {
                        parts.push(node_text(obj, source).to_string());
                        break;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            parts.reverse();
            parts.join(".")
        }
        _ => node_text(node, source).to_string(),
    }
}

// =============================================================================
// Function Analysis
// =============================================================================

/// Extract parameter names from a function definition
fn extract_param_names(func_node: Node, source: &[u8]) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut cursor = func_node.walk();

    for child in func_node.children(&mut cursor) {
        if child.kind() == "parameters" {
            let mut param_cursor = child.walk();
            for param in child.children(&mut param_cursor) {
                match param.kind() {
                    "identifier" => {
                        let name = node_text(param, source);
                        if name != "self" && name != "cls" {
                            names.insert(name.to_string());
                        }
                    }
                    "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                        // Get the identifier child
                        let mut inner_cursor = param.walk();
                        for inner in param.children(&mut inner_cursor) {
                            if inner.kind() == "identifier" {
                                let name = node_text(inner, source);
                                if name != "self" && name != "cls" {
                                    names.insert(name.to_string());
                                }
                                break;
                            }
                        }
                    }
                    "list_splat_pattern" | "dictionary_splat_pattern" => {
                        // *args, **kwargs
                        let mut inner_cursor = param.walk();
                        for inner in param.children(&mut inner_cursor) {
                            if inner.kind() == "identifier" {
                                names.insert(node_text(inner, source).to_string());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    names
}

/// Get function name
fn get_function_name(func_node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return Some(node_text(child, source).to_string());
        }
    }
    None
}

/// Extract docstring from function
fn extract_docstring(func_node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = func_node.walk();

    for child in func_node.children(&mut cursor) {
        if child.kind() == "block" {
            let mut block_cursor = child.walk();
            for stmt in child.children(&mut block_cursor) {
                if stmt.kind() == "expression_statement" {
                    let mut expr_cursor = stmt.walk();
                    for expr in stmt.children(&mut expr_cursor) {
                        if expr.kind() == "string" {
                            let text = node_text(expr, source);
                            // Remove quotes
                            let content = text
                                .trim_start_matches("\"\"\"")
                                .trim_start_matches("'''")
                                .trim_start_matches('"')
                                .trim_start_matches('\'')
                                .trim_end_matches("\"\"\"")
                                .trim_end_matches("'''")
                                .trim_end_matches('"')
                                .trim_end_matches('\'');
                            return Some(content.to_string());
                        }
                    }
                }
                break; // Only check first statement
            }
        }
    }

    None
}

/// Check if function is a generator (contains yield)
fn is_generator(func_node: Node) -> bool {
    fn check_yield(node: Node, depth: usize) -> bool {
        if depth > 100 {
            return false;
        }

        if node.kind() == "yield" || node.kind() == "yield_statement" {
            return true;
        }

        // Don't descend into nested functions
        if node.kind() == "function_definition" || node.kind() == "lambda" {
            return false;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if check_yield(child, depth + 1) {
                return true;
            }
        }

        false
    }

    // Check the function body
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if child.kind() == "block" {
            return check_yield(child, 0);
        }
    }

    false
}

/// Analyze a single function
pub fn analyze_function(func_node: Node, source: &[u8], file_path: &str, style: DocstringStyle) -> Option<FunctionBehavior> {
    let name = get_function_name(func_node, source)?;
    let param_names = extract_param_names(func_node, source);
    let docstring = extract_docstring(func_node, source);
    let is_async = func_node.kind() == "async_function_definition";
    let is_gen = is_generator(func_node);

    // Extract preconditions
    let pre_extractor = PreconditionExtractor::new(source, param_names.clone());
    let mut preconditions = Vec::new();
    preconditions.extend(pre_extractor.extract_from_guard_clauses(func_node));
    preconditions.extend(pre_extractor.extract_from_assertions(func_node));
    preconditions.extend(pre_extractor.extract_from_type_hints(func_node));
    if let Some(ref doc) = docstring {
        preconditions.extend(pre_extractor.extract_from_docstring(doc, style));
    }

    // Extract postconditions
    let post_extractor = PostconditionExtractor::new(source);
    let mut postconditions = Vec::new();
    postconditions.extend(post_extractor.extract_from_return_type(func_node));
    if let Some(ref doc) = docstring {
        postconditions.extend(post_extractor.extract_from_docstring(doc, style));
    }

    // Extract exceptions
    let exceptions = detect_exceptions(func_node, source, docstring.as_deref(), style);

    // Detect side effects
    let side_effects = detect_side_effects(func_node, source);

    // Extract yield info (if generator)
    let yields = if is_gen {
        extract_yield_info(docstring.as_deref(), style)
    } else {
        Vec::new()
    };

    // Determine purity classification.
    // When no side effects are detected, we have no evidence either way —
    // absence of evidence is not evidence of purity. Classify as "unknown".
    let has_side_effects = side_effects.iter().any(|e| {
        !matches!(e.effect_type, EffectType::UnknownCall)
    });
    let has_unknown_calls = side_effects.iter().any(|e| {
        matches!(e.effect_type, EffectType::UnknownCall)
    });

    let purity_classification = if has_side_effects {
        "impure".to_string()
    } else if has_unknown_calls {
        "unknown".to_string()
    } else {
        // No effects detected at all. This could mean the function is truly pure,
        // or that the analysis didn't detect any effects (parser limitations,
        // unsupported call patterns, etc.). Without positive evidence, classify
        // conservatively as "unknown".
        "unknown".to_string()
    };

    Some(FunctionBehavior {
        function_name: name,
        file_path: file_path.to_string(),
        line: get_line_number(func_node),
        purity_classification,
        is_generator: is_gen,
        is_async,
        preconditions,
        postconditions,
        exceptions,
        yields,
        side_effects,
    })
}

fn extract_yield_info(docstring: Option<&str>, style: DocstringStyle) -> Vec<YieldInfo> {
    let mut yields = Vec::new();

    if let Some(doc) = docstring {
        match style {
            DocstringStyle::Google => {
                if let Some(yields_start) = doc.find("Yields:") {
                    let yields_section = &doc[yields_start + 7..];
                    let section_end = yields_section
                        .find("\n    Note:")
                        .or_else(|| yields_section.find("\n    Example"))
                        .unwrap_or(yields_section.len());

                    let desc = yields_section[..section_end].trim();
                    if !desc.is_empty() {
                        yields.push(YieldInfo {
                            type_hint: None,
                            description: Some(desc.to_string()),
                        });
                    }
                }
            }
            DocstringStyle::Numpy => {
                if let Some(yields_start) = doc.find("Yields\n") {
                    let after_header = &doc[yields_start + 7..];
                    if let Some(dashes_end) = after_header.find('\n') {
                        let yields_section = &after_header[dashes_end + 1..];
                        let section_end = yields_section
                            .find("\nNotes\n")
                            .or_else(|| yields_section.find("\nExamples\n"))
                            .unwrap_or(yields_section.len());

                        let desc = yields_section[..section_end].trim();
                        if !desc.is_empty() {
                            yields.push(YieldInfo {
                                type_hint: None,
                                description: Some(desc.to_string()),
                            });
                        }
                    }
                }
            }
            DocstringStyle::Sphinx => {
                for line in doc.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with(":yields:") {
                        let desc = trimmed[8..].trim();
                        if !desc.is_empty() {
                            yields.push(YieldInfo {
                                type_hint: None,
                                description: Some(desc.to_string()),
                            });
                        }
                    }
                }
            }
            DocstringStyle::Plain => {}
        }
    }

    yields
}

// =============================================================================
// Class Analysis
// =============================================================================

/// Analyze a class for behavioral constraints
pub fn analyze_class(class_node: Node, source: &[u8], file_path: &str, style: DocstringStyle) -> Option<ClassBehavior> {
    let mut cursor = class_node.walk();
    let mut class_name = None;
    let mut methods = Vec::new();
    let invariants = Vec::new(); // Would extract from icontract @invariant decorators

    for child in class_node.children(&mut cursor) {
        if child.kind() == "identifier" && class_name.is_none() {
            class_name = Some(node_text(child, source).to_string());
        }

        if child.kind() == "block" {
            let mut block_cursor = child.walk();
            for stmt in child.children(&mut block_cursor) {
                if stmt.kind() == "function_definition" || stmt.kind() == "async_function_definition" {
                    if let Some(func_behavior) = analyze_function(stmt, source, file_path, style) {
                        methods.push(func_behavior);
                    }
                }
            }
        }
    }

    class_name.map(|name| ClassBehavior {
        class_name: name,
        invariants,
        methods,
    })
}


// =============================================================================
// File Analysis
// =============================================================================

/// Analyze a source file for behavioral constraints (any supported language)
pub fn analyze_file(path: &Path, args: &BehavioralArgs) -> PatternsResult<BehavioralReport> {
    // Validate path
    let canonical = if let Some(ref root) = args.project_root {
        validate_file_path_in_project(path, root)?
    } else {
        validate_file_path(path)?
    };

    // Detect language from file extension
    let lang = Language::from_path(&canonical)
        .ok_or_else(|| PatternsError::UnsupportedLanguage {
            language: canonical.extension()
                .map_or("none".to_string(), |e| e.to_string_lossy().to_string()),
        })?;

    // Verify we have a tree-sitter grammar for this language
    if get_ts_language(lang).is_none() {
        return Err(PatternsError::UnsupportedLanguage {
            language: format!("{:?}", lang),
        });
    }

    // Read source
    let source = read_file_safe(&canonical)?;
    let source_bytes = source.as_bytes();

    // Detect docstring style (primarily for Python, also applies to doc comments)
    let style = detect_docstring_style(&source);

    // Check for DbC library imports (Python-specific, harmless for other langs)
    let has_icontract = has_icontract_import(&source);
    let has_deal = has_deal_import(&source);

    // Get language config
    let config = get_lang_config(lang);

    // Parse with tree-sitter
    let mut parser = get_parser(lang)?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| PatternsError::parse_error(&canonical, "Failed to parse file"))?;

    let root = tree.root_node();
    let file_path_str = canonical.to_string_lossy().to_string();

    // Collect functions and classes
    let mut functions = Vec::new();
    let mut classes = Vec::new();

    collect_behaviors_recursive(
        root, source_bytes, &file_path_str, style, lang, &config,
        &args.function, &mut functions, &mut classes, 0,
    );

    Ok(BehavioralReport {
        file_path: file_path_str,
        docstring_style: style,
        has_icontract,
        has_deal,
        functions,
        classes,
    })
}

/// Recursively collect function and class behaviors from the AST (multi-language)
fn collect_behaviors_recursive(
    node: Node,
    source: &[u8],
    file_path: &str,
    style: DocstringStyle,
    lang: Language,
    config: &LangConfig,
    filter_name: &Option<String>,
    functions: &mut Vec<FunctionBehavior>,
    classes: &mut Vec<ClassBehavior>,
    depth: usize,
) {
    if depth > 50 {
        return;
    }

    let kind = node.kind();

    // Check if this is a function definition
    let is_function = config.function_kinds.contains(&kind);
    // For Elixir, also check that it's actually def/defp, not any call
    let is_function = if lang == Language::Elixir && kind == "call" {
        is_elixir_function_def(node, source)
    } else {
        is_function
    };

    if is_function {
        if let Some(func_behavior) = analyze_function_generic(node, source, file_path, style, lang, config) {
            if let Some(ref fname) = filter_name {
                if func_behavior.function_name == *fname {
                    functions.push(func_behavior);
                }
            } else {
                functions.push(func_behavior);
            }
        }
        return; // Don't recurse into function bodies for top-level collection
    }

    // Check if this is a class definition
    let is_class = config.class_kinds.contains(&kind);
    let is_class = if lang == Language::Elixir && kind == "call" {
        is_elixir_module_def(node, source)
    } else {
        is_class
    };

    if is_class {
        if let Some(class_behavior) = analyze_class_generic(node, source, file_path, style, lang, config) {
            if let Some(ref fname) = filter_name {
                let filtered_methods: Vec<_> = class_behavior.methods
                    .into_iter()
                    .filter(|m| m.function_name == *fname)
                    .collect();
                if !filtered_methods.is_empty() {
                    classes.push(ClassBehavior {
                        class_name: class_behavior.class_name,
                        invariants: class_behavior.invariants,
                        methods: filtered_methods,
                    });
                }
            } else {
                classes.push(class_behavior);
            }
        }
        return; // Don't double-recurse
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_behaviors_recursive(
            child, source, file_path, style, lang, config,
            filter_name, functions, classes, depth + 1,
        );
    }
}

/// Check if an Elixir call node is a def/defp function definition
fn is_elixir_function_def(node: Node, source: &[u8]) -> bool {
    if let Some(first) = node.child(0) {
        let text = node_text(first, source);
        text == "def" || text == "defp"
    } else {
        false
    }
}

/// Check if an Elixir call node is a defmodule definition
fn is_elixir_module_def(node: Node, source: &[u8]) -> bool {
    if let Some(first) = node.child(0) {
        node_text(first, source) == "defmodule"
    } else {
        false
    }
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format behavioral report as human-readable text
pub fn format_behavioral_text(report: &BehavioralReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!("File: {}", report.file_path));
    lines.push(format!("Docstring Style: {}", report.docstring_style));
    if report.has_icontract {
        lines.push("DbC: icontract detected".to_string());
    }
    if report.has_deal {
        lines.push("DbC: deal detected".to_string());
    }
    lines.push(String::new());

    // Functions
    for func in &report.functions {
        lines.push(format!("Function: {}", func.function_name));
        lines.push(format!("  Line: {}", func.line));
        lines.push(format!("  Pure: {}", func.purity_classification));
        if func.is_async {
            lines.push("  Async: Yes".to_string());
        }
        if func.is_generator {
            lines.push("  Generator: Yes".to_string());
        }

        // Preconditions
        if !func.preconditions.is_empty() {
            lines.push("  Preconditions:".to_string());
            for pre in &func.preconditions {
                let mut desc = format!("    - {}: ", pre.param);
                if let Some(ref expr) = pre.expression {
                    desc.push_str(expr);
                }
                if let Some(ref hint) = pre.type_hint {
                    if pre.expression.is_some() {
                        desc.push_str(&format!(" (type: {})", hint));
                    } else {
                        desc.push_str(&format!("type {}", hint));
                    }
                }
                if let Some(ref d) = pre.description {
                    desc.push_str(&format!(" [{}]", d));
                }
                desc.push_str(&format!(" (source: {})", pre.source));
                lines.push(desc);
            }
        }

        // Postconditions
        if !func.postconditions.is_empty() {
            lines.push("  Postconditions:".to_string());
            for post in &func.postconditions {
                let mut desc = "    - ".to_string();
                if let Some(ref expr) = post.expression {
                    desc.push_str(expr);
                }
                if let Some(ref hint) = post.type_hint {
                    if post.expression.is_some() {
                        desc.push_str(&format!(" (returns: {})", hint));
                    } else {
                        desc.push_str(&format!("returns {}", hint));
                    }
                }
                if let Some(ref d) = post.description {
                    desc.push_str(&format!(" [{}]", d));
                }
                lines.push(desc);
            }
        }

        // Exceptions
        if !func.exceptions.is_empty() {
            lines.push("  Exceptions:".to_string());
            for exc in &func.exceptions {
                let mut desc = format!("    - {}", exc.exception_type);
                if let Some(ref d) = exc.description {
                    desc.push_str(&format!(": {}", d));
                }
                lines.push(desc);
            }
        }

        // Side effects
        if !func.side_effects.is_empty() {
            lines.push("  Side Effects:".to_string());
            for effect in &func.side_effects {
                let mut desc = format!("    - {}", effect.effect_type);
                if let Some(ref target) = effect.target {
                    desc.push_str(&format!(": {}", target));
                }
                lines.push(desc);
            }
        }

        // Yields
        if !func.yields.is_empty() {
            lines.push("  Yields:".to_string());
            for y in &func.yields {
                let mut desc = "    - ".to_string();
                if let Some(ref hint) = y.type_hint {
                    desc.push_str(hint);
                }
                if let Some(ref d) = y.description {
                    if y.type_hint.is_some() {
                        desc.push_str(&format!(": {}", d));
                    } else {
                        desc.push_str(d);
                    }
                }
                lines.push(desc);
            }
        }

        lines.push(String::new());
    }

    // Classes
    for class in &report.classes {
        lines.push(format!("Class: {}", class.class_name));

        if !class.invariants.is_empty() {
            lines.push("  Invariants:".to_string());
            for inv in &class.invariants {
                lines.push(format!("    - {}", inv.expression));
            }
        }

        for method in &class.methods {
            lines.push(format!("  Method: {}", method.function_name));
            // Similar formatting as functions...
            if !method.preconditions.is_empty() {
                lines.push("    Preconditions:".to_string());
                for pre in &method.preconditions {
                    let mut desc = format!("      - {}: ", pre.param);
                    if let Some(ref expr) = pre.expression {
                        desc.push_str(expr);
                    }
                    lines.push(desc);
                }
            }
        }

        lines.push(String::new());
    }

    lines.join("\n")
}

/// Generate LLM-ready constraints output
pub fn generate_llm_constraints(report: &BehavioralReport) -> String {
    let mut lines = Vec::new();

    lines.push(format!("# Behavioral Constraints for {}", report.file_path));
    lines.push(String::new());

    for func in &report.functions {
        lines.push(format!("## Function: {}", func.function_name));
        lines.push(String::new());

        // Preconditions as requires
        if !func.preconditions.is_empty() {
            lines.push("### Requires (Preconditions)".to_string());
            for pre in &func.preconditions {
                if let Some(ref expr) = pre.expression {
                    lines.push(format!("- `{}` for parameter `{}`", expr, pre.param));
                } else if let Some(ref hint) = pre.type_hint {
                    lines.push(format!("- `{}` must be of type `{}`", pre.param, hint));
                } else if let Some(ref desc) = pre.description {
                    lines.push(format!("- `{}`: {}", pre.param, desc));
                }
            }
            lines.push(String::new());
        }

        // Postconditions as ensures
        if !func.postconditions.is_empty() {
            lines.push("### Ensures (Postconditions)".to_string());
            for post in &func.postconditions {
                if let Some(ref expr) = post.expression {
                    lines.push(format!("- {}", expr));
                } else if let Some(ref hint) = post.type_hint {
                    lines.push(format!("- Returns value of type `{}`", hint));
                } else if let Some(ref desc) = post.description {
                    lines.push(format!("- {}", desc));
                }
            }
            lines.push(String::new());
        }

        // Exceptions as raises
        if !func.exceptions.is_empty() {
            lines.push("### Raises".to_string());
            for exc in &func.exceptions {
                if let Some(ref desc) = exc.description {
                    lines.push(format!("- `{}`: {}", exc.exception_type, desc));
                } else {
                    lines.push(format!("- `{}`", exc.exception_type));
                }
            }
            lines.push(String::new());
        }

        // Side effects as warnings
        if !func.side_effects.is_empty() {
            lines.push("### Side Effects (Warning)".to_string());
            for effect in &func.side_effects {
                let effect_desc = match effect.effect_type {
                    EffectType::Io => "I/O operation",
                    EffectType::GlobalWrite => "Modifies global state",
                    EffectType::AttributeWrite => "Modifies object attribute",
                    EffectType::CollectionModify => "Modifies collection in-place",
                    EffectType::Call => "Calls function with side effects",
                    EffectType::UnknownCall => "Calls unknown function (potential side effects)",
                };
                if let Some(ref target) = effect.target {
                    lines.push(format!("- {}: `{}`", effect_desc, target));
                } else {
                    lines.push(format!("- {}", effect_desc));
                }
            }
            lines.push(String::new());
        }

        lines.push("### Purity".to_string());
        match func.purity_classification.as_str() {
            "pure" => lines.push("- This function is pure (no side effects)".to_string()),
            "impure" => lines.push("- This function is impure (has side effects)".to_string()),
            "unknown" => lines.push("- This function has unknown purity (calls unknown functions)".to_string()),
            _ => lines.push(format!("- Purity: {}", func.purity_classification)),
        }
        lines.push(String::new());
    }

    // Classes
    for class in &report.classes {
        lines.push(format!("## Class: {}", class.class_name));
        lines.push(String::new());

        if !class.invariants.is_empty() {
            lines.push("### Class Invariants".to_string());
            for inv in &class.invariants {
                lines.push(format!("- `{}`", inv.expression));
            }
            lines.push(String::new());
        }

        for method in &class.methods {
            lines.push(format!("### Method: {}", method.function_name));

            if !method.preconditions.is_empty() {
                lines.push("#### Requires".to_string());
                for pre in &method.preconditions {
                    if let Some(ref expr) = pre.expression {
                        lines.push(format!("- `{}` for `{}`", expr, pre.param));
                    }
                }
            }

            lines.push(String::new());
        }
    }

    lines.join("\n")
}


// =============================================================================
// Generic Multi-Language Analysis
// =============================================================================

/// Analyze a function node in any supported language
fn analyze_function_generic(
    func_node: Node, source: &[u8], file_path: &str, style: DocstringStyle,
    lang: Language, config: &LangConfig,
) -> Option<FunctionBehavior> {
    if lang == Language::Python {
        return analyze_function(func_node, source, file_path, style);
    }
    let name = get_function_name_generic(func_node, source, lang)?;
    let param_names = extract_param_names_generic(func_node, source, lang, config);
    let docstring = extract_doc_comment(func_node, source, lang);
    let is_async = is_async_function(func_node, source, lang);
    let is_gen = is_generator_generic(func_node, config);

    let pre_extractor = PreconditionExtractor::new(source, param_names.clone());
    let mut preconditions = Vec::new();
    preconditions.extend(extract_guard_clauses_generic(func_node, source, lang, config, &param_names));
    preconditions.extend(extract_type_annotations(func_node, source, lang));
    if let Some(ref doc) = docstring {
        preconditions.extend(pre_extractor.extract_from_docstring(doc, style));
        preconditions.extend(extract_doc_comment_params(doc, lang, &param_names));
    }

    let post_extractor = PostconditionExtractor::new(source);
    let mut postconditions = Vec::new();
    postconditions.extend(extract_return_type_generic(func_node, source, lang));
    if let Some(ref doc) = docstring {
        postconditions.extend(post_extractor.extract_from_docstring(doc, style));
        postconditions.extend(extract_doc_comment_returns(doc, lang));
    }

    let exceptions = detect_exceptions_generic(func_node, source, lang, config, docstring.as_deref(), style);
    let side_effects = detect_side_effects(func_node, source);
    let yields = if is_gen { extract_yield_info(docstring.as_deref(), style) } else { Vec::new() };
    
    // Determine purity classification.
    // When no side effects are detected, we have no evidence either way —
    // absence of evidence is not evidence of purity. Classify as "unknown".
    let has_side_effects = side_effects.iter().any(|e| {
        !matches!(e.effect_type, EffectType::UnknownCall)
    });
    let has_unknown_calls = side_effects.iter().any(|e| {
        matches!(e.effect_type, EffectType::UnknownCall)
    });

    let purity_classification = if has_side_effects {
        "impure".to_string()
    } else if has_unknown_calls {
        "unknown".to_string()
    } else {
        // No effects detected at all — classify conservatively as "unknown".
        "unknown".to_string()
    };

    Some(FunctionBehavior {
        function_name: name, file_path: file_path.to_string(), line: get_line_number(func_node),
        purity_classification, is_generator: is_gen, is_async, preconditions, postconditions,
        exceptions, yields, side_effects,
    })
}

fn analyze_class_generic(
    class_node: Node, source: &[u8], file_path: &str, style: DocstringStyle,
    lang: Language, config: &LangConfig,
) -> Option<ClassBehavior> {
    if lang == Language::Python { return analyze_class(class_node, source, file_path, style); }
    let class_name = get_class_name_generic(class_node, source, lang)?;
    let mut methods = Vec::new();
    collect_methods_recursive(class_node, source, file_path, style, lang, config, &mut methods, 0);
    Some(ClassBehavior { class_name, invariants: Vec::new(), methods })
}

fn collect_methods_recursive(
    node: Node, source: &[u8], file_path: &str, style: DocstringStyle,
    lang: Language, config: &LangConfig, methods: &mut Vec<FunctionBehavior>, depth: usize,
) {
    if depth > 20 { return; }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let is_func = config.function_kinds.contains(&child.kind())
            && !(lang == Language::Elixir && child.kind() == "call" && !is_elixir_function_def(child, source));
        if is_func {
            if let Some(b) = analyze_function_generic(child, source, file_path, style, lang, config) {
                methods.push(b);
            }
        } else {
            collect_methods_recursive(child, source, file_path, style, lang, config, methods, depth + 1);
        }
    }
}

fn get_function_name_generic(func_node: Node, source: &[u8], lang: Language) -> Option<String> {
    match lang {
        Language::C | Language::Cpp => {
            if let Some(decl) = func_node.child_by_field_name("declarator") {
                if decl.kind() == "function_declarator" {
                    if let Some(n) = decl.child_by_field_name("declarator") {
                        if n.kind() == "identifier" { return Some(node_text(n, source).to_string()); }
                    }
                }
                if decl.kind() == "identifier" { return Some(node_text(decl, source).to_string()); }
            }
            None
        }
        Language::Elixir => {
            if let Some(arg) = func_node.child(1) {
                if arg.kind() == "identifier" { return Some(node_text(arg, source).to_string()); }
                if arg.kind() == "call" || arg.kind() == "arguments" {
                    let mut c = arg.walk();
                    for ch in arg.children(&mut c) {
                        if ch.kind() == "identifier" { return Some(node_text(ch, source).to_string()); }
                        if ch.kind() == "call" {
                            if let Some(n) = ch.child(0) {
                                if n.kind() == "identifier" { return Some(node_text(n, source).to_string()); }
                            }
                        }
                    }
                }
            }
            None
        }
        _ => func_node.child_by_field_name("name").map(|n| node_text(n, source).to_string()),
    }
}

fn get_class_name_generic(class_node: Node, source: &[u8], lang: Language) -> Option<String> {
    match lang {
        Language::Rust => {
            class_node.child_by_field_name("name").map(|n| node_text(n, source).to_string())
                .or_else(|| if class_node.kind() == "impl_item" {
                    class_node.child_by_field_name("type").map(|t| node_text(t, source).to_string())
                } else { None })
        }
        Language::Elixir => {
            if let Some(arg) = class_node.child(1) {
                if arg.kind() == "alias" || arg.kind() == "identifier" { return Some(node_text(arg, source).to_string()); }
                let mut c = arg.walk();
                for ch in arg.children(&mut c) {
                    if ch.kind() == "alias" || ch.kind() == "identifier" { return Some(node_text(ch, source).to_string()); }
                }
            }
            None
        }
        _ => class_node.child_by_field_name("name").map(|n| node_text(n, source).to_string()),
    }
}

fn extract_param_names_generic(func_node: Node, source: &[u8], lang: Language, config: &LangConfig) -> HashSet<String> {
    if lang == Language::Python { return extract_param_names(func_node, source); }
    let mut names = HashSet::new();
    let params_node = match lang {
        Language::C | Language::Cpp => func_node.child_by_field_name("declarator").and_then(|d| d.child_by_field_name("parameters")),
        Language::Kotlin => func_node.child_by_field_name("function_value_parameters"),
        _ => func_node.child_by_field_name("parameters")
            .or_else(|| func_node.child_by_field_name("formal_parameters"))
            .or_else(|| func_node.child_by_field_name("parameter_list")),
    };
    if let Some(params) = params_node { collect_param_ids(params, source, config, &mut names); }
    names
}

fn collect_param_ids(node: Node, source: &[u8], config: &LangConfig, names: &mut HashSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let n = node_text(child, source);
                if n != config.self_keyword && n != "cls" { names.insert(n.to_string()); }
            }
            "parameter_declaration" | "formal_parameter" | "required_parameter"
            | "optional_parameter" | "rest_parameter" | "parameter"
            | "simple_formal_parameter" | "keyword_parameter"
            | "splat_parameter" | "hash_splat_parameter" | "block_parameter"
            | "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
                if let Some(nn) = child.child_by_field_name("name")
                    .or_else(|| child.child_by_field_name("internal_name"))
                    .or_else(|| child.child_by_field_name("pattern"))
                    .or_else(|| child.child_by_field_name("declarator")) {
                    if nn.kind() == "identifier" || nn.kind() == "simple_identifier" {
                        let n = node_text(nn, source);
                        if n != config.self_keyword && n != "cls" { names.insert(n.to_string()); }
                    }
                } else {
                    let mut inner = child.walk();
                    for c in child.children(&mut inner) {
                        if c.kind() == "identifier" || c.kind() == "simple_identifier" {
                            let n = node_text(c, source);
                            if n != config.self_keyword && n != "cls" { names.insert(n.to_string()); }
                            break;
                        }
                    }
                }
            }
            "self_parameter" => {}
            _ => collect_param_ids(child, source, config, names),
        }
    }
}

fn extract_doc_comment(func_node: Node, source: &[u8], lang: Language) -> Option<String> {
    if lang == Language::Python { return extract_docstring(func_node, source); }
    extract_preceding_block_comment(func_node, source)
        .or_else(|| extract_preceding_comments(func_node, source))
}

fn extract_preceding_comments(node: Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut current = node;
    while let Some(prev) = current.prev_sibling() {
        let kind = prev.kind();
        if kind == "comment" || kind == "line_comment" || kind == "block_comment" {
            comments.push(node_text(prev, source).to_string());
            current = prev;
        } else if kind == "attribute_item" || kind == "attribute" { current = prev; }
        else { break; }
    }
    if comments.is_empty() { return None; }
    comments.reverse();
    let cleaned: Vec<String> = comments.iter().map(|c| {
        let c = c.trim();
        let c = c.strip_prefix("///").or_else(|| c.strip_prefix("//!")).or_else(|| c.strip_prefix("//")).unwrap_or(c);
        let c = c.strip_prefix('#').unwrap_or(c);
        let c = c.strip_prefix("--").unwrap_or(c);
        c.trim().to_string()
    }).collect();
    let result = cleaned.join("\n");
    if result.is_empty() { None } else { Some(result) }
}

fn extract_preceding_block_comment(node: Node, source: &[u8]) -> Option<String> {
    let mut current = node;
    while let Some(prev) = current.prev_sibling() {
        if prev.kind() == "comment" || prev.kind() == "block_comment" {
            let text = node_text(prev, source);
            if text.starts_with("/**") {
                let cleaned = text.strip_prefix("/**").unwrap_or(text).strip_suffix("*/").unwrap_or(text);
                let lines: Vec<&str> = cleaned.lines().map(|l| l.trim().trim_start_matches('*').trim()).collect();
                let result = lines.join("\n").trim().to_string();
                return if result.is_empty() { None } else { Some(result) };
            }
            current = prev;
        } else { break; }
    }
    None
}

fn is_async_function(func_node: Node, source: &[u8], lang: Language) -> bool {
    match lang {
        Language::Python => func_node.kind() == "async_function_definition",
        Language::TypeScript | Language::JavaScript | Language::Rust => {
            let text = node_text(func_node, source);
            text.starts_with("async ")
        }
        _ => false,
    }
}

fn is_generator_generic(func_node: Node, config: &LangConfig) -> bool {
    if config.yield_kinds.is_empty() { return false; }
    fn check(node: Node, yk: &[&str], fk: &[&str], d: usize) -> bool {
        if d > 100 { return false; }
        if yk.contains(&node.kind()) { return true; }
        if d > 0 && fk.contains(&node.kind()) { return false; }
        let mut c = node.walk();
        for ch in node.children(&mut c) { if check(ch, yk, fk, d + 1) { return true; } }
        false
    }
    check(func_node, config.yield_kinds, config.function_kinds, 0)
}

fn extract_guard_clauses_generic(
    func_node: Node, source: &[u8], lang: Language, config: &LangConfig, param_names: &HashSet<String>,
) -> Vec<Precondition> {
    let mut pres = Vec::new();
    let body = match get_function_body_generic(func_node, lang) { Some(b) => b, None => return pres };
    let mut cursor = body.walk();
    let mut count = 0;
    for stmt in body.children(&mut cursor) {
        count += 1;
        if count > 10 { break; }
        if config.if_kinds.contains(&stmt.kind()) {
            if let Some(p) = extract_guard_from_if_generic(stmt, source, lang, config, param_names) { pres.push(p); }
        }
    }
    pres
}

fn get_function_body_generic(func_node: Node, lang: Language) -> Option<Node> {
    match lang {
        Language::Python => {
            let mut c = func_node.walk();
            for ch in func_node.children(&mut c) { if ch.kind() == "block" { return Some(ch); } }
            None
        }
        Language::Elixir => {
            let mut c = func_node.walk();
            for ch in func_node.children(&mut c) { if ch.kind() == "do_block" { return Some(ch); } }
            None
        }
        _ => func_node.child_by_field_name("body"),
    }
}

fn extract_guard_from_if_generic(
    if_stmt: Node, source: &[u8], lang: Language, config: &LangConfig, param_names: &HashSet<String>,
) -> Option<Precondition> {
    let cond = if_stmt.child_by_field_name("condition").or_else(|| {
        let mut c = if_stmt.walk();
        for ch in if_stmt.children(&mut c) {
            if matches!(ch.kind(), "parenthesized_expression" | "comparison_operator" | "binary_expression"
                | "boolean_operator" | "not_operator" | "unary_expression" | "binary_operator") {
                return Some(ch);
            }
        }
        None
    });
    let consequence = if_stmt.child_by_field_name("consequence").or_else(|| if_stmt.child_by_field_name("body"));
    let cond = cond?;
    let has_raise = if let Some(body) = consequence {
        body_has_error_exit(body, source, lang, config)
    } else {
        let mut c = if_stmt.walk();
        let mut found = false;
        for ch in if_stmt.children(&mut c) {
            if body_has_error_exit(ch, source, lang, config) { found = true; break; }
        }
        found
    };
    if !has_raise { return None; }
    let cond_text = node_text(cond, source).to_string();
    let param = find_param_in_text(&cond_text, param_names)?;
    Some(Precondition {
        param, expression: Some(negate_condition_generic(&cond_text)),
        description: None, type_hint: None, source: ConditionSource::Guard,
    })
}

fn body_has_error_exit(node: Node, source: &[u8], lang: Language, config: &LangConfig) -> bool {
    if config.raise_kinds.contains(&node.kind()) { return true; }
    if matches!(node.kind(), "call" | "call_expression" | "macro_invocation") {
        let t = node_text(node, source);
        if t.starts_with("panic") || t.starts_with("raise") || t.starts_with("error(") || t.starts_with("abort(")
            || t.starts_with("fatalError(") || t.starts_with("preconditionFailure(") { return true; }
    }
    if matches!(node.kind(), "return_expression" | "return_statement") {
        let t = node_text(node, source);
        if t.contains("Err(") || t.contains("err(") { return true; }
    }
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        if body_has_error_exit(ch, source, lang, config) { return true; }
    }
    false
}

fn find_param_in_text(cond: &str, param_names: &HashSet<String>) -> Option<String> {
    for p in param_names {
        for w in cond.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if w == p { return Some(p.clone()); }
        }
    }
    None
}

fn negate_condition_generic(cond: &str) -> String {
    let cond = cond.trim();
    if cond.contains(" < ") { return cond.replace(" < ", " >= "); }
    if cond.contains(" <= ") { return cond.replace(" <= ", " > "); }
    if cond.contains(" > ") { return cond.replace(" > ", " <= "); }
    if cond.contains(" >= ") { return cond.replace(" >= ", " < "); }
    if cond.contains(" == ") { return cond.replace(" == ", " != "); }
    if cond.contains(" != ") { return cond.replace(" != ", " == "); }
    if cond.contains(" is None") { return cond.replace(" is None", " is not None"); }
    if cond.starts_with("not ") { return cond[4..].to_string(); }
    if cond.starts_with('!') { return cond[1..].to_string(); }
    format!("!({})", cond)
}

fn extract_type_annotations(func_node: Node, source: &[u8], lang: Language) -> Vec<Precondition> {
    let mut pres = Vec::new();
    match lang {
        Language::TypeScript | Language::JavaScript => {
            if let Some(params) = func_node.child_by_field_name("parameters").or_else(|| func_node.child_by_field_name("formal_parameters")) {
                let mut c = params.walk();
                for p in params.children(&mut c) {
                    if let (Some(nn), Some(tn)) = (p.child_by_field_name("name"), p.child_by_field_name("type")) {
                        pres.push(Precondition { param: node_text(nn, source).to_string(), expression: None, description: None, type_hint: Some(node_text(tn, source).to_string()), source: ConditionSource::TypeHint });
                    }
                }
            }
        }
        Language::Java | Language::CSharp | Language::Kotlin => {
            if let Some(params) = func_node.child_by_field_name("parameters")
                .or_else(|| func_node.child_by_field_name("formal_parameters"))
                .or_else(|| func_node.child_by_field_name("parameter_list"))
                .or_else(|| func_node.child_by_field_name("function_value_parameters")) {
                let mut c = params.walk();
                for p in params.children(&mut c) {
                    if let (Some(tn), Some(nn)) = (p.child_by_field_name("type"), p.child_by_field_name("name")) {
                        pres.push(Precondition { param: node_text(nn, source).to_string(), expression: None, description: None, type_hint: Some(node_text(tn, source).to_string()), source: ConditionSource::TypeHint });
                    }
                }
            }
        }
        Language::Swift => {
            // Swift params: func f(x: Int, y: String) — parameter nodes with name/type fields
            // Also handle internal_name for `func f(externalName internalName: Type)`
            if let Some(params) = func_node.child_by_field_name("parameters") {
                let mut c = params.walk();
                for p in params.children(&mut c) {
                    if p.kind() == "parameter" || p.kind() == "simple_parameter" {
                        let name = p.child_by_field_name("internal_name")
                            .or_else(|| p.child_by_field_name("name"))
                            .map(|n| node_text(n, source).to_string());
                        let type_ann = p.child_by_field_name("type")
                            .map(|n| node_text(n, source).to_string());
                        if let (Some(n), Some(t)) = (name, type_ann) {
                            if n != "_" {
                                pres.push(Precondition { param: n, expression: None, description: None, type_hint: Some(t), source: ConditionSource::TypeHint });
                            }
                        }
                    }
                }
            }
        }
        Language::Go => {
            if let Some(params) = func_node.child_by_field_name("parameters") {
                let mut c = params.walk();
                for p in params.children(&mut c) {
                    if p.kind() == "parameter_declaration" {
                        let mut pnames = Vec::new();
                        let mut ttext = None;
                        let mut inner = p.walk();
                        for ch in p.children(&mut inner) {
                            if ch.kind() == "identifier" { pnames.push(node_text(ch, source).to_string()); }
                            else if ttext.is_none() && ch.kind() != "," { ttext = Some(node_text(ch, source).to_string()); }
                        }
                        if let Some(ref tt) = ttext {
                            for n in &pnames { pres.push(Precondition { param: n.clone(), expression: None, description: None, type_hint: Some(tt.clone()), source: ConditionSource::TypeHint }); }
                        }
                    }
                }
            }
        }
        Language::Rust => {
            if let Some(params) = func_node.child_by_field_name("parameters") {
                let mut c = params.walk();
                for p in params.children(&mut c) {
                    if p.kind() == "parameter" {
                        if let (Some(pn), Some(pt)) = (p.child_by_field_name("pattern"), p.child_by_field_name("type")) {
                            pres.push(Precondition { param: node_text(pn, source).to_string(), expression: None, description: None, type_hint: Some(node_text(pt, source).to_string()), source: ConditionSource::TypeHint });
                        }
                    }
                }
            }
        }
        _ => {}
    }
    pres
}

fn extract_return_type_generic(func_node: Node, source: &[u8], lang: Language) -> Vec<Postcondition> {
    let mut posts = Vec::new();
    let ret = match lang {
        Language::Python | Language::TypeScript | Language::JavaScript => func_node.child_by_field_name("return_type").map(|n| node_text(n, source).to_string()),
        Language::Go => func_node.child_by_field_name("result").map(|n| node_text(n, source).to_string()),
        Language::Rust => func_node.child_by_field_name("return_type").map(|n| node_text(n, source).to_string()),
        Language::Java | Language::CSharp | Language::Kotlin => func_node.child_by_field_name("type").map(|n| node_text(n, source).to_string()),
        Language::Swift => func_node.child_by_field_name("return_type").map(|n| node_text(n, source).to_string()),
        _ => None,
    };
    if let Some(rt) = ret {
        let rt = rt.trim().to_string();
        if !rt.is_empty() && rt != "void" && rt != "()" { posts.push(Postcondition { expression: None, description: None, type_hint: Some(rt) }); }
    }
    posts
}

fn detect_exceptions_generic(func_node: Node, source: &[u8], lang: Language, config: &LangConfig, docstring: Option<&str>, style: DocstringStyle) -> Vec<ExceptionInfo> {
    if lang == Language::Python { return detect_exceptions(func_node, source, docstring, style); }
    let mut excs = Vec::new();
    let mut seen = HashSet::new();
    extract_error_exits_recursive(func_node, source, lang, config, &mut excs, &mut seen, 0);
    if let Some(doc) = docstring {
        extract_doc_exceptions(doc, lang, &mut excs, &mut seen);
        extract_docstring_exceptions(doc, style, &mut excs, &mut seen);
    }
    excs
}

fn extract_error_exits_recursive(node: Node, source: &[u8], lang: Language, config: &LangConfig, excs: &mut Vec<ExceptionInfo>, seen: &mut HashSet<String>, depth: usize) {
    if depth > 100 { return; }
    if config.raise_kinds.contains(&node.kind()) {
        if let Some(et) = extract_exception_type_node(node, source) {
            if !seen.contains(&et) { seen.insert(et.clone()); excs.push(ExceptionInfo { exception_type: et, description: None }); }
        }
    }
    if matches!(node.kind(), "call_expression" | "call" | "macro_invocation") {
        let t = node_text(node, source);
        if t.starts_with("panic") && !seen.contains("panic") { seen.insert("panic".into()); excs.push(ExceptionInfo { exception_type: "panic".into(), description: None }); }
        if t.starts_with("fatalError") && !seen.contains("fatalError") { seen.insert("fatalError".into()); excs.push(ExceptionInfo { exception_type: "fatalError".into(), description: None }); }
        if t.starts_with("preconditionFailure") && !seen.contains("preconditionFailure") { seen.insert("preconditionFailure".into()); excs.push(ExceptionInfo { exception_type: "preconditionFailure".into(), description: None }); }
    }
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        if config.function_kinds.contains(&ch.kind()) { continue; }
        extract_error_exits_recursive(ch, source, lang, config, excs, seen, depth + 1);
    }
}

fn extract_exception_type_node(node: Node, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        match ch.kind() {
            "call" | "call_expression" | "object_creation_expression" => {
                if let Some(f) = ch.child_by_field_name("function").or_else(|| ch.child_by_field_name("type")) {
                    return Some(node_text(f, source).to_string());
                }
                let mut inner = ch.walk();
                for cc in ch.children(&mut inner) {
                    if cc.kind() == "identifier" || cc.kind() == "type_identifier" { return Some(node_text(cc, source).to_string()); }
                }
            }
            "new_expression" => {
                if let Some(ctor) = ch.child_by_field_name("constructor") { return Some(node_text(ctor, source).to_string()); }
            }
            "identifier" | "type_identifier" => {
                let t = node_text(ch, source);
                if t != "throw" && t != "raise" && t != "new" { return Some(t.to_string()); }
            }
            _ => {}
        }
    }
    None
}

fn extract_doc_comment_params(doc: &str, lang: Language, param_names: &HashSet<String>) -> Vec<Precondition> {
    let mut pres = Vec::new();
    match lang {
        Language::Java | Language::CSharp | Language::Kotlin | Language::Scala
        | Language::TypeScript | Language::JavaScript | Language::Php => {
            for line in doc.lines() {
                let t = line.trim();
                if t.starts_with("@param") {
                    let rest = t.strip_prefix("@param").unwrap_or("").trim();
                    let rest = if rest.starts_with('{') { rest.find('}').map_or(rest, |i| &rest[i+1..]).trim() } else { rest };
                    let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
                    if !parts.is_empty() {
                        let name = parts[0].to_string();
                        let desc = parts.get(1).map(|s| s.trim().to_string());
                        if param_names.contains(&name) {
                            pres.push(Precondition { param: name, expression: None, description: desc, type_hint: None, source: ConditionSource::Docstring });
                        }
                    }
                }
            }
        }
        Language::Rust => {
            if let Some(start) = doc.find("# Arguments") {
                for line in doc[start..].lines().skip(1) {
                    let t = line.trim();
                    if (t.starts_with("* `") || t.starts_with("- `")) && t.len() > 3 {
                        if let Some(end) = t[3..].find('`') {
                            let name = t[3..3+end].to_string();
                            let desc = t.get(3+end+1..).map(|s| s.trim().trim_start_matches('-').trim().to_string());
                            if param_names.contains(&name) {
                                pres.push(Precondition { param: name, expression: None, description: desc, type_hint: None, source: ConditionSource::Docstring });
                            }
                        }
                    } else if t.is_empty() || t.starts_with('#') { break; }
                }
            }
        }
        Language::Swift => {
            // Swift uses `- Parameter name: description` format
            for line in doc.lines() {
                let t = line.trim().trim_start_matches('-').trim();
                if let Some(rest) = t.strip_prefix("Parameter ").or_else(|| t.strip_prefix("parameter ")) {
                    let parts: Vec<&str> = rest.splitn(2, ':').collect();
                    if !parts.is_empty() {
                        let name = parts[0].trim().to_string();
                        let desc = parts.get(1).map(|s| s.trim().to_string());
                        if param_names.contains(&name) {
                            pres.push(Precondition { param: name, expression: None, description: desc, type_hint: None, source: ConditionSource::Docstring });
                        }
                    }
                }
            }
        }
        _ => {}
    }
    pres
}

fn extract_doc_comment_returns(doc: &str, lang: Language) -> Vec<Postcondition> {
    let mut posts = Vec::new();
    match lang {
        Language::Java | Language::CSharp | Language::Kotlin | Language::Scala
        | Language::TypeScript | Language::JavaScript | Language::Php => {
            for line in doc.lines() {
                let t = line.trim();
                if t.starts_with("@return") {
                    let rest = t.strip_prefix("@returns").or_else(|| t.strip_prefix("@return")).unwrap_or("").trim();
                    let rest = if rest.starts_with('{') { rest.find('}').map_or(rest, |i| &rest[i+1..]).trim() } else { rest };
                    if !rest.is_empty() { posts.push(Postcondition { expression: None, description: Some(rest.to_string()), type_hint: None }); }
                }
            }
        }
        Language::Swift => {
            // Swift uses `- Returns: description` format
            for line in doc.lines() {
                let t = line.trim().trim_start_matches('-').trim();
                if let Some(rest) = t.strip_prefix("Returns:").or_else(|| t.strip_prefix("returns:")) {
                    let desc = rest.trim();
                    if !desc.is_empty() { posts.push(Postcondition { expression: None, description: Some(desc.to_string()), type_hint: None }); }
                }
            }
        }
        Language::Rust => {
            if let Some(start) = doc.find("# Returns") {
                let mut lines = Vec::new();
                for line in doc[start..].lines().skip(1) {
                    let t = line.trim();
                    if t.starts_with('#') { break; }
                    if !t.is_empty() { lines.push(t); }
                }
                if !lines.is_empty() { posts.push(Postcondition { expression: None, description: Some(lines.join(" ")), type_hint: None }); }
            }
        }
        _ => {}
    }
    posts
}

fn extract_doc_exceptions(doc: &str, lang: Language, excs: &mut Vec<ExceptionInfo>, seen: &mut HashSet<String>) {
    match lang {
        Language::Java | Language::CSharp | Language::Kotlin | Language::Scala
        | Language::TypeScript | Language::JavaScript | Language::Php => {
            for line in doc.lines() {
                let t = line.trim();
                let rest = if t.starts_with("@throws") { Some(t.strip_prefix("@throws").unwrap_or("").trim()) }
                    else if t.starts_with("@exception") { Some(t.strip_prefix("@exception").unwrap_or("").trim()) }
                    else { None };
                if let Some(rest) = rest {
                    let rest = if rest.starts_with('{') { rest.find('}').map_or(rest, |i| &rest[i+1..]).trim() } else { rest };
                    let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
                    if !parts.is_empty() {
                        let et = parts[0].to_string();
                        let desc = parts.get(1).map(|s| s.trim().to_string());
                        if !et.is_empty() && !seen.contains(&et) { seen.insert(et.clone()); excs.push(ExceptionInfo { exception_type: et, description: desc }); }
                    }
                }
            }
        }
        Language::Rust => {
            for (section, exc_type) in [("# Panics", "panic"), ("# Errors", "Error")] {
                if let Some(start) = doc.find(section) {
                    let mut lines = Vec::new();
                    for line in doc[start..].lines().skip(1) {
                        let t = line.trim();
                        if t.starts_with('#') { break; }
                        if !t.is_empty() { lines.push(t.to_string()); }
                    }
                    if !lines.is_empty() && !seen.contains(exc_type) {
                        seen.insert(exc_type.into());
                        excs.push(ExceptionInfo { exception_type: exc_type.into(), description: Some(lines.join(" ")) });
                    }
                }
            }
        }
        Language::Swift => {
            // Swift uses `- Throws: description` format
            for line in doc.lines() {
                let t = line.trim().trim_start_matches('-').trim();
                if let Some(rest) = t.strip_prefix("Throws:").or_else(|| t.strip_prefix("throws:")) {
                    let desc = rest.trim();
                    if !desc.is_empty() {
                        let parts: Vec<&str> = desc.splitn(2, char::is_whitespace).collect();
                        let et = parts[0].to_string();
                        let description = parts.get(1).map(|s| s.trim().to_string());
                        if !seen.contains(&et) {
                            seen.insert(et.clone());
                            excs.push(ExceptionInfo { exception_type: et, description });
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

// =============================================================================
// Entry Point
// =============================================================================

/// Execute the behavioral command
pub fn run(args: BehavioralArgs) -> anyhow::Result<()> {
    let report = analyze_file(&args.file, &args)?;

    if args.constraints {
        // Output LLM-ready constraints
        println!("{}", generate_llm_constraints(&report));
    } else {
        match args.output_format {
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&report)?;
                println!("{}", json);
            }
            OutputFormat::Text => {
                println!("{}", format_behavioral_text(&report));
            }
        }
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_detect_docstring_style_google() {
        let source = r#"
def foo():
    """Do something.

    Args:
        x: The value.

    Returns:
        The result.
    """
    pass
"#;
        assert_eq!(detect_docstring_style(source), DocstringStyle::Google);
    }

    #[test]
    fn test_detect_docstring_style_numpy() {
        let source = r#"
def foo():
    """Do something.

    Parameters
    ----------
    x : int
        The value.
    """
    pass
"#;
        assert_eq!(detect_docstring_style(source), DocstringStyle::Numpy);
    }

    #[test]
    fn test_detect_docstring_style_sphinx() {
        let source = r#"
def foo():
    """Do something.

    :param x: The value.
    :returns: The result.
    """
    pass
"#;
        assert_eq!(detect_docstring_style(source), DocstringStyle::Sphinx);
    }

    #[test]
    fn test_detect_docstring_style_plain() {
        let source = r#"
def foo():
    """Do something."""
    pass
"#;
        assert_eq!(detect_docstring_style(source), DocstringStyle::Plain);
    }

    #[test]
    fn test_has_icontract_import() {
        assert!(has_icontract_import("import icontract"));
        assert!(has_icontract_import("from icontract import require"));
        assert!(!has_icontract_import("import os"));
    }

    #[test]
    fn test_has_deal_import() {
        assert!(has_deal_import("import deal"));
        assert!(has_deal_import("from deal import pre"));
        assert!(!has_deal_import("import os"));
    }

    #[test]
    fn test_analyze_function_with_guard_clause() {
        let source = r#"
def process_positive(x):
    if x <= 0:
        raise ValueError("x must be positive")
    return x * 2
"#;
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let args = BehavioralArgs {
            file: file.path().to_path_buf(),
            function: Some("process_positive".to_string()),
            output_format: OutputFormat::Json,
            constraints: false,
            project_root: None,
        };

        let report = analyze_file(file.path(), &args).unwrap();
        assert_eq!(report.functions.len(), 1);

        let func = &report.functions[0];
        assert_eq!(func.function_name, "process_positive");

        // Should have a precondition from the guard clause
        let guard_precond = func.preconditions.iter()
            .find(|p| p.source == ConditionSource::Guard);
        assert!(guard_precond.is_some(), "Should detect guard clause precondition");
    }

    #[test]
    fn test_analyze_function_with_assertions() {
        let source = r#"
def process(x, y):
    assert x > 0
    assert y is not None
    return x + y
"#;
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let args = BehavioralArgs {
            file: file.path().to_path_buf(),
            function: None,
            output_format: OutputFormat::Json,
            constraints: false,
            project_root: None,
        };

        let report = analyze_file(file.path(), &args).unwrap();
        assert_eq!(report.functions.len(), 1);

        let func = &report.functions[0];
        let assert_precond = func.preconditions.iter()
            .filter(|p| p.source == ConditionSource::Assertion)
            .count();
        assert!(assert_precond >= 1, "Should detect assertion preconditions");
    }

    #[test]
    fn test_analyze_function_with_exceptions() {
        let source = r#"
def divide(a, b):
    if b == 0:
        raise ZeroDivisionError("Cannot divide by zero")
    return a / b
"#;
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let args = BehavioralArgs {
            file: file.path().to_path_buf(),
            function: None,
            output_format: OutputFormat::Json,
            constraints: false,
            project_root: None,
        };

        let report = analyze_file(file.path(), &args).unwrap();
        assert_eq!(report.functions.len(), 1);

        let func = &report.functions[0];
        assert!(!func.exceptions.is_empty(), "Should detect exceptions");
        assert!(func.exceptions.iter().any(|e| e.exception_type == "ZeroDivisionError"));
    }

    #[test]
    fn test_analyze_impure_function() {
        let source = r#"
def log_message(msg):
    print(msg)
"#;
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let args = BehavioralArgs {
            file: file.path().to_path_buf(),
            function: None,
            output_format: OutputFormat::Json,
            constraints: false,
            project_root: None,
        };

        let report = analyze_file(file.path(), &args).unwrap();
        assert_eq!(report.functions.len(), 1);

        let func = &report.functions[0];
        assert_eq!(func.purity_classification, "impure", "Function with print should be impure");
        assert!(!func.side_effects.is_empty(), "Should detect side effects");
    }

    #[test]
    fn test_analyze_pure_function() {
        let source = r#"
def add(a, b):
    return a + b
"#;
        let mut file = NamedTempFile::with_suffix(".py").unwrap();
        file.write_all(source.as_bytes()).unwrap();

        let args = BehavioralArgs {
            file: file.path().to_path_buf(),
            function: None,
            output_format: OutputFormat::Json,
            constraints: false,
            project_root: None,
        };

        let report = analyze_file(file.path(), &args).unwrap();
        assert_eq!(report.functions.len(), 1);

        let func = &report.functions[0];
        assert_eq!(func.purity_classification, "pure", "Simple arithmetic function should be pure");
        assert!(func.side_effects.is_empty(), "Should have no side effects");
    }
}
