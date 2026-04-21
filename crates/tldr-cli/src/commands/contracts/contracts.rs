//! Contracts command - Pre/postcondition inference from source code.
//!
//! Analyzes guard clauses, assertions, type checks, and return patterns
//! to infer preconditions, postconditions, and invariants for all supported
//! languages.
//!
//! # TIGER Mitigations Addressed
//! - T07: Regex DoS - Use compiled regex with limits
//! - T08: AST stack overflow - check_ast_depth() limits traversal depth
//!
//! # Detection Patterns
//!
//! | Pattern | Confidence | Example |
//! |---------|------------|---------|
//! | `if <cond>: raise/throw/panic` | High | Guard clause -> negated precondition |
//! | `assert <cond>` / `assert!(<cond>)` | High | Direct assertion -> precondition |
//! | `if not isinstance(...)`: raise | High | Direct type precondition (Python) |
//! | Assert after `result =` | High | Postcondition on result |
//! | Type annotations / signatures | Low | Parameter types -> type preconditions |
//!
//! # Supported Languages
//!
//! Python, Go, Rust, Java, TypeScript/JavaScript, C, C++, Ruby, C#, Scala,
//! PHP, Lua, Luau, Elixir, OCaml, and more via tree-sitter grammars.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use regex::Regex;
use tree_sitter::{Node, Parser, Tree};

use tldr_core::ast::ParserPool;
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

use super::error::{ContractsError, ContractsResult};
use super::types::{Condition, ContractsReport, OutputFormat as ContractsOutputFormat};
#[cfg(test)]
use super::types::Confidence;
use super::validation::{
    check_ast_depth, read_file_safe, validate_file_path, validate_function_name, MAX_AST_DEPTH,
    MAX_CONDITIONS_PER_FUNCTION,
};

// =============================================================================
// Language Configuration
// =============================================================================

/// Language-specific AST node kinds for contract extraction.
///
/// Each language has different tree-sitter grammar node names for equivalent
/// constructs (e.g., `if_statement` vs `if_expression`, `raise_statement` vs
/// `throw_statement`). This config maps those differences.
#[derive(Debug, Clone)]
struct LanguageConfig {
    /// Node kinds for function definitions (e.g., "function_definition", "function_declaration")
    function_kinds: &'static [&'static str],
    /// Node kinds for class/struct/impl containers that may hold methods
    class_kinds: &'static [&'static str],
    /// Node kinds for if-statements/expressions
    if_kinds: &'static [&'static str],
    /// Node kinds for assert statements/macros
    assert_kinds: &'static [&'static str],
    /// Node kinds for raise/throw/panic statements
    throw_kinds: &'static [&'static str],
    /// Node kinds for return statements/expressions
    return_kinds: &'static [&'static str],
    /// Node kinds for loop constructs
    loop_kinds: &'static [&'static str],
    /// Node kinds for assignment statements
    assignment_kinds: &'static [&'static str],
    /// Field name for function name in the AST
    func_name_field: &'static str,
    /// Field name for function body in the AST
    func_body_field: &'static str,
    /// Field name for if-condition in the AST
    if_condition_field: &'static str,
    /// Field name for if-consequence (then-block) in the AST
    if_consequence_field: &'static str,
    /// Field name for if-alternative (else-block) in the AST
    if_alternative_field: &'static str,
    /// Field name for function parameters
    func_params_field: &'static str,
    /// Field name for return type annotation
    return_type_field: &'static str,
    /// Field name for class body
    class_body_field: &'static str,
    /// Field name for loop body
    loop_body_field: &'static str,
    /// Whether the language uses `not` prefix (Python) or `!` prefix
    negation_prefix: &'static str,
    /// Whether the language has isinstance-style type checks
    has_isinstance: bool,
    /// Node kinds for typed parameters (for type annotation extraction)
    typed_param_kinds: &'static [&'static str],
    /// Whether the assert is a macro (Rust: assert!) vs statement (Python: assert)
    assert_is_macro: bool,
    /// Function names that act as precondition assertions when called directly
    /// (e.g., Kotlin: require, check; Swift: precondition, assert; Luau: assert)
    assert_call_names: &'static [&'static str],
    /// Function names that act as error/throw when called directly
    /// (e.g., Luau: error; Swift: fatalError, preconditionFailure)
    error_call_names: &'static [&'static str],
    /// Node kinds for call expressions (e.g., "call_expression", "function_call")
    call_kinds: &'static [&'static str],
}

impl LanguageConfig {
    fn for_language(lang: Language) -> Self {
        match lang {
            Language::Python => Self::python(),
            Language::Go => Self::go(),
            Language::Rust => Self::rust(),
            Language::Java => Self::java(),
            Language::TypeScript | Language::JavaScript => Self::typescript(),
            Language::C => Self::c(),
            Language::Cpp => Self::cpp(),
            Language::Ruby => Self::ruby(),
            Language::CSharp => Self::csharp(),
            Language::Scala => Self::scala(),
            Language::Php => Self::php(),
            Language::Lua => Self::lua(),
            Language::Luau => Self::luau(),
            Language::Elixir => Self::elixir(),
            Language::Ocaml => Self::ocaml(),
            Language::Kotlin => Self::kotlin(),
            Language::Swift => Self::swift(),
        }
    }

    fn python() -> Self {
        Self {
            function_kinds: &["function_definition"],
            class_kinds: &["class_definition"],
            if_kinds: &["if_statement"],
            assert_kinds: &["assert_statement"],
            throw_kinds: &["raise_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement"],
            assignment_kinds: &["assignment", "expression_statement"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "return_type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "not",
            has_isinstance: true,
            typed_param_kinds: &["typed_parameter", "typed_default_parameter"],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["call"],
        }
    }

    fn go() -> Self {
        Self {
            function_kinds: &["function_declaration", "method_declaration"],
            class_kinds: &[],
            if_kinds: &["if_statement"],
            assert_kinds: &[],                  // Go has no built-in assert
            throw_kinds: &["return_statement"], // Go uses early return for errors; also panic
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement"],
            assignment_kinds: &["assignment_statement", "short_var_declaration"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "result",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter_declaration"],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &["panic"],
            call_kinds: &["call_expression"],
        }
    }

    fn rust() -> Self {
        Self {
            function_kinds: &["function_item"],
            class_kinds: &["impl_item", "trait_item"],
            if_kinds: &["if_expression"],
            assert_kinds: &["macro_invocation"], // assert!(), debug_assert!()
            throw_kinds: &["return_expression"], // panic!() is also a macro_invocation
            return_kinds: &["return_expression"],
            loop_kinds: &["for_expression", "while_expression", "loop_expression"],
            assignment_kinds: &[
                "let_declaration",
                "assignment_expression",
                "expression_statement",
            ],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "return_type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: true,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["call_expression"],
        }
    }

    fn java() -> Self {
        Self {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &["class_declaration", "interface_declaration"],
            if_kinds: &["if_statement"],
            assert_kinds: &["assert_statement"],
            throw_kinds: &["throw_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement", "enhanced_for_statement"],
            assignment_kinds: &[
                "assignment_expression",
                "local_variable_declaration",
                "expression_statement",
            ],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["formal_parameter"],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["method_invocation"],
        }
    }

    fn typescript() -> Self {
        Self {
            function_kinds: &[
                "function_declaration",
                "method_definition",
                "arrow_function",
            ],
            class_kinds: &["class_declaration"],
            if_kinds: &["if_statement"],
            assert_kinds: &[], // No built-in assert statement (assert is a function call)
            throw_kinds: &["throw_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &[
                "for_statement",
                "while_statement",
                "for_in_statement",
                "for_of_statement",
            ],
            assignment_kinds: &[
                "assignment_expression",
                "variable_declaration",
                "lexical_declaration",
                "expression_statement",
            ],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "return_type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["required_parameter", "optional_parameter"],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["call_expression"],
        }
    }

    fn c() -> Self {
        Self {
            function_kinds: &["function_definition"],
            class_kinds: &[],
            if_kinds: &["if_statement"],
            assert_kinds: &[], // assert() is a macro/call
            throw_kinds: &["return_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement", "do_statement"],
            assignment_kinds: &[
                "assignment_expression",
                "declaration",
                "expression_statement",
            ],
            func_name_field: "declarator",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "declarator",
            return_type_field: "type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter_declaration"],
            assert_is_macro: false,
            assert_call_names: &["assert"],
            error_call_names: &["abort", "exit"],
            call_kinds: &["call_expression"],
        }
    }

    fn cpp() -> Self {
        Self {
            function_kinds: &["function_definition"],
            class_kinds: &["class_specifier", "struct_specifier"],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            throw_kinds: &["throw_statement", "return_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &[
                "for_statement",
                "while_statement",
                "do_statement",
                "for_range_loop",
            ],
            assignment_kinds: &[
                "assignment_expression",
                "declaration",
                "expression_statement",
            ],
            func_name_field: "declarator",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "declarator",
            return_type_field: "type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter_declaration"],
            assert_is_macro: false,
            assert_call_names: &["assert"],
            error_call_names: &["abort", "exit"],
            call_kinds: &["call_expression"],
        }
    }

    fn ruby() -> Self {
        Self {
            function_kinds: &["method"],
            class_kinds: &["class", "module"],
            if_kinds: &["if", "unless"],
            assert_kinds: &[],
            throw_kinds: &["raise", "call"], // raise is a method call in Ruby
            return_kinds: &["return"],
            loop_kinds: &["while", "until", "for"],
            assignment_kinds: &["assignment"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &[],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &["raise"],
            call_kinds: &["call"],
        }
    }

    fn csharp() -> Self {
        Self {
            function_kinds: &["method_declaration", "constructor_declaration"],
            class_kinds: &[
                "class_declaration",
                "interface_declaration",
                "struct_declaration",
            ],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            throw_kinds: &["throw_statement"],
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement", "foreach_statement"],
            assignment_kinds: &[
                "assignment_expression",
                "local_declaration_statement",
                "expression_statement",
            ],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["invocation_expression"],
        }
    }

    fn scala() -> Self {
        Self {
            function_kinds: &["function_definition"],
            class_kinds: &["class_definition", "object_definition", "trait_definition"],
            if_kinds: &["if_expression"],
            assert_kinds: &[], // assert is a function call
            throw_kinds: &["throw_expression"],
            return_kinds: &["return_expression"],
            loop_kinds: &["while_expression", "for_expression"],
            assignment_kinds: &["assignment_expression", "val_definition", "var_definition"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "return_type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: false,
            assert_call_names: &["assert", "require"],
            error_call_names: &[],
            call_kinds: &["call_expression"],
        }
    }

    fn php() -> Self {
        Self {
            function_kinds: &["function_definition", "method_declaration"],
            class_kinds: &["class_declaration"],
            if_kinds: &["if_statement"],
            assert_kinds: &[],
            throw_kinds: &["throw_expression"],
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement", "foreach_statement"],
            assignment_kinds: &["assignment_expression", "expression_statement"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "return_type",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["simple_parameter"],
            assert_is_macro: false,
            assert_call_names: &["assert"],
            error_call_names: &[],
            call_kinds: &["function_call_expression"],
        }
    }

    fn lua() -> Self {
        Self {
            function_kinds: &["function_declaration", "function_definition_statement"],
            class_kinds: &[],
            if_kinds: &["if_statement"],
            assert_kinds: &[], // assert() is a function call in Lua
            throw_kinds: &["return_statement"], // error() is a function call
            return_kinds: &["return_statement"],
            loop_kinds: &[
                "for_statement",
                "while_statement",
                "repeat_statement",
                "for_numeric_statement",
                "for_generic_statement",
            ],
            assignment_kinds: &["assignment_statement", "variable_declaration"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "not",
            has_isinstance: false,
            typed_param_kinds: &[],
            assert_is_macro: false,
            assert_call_names: &["assert"],
            error_call_names: &["error"],
            call_kinds: &["function_call"],
        }
    }

    fn elixir() -> Self {
        Self {
            function_kinds: &["call"], // def/defp are function calls in Elixir tree-sitter
            class_kinds: &["call"],    // defmodule is also a call
            if_kinds: &["call"],       // if/unless are also calls in Elixir
            assert_kinds: &[],
            throw_kinds: &["call"],                 // raise is a function call
            return_kinds: &[],                      // Elixir returns last expression
            loop_kinds: &["call"],                  // for/Enum.each are calls
            assignment_kinds: &["binary_operator"], // = is a binary op
            func_name_field: "target",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "arguments",
            return_type_field: "",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &[],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &["raise"],
            call_kinds: &["call"],
        }
    }

    fn ocaml() -> Self {
        Self {
            function_kinds: &["let_binding", "value_definition"],
            class_kinds: &["module_definition"],
            if_kinds: &["if_expression"],
            assert_kinds: &["assert_expression"],
            throw_kinds: &["raise_expression"],
            return_kinds: &[],
            loop_kinds: &["while_expression", "for_expression"],
            assignment_kinds: &["let_binding"],
            func_name_field: "pattern",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameter",
            return_type_field: "",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "not",
            has_isinstance: false,
            typed_param_kinds: &[],
            assert_is_macro: false,
            assert_call_names: &[],
            error_call_names: &[],
            call_kinds: &["application"],
        }
    }

    fn kotlin() -> Self {
        Self {
            function_kinds: &["function_declaration"],
            class_kinds: &["class_declaration", "object_declaration"],
            if_kinds: &["if_expression"],
            assert_kinds: &[], // assert/require/check are function calls, not statement kinds
            throw_kinds: &["throw_expression"],
            return_kinds: &["return_expression"],
            loop_kinds: &["for_statement", "while_statement"],
            assignment_kinds: &["assignment", "property_declaration"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "function_value_parameters",
            return_type_field: "type",
            class_body_field: "class_body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: false,
            assert_call_names: &[
                "assert",
                "require",
                "check",
                "requireNotNull",
                "checkNotNull",
            ],
            error_call_names: &[],
            call_kinds: &["call_expression"],
        }
    }

    fn swift() -> Self {
        Self {
            function_kinds: &["function_declaration"],
            class_kinds: &[
                "class_declaration",
                "struct_declaration",
                "protocol_declaration",
            ],
            if_kinds: &["if_statement", "guard_statement"],
            assert_kinds: &[], // precondition/assert are function calls
            throw_kinds: &["control_transfer_statement"], // Swift throw/return via control_transfer_statement
            return_kinds: &["control_transfer_statement"],
            loop_kinds: &["for_statement", "while_statement", "repeat_while_statement"],
            assignment_kinds: &["assignment", "property_declaration"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "body", // Swift parameters are children of function_declaration directly
            return_type_field: "return_type",
            class_body_field: "class_body",
            loop_body_field: "body",
            negation_prefix: "!",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: false,
            assert_call_names: &["precondition", "assert", "assertionFailure"],
            error_call_names: &["fatalError", "preconditionFailure"],
            call_kinds: &["call_expression"],
        }
    }

    fn luau() -> Self {
        Self {
            function_kinds: &["function_declaration", "function_definition"],
            class_kinds: &[],
            if_kinds: &["if_statement"],
            assert_kinds: &[], // assert() is a function call in Luau
            throw_kinds: &["return_statement"], // error() is a function call
            return_kinds: &["return_statement"],
            loop_kinds: &["for_statement", "while_statement"],
            assignment_kinds: &["assignment_statement", "variable_declaration"],
            func_name_field: "name",
            func_body_field: "body",
            if_condition_field: "condition",
            if_consequence_field: "consequence",
            if_alternative_field: "alternative",
            func_params_field: "parameters",
            return_type_field: "",
            class_body_field: "body",
            loop_body_field: "body",
            negation_prefix: "not",
            has_isinstance: false,
            typed_param_kinds: &["parameter"],
            assert_is_macro: false,
            assert_call_names: &["assert"],
            error_call_names: &["error"],
            call_kinds: &["function_call"],
        }
    }

    /// Check if a node kind is a function definition
    fn is_function(&self, kind: &str) -> bool {
        self.function_kinds.contains(&kind)
    }

    /// Check if a node kind is a class/container
    fn is_class(&self, kind: &str) -> bool {
        self.class_kinds.contains(&kind)
    }

    /// Check if a node kind is an if-statement/expression
    fn is_if(&self, kind: &str) -> bool {
        self.if_kinds.contains(&kind)
    }

    /// Check if a node kind is an assert statement/macro
    fn is_assert(&self, kind: &str) -> bool {
        self.assert_kinds.contains(&kind)
    }

    /// Check if a node kind is a throw/raise/panic statement
    fn is_throw(&self, kind: &str) -> bool {
        self.throw_kinds.contains(&kind)
    }

    /// Check if a node kind is a loop
    fn is_loop(&self, kind: &str) -> bool {
        self.loop_kinds.contains(&kind)
    }

    /// Check if a node kind is an assignment
    fn is_assignment(&self, kind: &str) -> bool {
        self.assignment_kinds.contains(&kind)
    }

    /// Check if a node kind is a call expression
    fn is_call(&self, kind: &str) -> bool {
        self.call_kinds.contains(&kind)
    }

    /// Check if a function name is an assertion-like call (e.g., require, check, assert)
    fn is_assert_call_name(&self, name: &str) -> bool {
        self.assert_call_names.contains(&name)
    }

    /// Check if a function name is an error/throw-like call (e.g., error, fatalError)
    fn is_error_call_name(&self, name: &str) -> bool {
        self.error_call_names.contains(&name)
    }

    /// Whether this language has call-based assertion patterns
    fn has_assert_calls(&self) -> bool {
        !self.assert_call_names.is_empty()
    }

    /// Whether this language has call-based error patterns
    fn has_error_calls(&self) -> bool {
        !self.error_call_names.is_empty()
    }
}

// =============================================================================
// CLI Arguments
// =============================================================================

/// Infer pre/postconditions from guard clauses, assertions, and type checks.
///
/// Analyzes source code to detect contracts:
/// - Preconditions from guard clauses (`if x < 0: raise` / `if x < 0 { panic!() }`)
/// - Preconditions from assertions (`assert x > 0` / `assert!(x > 0)`)
/// - Type constraints from isinstance checks (Python)
/// - Postconditions from assertions on result variables
///
/// Supports all languages with tree-sitter grammars.
///
/// # Example
///
/// ```bash
/// tldr contracts src/module.py process_data
/// tldr contracts src/lib.rs validate --format text
/// tldr contracts src/main.go processData
/// ```
#[derive(Debug, Args)]
pub struct ContractsArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Output format (json or text). Prefer global --format/-f flag.
    #[arg(
        long = "output-format",
        short = 'o',
        hide = true,
        default_value = "json"
    )]
    pub output_format: ContractsOutputFormat,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Maximum conditions to report per category
    #[arg(long, default_value = "100")]
    pub limit: usize,
}

impl ContractsArgs {
    /// Run the contracts command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Validate inputs
        let canonical_path = validate_file_path(&self.file)?;
        validate_function_name(&self.function)?;

        writer.progress(&format!(
            "Analyzing contracts for {}::{}...",
            self.file.display(),
            self.function
        ));

        // Determine language (FM-22, FM-44: no silent Python fallback)
        let language = match self.lang {
            Some(l) => l,
            None => Language::from_path(&self.file).ok_or_else(|| ContractsError::ParseError {
                file: self.file.clone(),
                message: format!(
                    "Cannot determine language for '{}'. Use --lang to specify.",
                    self.file.display()
                ),
            })?,
        };

        // Verify we have a tree-sitter grammar for this language
        if ParserPool::get_ts_language(language).is_none() {
            return Err(ContractsError::ParseError {
                file: self.file.clone(),
                message: format!("No tree-sitter grammar available for {:?}", language),
            }
            .into());
        }

        // Parse and analyze
        let report = run_contracts(&canonical_path, &self.function, language, self.limit)?;

        // Output based on format
        let use_text = matches!(self.output_format, ContractsOutputFormat::Text)
            || matches!(format, OutputFormat::Text);

        if use_text {
            let text = format_contracts_text(&report);
            writer.write_text(&text)?;
        } else {
            writer.write(&report)?;
        }

        Ok(())
    }
}

// =============================================================================
// Core Analysis Functions
// =============================================================================

/// Run contracts analysis on a file and function.
///
/// # Arguments
/// * `file` - Path to the source file
/// * `function` - Name of the function to analyze
/// * `language` - Programming language
/// * `limit` - Maximum conditions per category
///
/// # Returns
/// ContractsReport with detected preconditions, postconditions, and invariants.
pub fn run_contracts(
    file: &Path,
    function: &str,
    language: Language,
    limit: usize,
) -> ContractsResult<ContractsReport> {
    // Read the file
    let source = read_file_safe(file)?;

    // Get language configuration
    let config = LanguageConfig::for_language(language);
    let _ = config.return_kinds;

    // Parse with tree-sitter (multi-language)
    let tree = parse_source(&source, language, file)?;
    let root = tree.root_node();

    // Find the function in the AST
    let func_node =
        find_function_node(root, function, source.as_bytes(), &config).ok_or_else(|| {
            ContractsError::FunctionNotFound {
                function: function.to_string(),
                file: file.to_path_buf(),
            }
        })?;

    // Extract contracts with depth limiting
    let mut preconditions = Vec::new();
    let mut postconditions = Vec::new();
    let mut invariants = Vec::new();

    // Track lines used for postconditions to avoid duplicating in preconditions
    let mut postcondition_lines = HashSet::new();

    // Extract postconditions first (asserts after result assignment)
    extract_postconditions(
        func_node,
        source.as_bytes(),
        &mut postconditions,
        0,
        &config,
    )?;
    for cond in &postconditions {
        postcondition_lines.insert(cond.source_line);
    }

    // Extract preconditions from guard clauses and asserts
    extract_preconditions(
        func_node,
        source.as_bytes(),
        &mut preconditions,
        &postcondition_lines,
        0,
        &config,
    )?;

    // Extract type annotation preconditions (low confidence)
    extract_type_annotation_preconditions(
        func_node,
        source.as_bytes(),
        &mut preconditions,
        &config,
    )?;

    // Extract untyped parameter preconditions (low confidence)
    // This captures parameters that have no type annotations (e.g., Python's `def f(x, y, **kwargs)`)
    extract_untyped_param_preconditions(func_node, source.as_bytes(), &mut preconditions, &config)?;

    // Extract return type postconditions (low confidence)
    extract_return_type_postconditions(func_node, source.as_bytes(), &mut postconditions, &config)?;

    // Extract contracts from docstrings (Python :param/:return:) and JSDoc (@param/@returns)
    extract_docstring_contracts(
        func_node,
        source.as_bytes(),
        &mut preconditions,
        &mut postconditions,
        &config,
        language,
    )?;

    // Extract loop invariants
    extract_invariants(func_node, source.as_bytes(), &mut invariants, 0, &config)?;

    // Deduplicate conditions
    preconditions = deduplicate_conditions(preconditions);
    postconditions = deduplicate_conditions(postconditions);
    invariants = deduplicate_conditions(invariants);

    // Apply limits
    preconditions.truncate(limit.min(MAX_CONDITIONS_PER_FUNCTION));
    postconditions.truncate(limit.min(MAX_CONDITIONS_PER_FUNCTION));
    invariants.truncate(limit.min(MAX_CONDITIONS_PER_FUNCTION));

    Ok(ContractsReport {
        function: function.to_string(),
        file: file.to_path_buf(),
        preconditions,
        postconditions,
        invariants,
    })
}

/// Parse source code with the appropriate tree-sitter grammar.
fn parse_source(source: &str, language: Language, file: &Path) -> ContractsResult<Tree> {
    let ts_language =
        ParserPool::get_ts_language(language).ok_or_else(|| ContractsError::ParseError {
            file: file.to_path_buf(),
            message: format!("No tree-sitter grammar for {:?}", language),
        })?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| ContractsError::ParseError {
            file: file.to_path_buf(),
            message: format!("Failed to set {:?} language: {}", language, e),
        })?;

    parser
        .parse(source, None)
        .ok_or_else(|| ContractsError::ParseError {
            file: file.to_path_buf(),
            message: "Parsing returned None".to_string(),
        })
}

// =============================================================================
// AST Navigation
// =============================================================================

/// Find a function definition node by name (multi-language).
fn find_function_node<'a>(
    root: Node<'a>,
    function_name: &str,
    source: &[u8],
    config: &LanguageConfig,
) -> Option<Node<'a>> {
    // Recursive search through the entire AST
    find_function_recursive(root, function_name, source, config, 0)
}

/// Recursively search for a function node by name.
fn find_function_recursive<'a>(
    node: Node<'a>,
    function_name: &str,
    source: &[u8],
    config: &LanguageConfig,
    depth: usize,
) -> Option<Node<'a>> {
    if depth > MAX_AST_DEPTH {
        return None;
    }

    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if config.is_function(child.kind()) {
            // Elixir: call nodes where target is "def"/"defp" — name is in arguments
            if child.kind() == "call" {
                if let Some(target) = child.child_by_field_name("target") {
                    let target_text = get_node_text(target, source);
                    if target_text == "def" || target_text == "defp" {
                        // The function name is the first child of the arguments
                        if let Some(args) = child.child_by_field_name("arguments") {
                            if let Some(first_arg) = args.child(0) {
                                let fname = if first_arg.kind() == "call" {
                                    // def send_resp(conn, ...) parses as call -> identifier
                                    first_arg
                                        .child_by_field_name("target")
                                        .map(|n| get_node_text(n, source))
                                        .unwrap_or_default()
                                } else {
                                    get_node_text(first_arg, source)
                                };
                                if fname == function_name {
                                    return Some(child);
                                }
                            }
                        }
                    }
                }
            }

            // OCaml: value_definition -> let_binding -> pattern field
            if child.kind() == "value_definition" {
                let mut inner = child.walk();
                for sub in child.children(&mut inner) {
                    if sub.kind() == "let_binding" {
                        if let Some(pattern_node) = sub.child_by_field_name("pattern") {
                            let name = get_node_text(pattern_node, source);
                            if name == function_name {
                                return Some(child);
                            }
                        }
                    }
                }
            }

            // Try the configured name field first
            if let Some(name_node) = child.child_by_field_name(config.func_name_field) {
                let name = get_node_text(name_node, source);
                // For C/C++, the declarator field may be a function_declarator wrapping the name
                if name == function_name {
                    return Some(child);
                }
                // Check if the name_node contains the function name (e.g., C declarators)
                if name.contains(function_name) {
                    // Verify it's an exact name match by checking for identifier child
                    if let Some(found) = find_identifier_match(name_node, function_name, source) {
                        let _ = found; // We found the match
                        return Some(child);
                    }
                }
            }
            // Fallback: search for identifier children directly
            if find_name_in_children(child, function_name, source) {
                return Some(child);
            }
        }

        // Check for arrow functions in variable declarations (TS/JS pattern):
        // lexical_declaration / variable_declaration -> variable_declarator -> name + value(arrow_function)
        if matches!(child.kind(), "lexical_declaration" | "variable_declaration") {
            let mut decl_cursor = child.walk();
            for decl_child in child.children(&mut decl_cursor) {
                if decl_child.kind() == "variable_declarator" {
                    if let Some(name_node) = decl_child.child_by_field_name("name") {
                        let var_name = get_node_text(name_node, source);
                        if var_name == function_name {
                            if let Some(value_node) = decl_child.child_by_field_name("value") {
                                if matches!(
                                    value_node.kind(),
                                    "arrow_function"
                                        | "function"
                                        | "function_expression"
                                        | "generator_function"
                                ) {
                                    return Some(value_node);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Search inside class/struct/impl containers
        if config.is_class(child.kind()) {
            if let Some(body) = child.child_by_field_name(config.class_body_field) {
                if let Some(found) =
                    find_function_recursive(body, function_name, source, config, depth + 1)
                {
                    return Some(found);
                }
            }
            // Also try searching directly in the class node
            if let Some(found) =
                find_function_recursive(child, function_name, source, config, depth + 1)
            {
                return Some(found);
            }
        }

        // For languages with nested scopes, wrappers, and exports
        if child.kind() == "block"
            || child.kind() == "declaration_list"
            || child.kind() == "module"
            || child.kind() == "source_file"
            || child.kind() == "program"
            || child.kind() == "compound_statement"
            || child.kind() == "export_statement"
            || child.kind() == "export_default_declaration"
            || child.kind() == "decorated_definition"
            || child.kind() == "namespace_declaration"
            || child.kind() == "module_declaration"
            || child.kind() == "class_body"
            || child.kind() == "enum_body"
            || child.kind() == "translation_unit"
            || child.kind() == "do_block"            // Elixir do/end blocks
            || child.kind() == "stab_clause"         // Elixir fn -> clauses
            || child.kind() == "body"                // Elixir/OCaml body nodes
            || child.kind() == "arguments"           // Elixir defmodule arguments
            || child.kind() == "structure"           // OCaml module structures
            || child.kind() == "structure_item"      // OCaml top-level items
            || child.kind() == "module_definition"   // OCaml module/functor defs
            || child.kind() == "module_binding"      // OCaml module binding
            || child.kind() == "functor"
        // OCaml functors
        {
            if let Some(found) =
                find_function_recursive(child, function_name, source, config, depth + 1)
            {
                return Some(found);
            }
        }
    }

    None
}

/// Check if a node contains an identifier matching the function name.
fn find_identifier_match<'a>(
    node: Node<'a>,
    function_name: &str,
    source: &[u8],
) -> Option<Node<'a>> {
    if node.kind() == "identifier" || node.kind() == "name" {
        let text = get_node_text(node, source);
        if text == function_name {
            return Some(node);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_identifier_match(child, function_name, source) {
            return Some(found);
        }
    }
    None
}

/// Check if any direct child identifiers match the function name.
fn find_name_in_children(func_node: Node, function_name: &str, source: &[u8]) -> bool {
    let mut cursor = func_node.walk();
    for child in func_node.children(&mut cursor) {
        if (child.kind() == "identifier" || child.kind() == "name")
            && get_node_text(child, source) == function_name
        {
            return true;
        }
    }
    false
}

/// Find the first identifier or name node among direct children.
fn find_first_identifier(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .find(|&child| child.kind() == "identifier" || child.kind() == "name");
    found
}

/// Get text content of a node.
fn get_node_text<'a>(node: Node<'a>, source: &'a [u8]) -> &'a str {
    let start = node.start_byte();
    let end = node.end_byte();
    if end <= source.len() {
        std::str::from_utf8(&source[start..end]).unwrap_or("")
    } else {
        ""
    }
}

/// Get the function body node.
fn get_function_body<'a>(func: Node<'a>, config: &LanguageConfig) -> Option<Node<'a>> {
    // Try the configured field name first
    if let Some(body) = func.child_by_field_name(config.func_body_field) {
        // If the body node has a "block" child, prefer that (e.g., Swift function_body -> block)
        if let Some(block) = body.child_by_field_name("body") {
            return Some(block);
        }
        return Some(body);
    }

    // Fallback: search for common body node kinds among children
    // This handles Kotlin where function_body is a child node (not a named field)
    let mut cursor = func.walk();
    for child in func.children(&mut cursor) {
        let kind = child.kind();
        if kind == "function_body" || kind == "block" || kind == "compound_statement" {
            // For function_body wrappers, look inside for a block
            if kind == "function_body" {
                let mut inner = child.walk();
                for inner_child in child.children(&mut inner) {
                    if inner_child.kind() == "block" {
                        return Some(inner_child);
                    }
                }
                // If no block inside function_body, return function_body itself
                return Some(child);
            }
            return Some(child);
        }
    }

    None
}

// =============================================================================
// Precondition Extraction
// =============================================================================

/// Extract preconditions from guard clauses and assertions (multi-language).
fn extract_preconditions(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    skip_lines: &HashSet<u32>,
    depth: usize,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    check_ast_depth(depth, &PathBuf::from("<source>"))?;

    let body = match get_function_body(func, config) {
        Some(b) => b,
        None => return Ok(()),
    };

    let mut cursor = body.walk();
    for stmt in body.children(&mut cursor) {
        let line = stmt.start_position().row as u32 + 1;

        // Skip lines already used for postconditions
        if skip_lines.contains(&line) {
            continue;
        }

        let kind = stmt.kind();

        if config.is_if(kind) {
            // Pattern: if <cond>: raise/throw/panic ...
            if body_contains_throw(stmt, source, config) {
                if let Some(cond) = precondition_from_guard(stmt, source, config) {
                    conditions.push(cond);
                }
            }
        } else if config.is_assert(kind) {
            // Pattern: assert <expr> / assert!(<expr>)
            if let Some(cond) = precondition_from_assert(stmt, source, config) {
                conditions.push(cond);
            }
        } else if config.assert_is_macro && kind == "expression_statement" {
            // For Rust: both assert!() and if_expression appear wrapped in expression_statement.
            //
            // assert!() -> expression_statement > macro_invocation
            // if x < 0 { return Err(...); } -> expression_statement > if_expression
            //
            // The if_expression case is the guard clause pattern using `return Err(...)`.
            let mut inner = stmt.walk();
            for child in stmt.children(&mut inner) {
                if config.is_assert(child.kind()) {
                    if let Some(cond) = precondition_from_assert(child, source, config) {
                        conditions.push(cond);
                    }
                } else if config.is_if(child.kind()) {
                    // Guard clause: if <cond> { return Err(...) } or if <cond> { panic!(...) }
                    if body_contains_throw(child, source, config) {
                        if let Some(cond) = precondition_from_guard(child, source, config) {
                            conditions.push(cond);
                        }
                    }
                }
            }
        } else if config.has_assert_calls() && config.is_call(kind) {
            // Pattern: require(cond), check(cond), assert(cond), precondition(cond)
            // These are call expressions that act as assertions (Kotlin, Swift, Luau)
            if let Some(cond) = precondition_from_assert_call(stmt, source, config) {
                conditions.push(cond);
            }
        } else if config.has_assert_calls() && kind == "expression_statement" {
            // Call expressions may be wrapped in expression_statement
            let mut inner = stmt.walk();
            for child in stmt.children(&mut inner) {
                if config.is_call(child.kind()) {
                    if let Some(cond) = precondition_from_assert_call(child, source, config) {
                        conditions.push(cond);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Extract precondition from a call expression that acts as an assertion.
///
/// Pattern: `require(x >= 0)` -> precondition: `x >= 0` (Kotlin)
/// Pattern: `precondition(x >= 0, "msg")` -> precondition: `x >= 0` (Swift)
/// Pattern: `assert(x >= 0, "msg")` -> precondition: `x >= 0` (Luau)
fn precondition_from_assert_call(
    call_node: Node,
    source: &[u8],
    config: &LanguageConfig,
) -> Option<Condition> {
    let call_name = extract_call_name(call_node, source)?;
    if !config.is_assert_call_name(&call_name) {
        return None;
    }
    let line = call_node.start_position().row as u32 + 1;

    // Extract the first argument as the condition
    let first_arg = extract_first_call_argument(call_node, source)?;
    let arg_text = first_arg.trim().to_string();

    if arg_text.is_empty() {
        return None;
    }

    Some(Condition::high(arg_text.clone(), arg_text, line))
}

/// Extract the name of a function being called.
fn extract_call_name(call_node: Node, source: &[u8]) -> Option<String> {
    // Try "function" field first (many languages)
    if let Some(func) = call_node.child_by_field_name("function") {
        return Some(get_node_text(func, source).to_string());
    }

    // Try "name" field (Luau function_call)
    if let Some(name) = call_node.child_by_field_name("name") {
        return Some(get_node_text(name, source).to_string());
    }

    // Fallback: first child that looks like an identifier or navigation_expression
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "identifier" || kind == "simple_identifier" || kind == "name" {
            return Some(get_node_text(child, source).to_string());
        }
        // For navigation expressions like obj.method, get the full text
        if kind == "navigation_expression" {
            return Some(get_node_text(child, source).to_string());
        }
    }

    None
}

/// Extract the first argument text from a call expression.
fn extract_first_call_argument(call_node: Node, source: &[u8]) -> Option<String> {
    // Try "arguments" field first
    if let Some(args) = call_node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for arg in args.children(&mut cursor) {
            let kind = arg.kind();
            if kind != "(" && kind != ")" && kind != "," && kind != "{" && kind != "}" {
                return Some(get_node_text(arg, source).to_string());
            }
        }
    }

    // For Kotlin: call_expression has children [expression, value_arguments]
    // For Swift: call_expression has children [expression, call_suffix]
    let mut cursor = call_node.walk();
    for child in call_node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "value_arguments" || kind == "call_suffix" || kind == "argument_list" {
            let mut inner = child.walk();
            for arg in child.children(&mut inner) {
                let ak = arg.kind();
                if ak != "("
                    && ak != ")"
                    && ak != ","
                    && ak != "{"
                    && ak != "}"
                    && ak != "value_argument"
                    && ak != "annotated_lambda"
                {
                    return Some(get_node_text(arg, source).to_string());
                }
                // Unwrap value_argument to get the expression inside
                if ak == "value_argument" {
                    let mut va_cursor = arg.walk();
                    for va_child in arg.children(&mut va_cursor) {
                        let vak = va_child.kind();
                        if vak != "value_argument_label" && vak != ":" {
                            return Some(get_node_text(va_child, source).to_string());
                        }
                    }
                }
            }
        }
    }

    // Last resort: extract from the full call text by finding the first parenthesized argument
    let text = get_node_text(call_node, source);
    if let Some(start) = text.find('(') {
        let rest = &text[start + 1..];
        // Find the first comma or closing paren
        let end = rest
            .find(',')
            .or_else(|| rest.find(')'))
            .unwrap_or(rest.len());
        let arg = rest[..end].trim();
        if !arg.is_empty() {
            return Some(arg.to_string());
        }
    }

    None
}

/// Check if an if-statement body contains a throw/raise/panic statement (multi-language).
fn body_contains_throw(if_stmt: Node, source: &[u8], config: &LanguageConfig) -> bool {
    // For Swift guard_statement: the else block is where the throw/fatalError goes
    if if_stmt.kind() == "guard_statement" {
        // guard_statement children include "else" keyword and "statements" block
        let mut cursor = if_stmt.walk();
        for child in if_stmt.children(&mut cursor) {
            if (child.kind() == "statements" || child.kind() == "else")
                && node_tree_contains_throw(child, source, config)
            {
                return true;
            }
        }
        // Also check all children recursively (guard may have different structure)
        return node_tree_contains_throw(if_stmt, source, config);
    }

    // Check the consequence (then-block) field
    if let Some(consequence) = if_stmt.child_by_field_name(config.if_consequence_field) {
        if node_tree_contains_throw(consequence, source, config) {
            return true;
        }
    }

    // Also check direct children (for various grammar shapes)
    let mut cursor = if_stmt.walk();
    for child in if_stmt.children(&mut cursor) {
        if config.is_throw(child.kind()) {
            return true;
        }
        // Check inside blocks/compound statements
        if child.kind() == "block"
            || child.kind() == "compound_statement"
            || child.kind() == "function_body"
            || child.kind() == "statements"
            && node_tree_contains_throw(child, source, config)
        {
            return true;
        }
    }

    // For Rust: check for panic!() macro inside the block
    if config.assert_is_macro {
        if let Some(consequence) = if_stmt.child_by_field_name(config.if_consequence_field) {
            if block_contains_panic_macro(consequence, source) {
                return true;
            }
        }
    }

    false
}

/// Check if a node tree contains any throw/raise statements or error function calls.
fn node_tree_contains_throw(node: Node, source: &[u8], config: &LanguageConfig) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if config.is_throw(child.kind()) {
            return true;
        }
        // For Rust: check for panic!() macros
        if config.assert_is_macro
            && child.kind() == "macro_invocation"
            && is_panic_macro(child, source)
        {
            return true;
        }
        // Check for error function calls (e.g., Luau error(), Swift fatalError())
        if config.has_error_calls() && config.is_call(child.kind()) {
            if let Some(name) = extract_call_name(child, source) {
                if config.is_error_call_name(&name) {
                    return true;
                }
            }
        }
        // Check expression_statement wrapping throw or error call
        if child.kind() == "expression_statement" {
            let mut inner = child.walk();
            for grandchild in child.children(&mut inner) {
                if config.is_throw(grandchild.kind()) {
                    return true;
                }
                if config.assert_is_macro
                    && grandchild.kind() == "macro_invocation"
                    && is_panic_macro(grandchild, source)
                {
                    return true;
                }
                if config.has_error_calls() && config.is_call(grandchild.kind()) {
                    if let Some(name) = extract_call_name(grandchild, source) {
                        if config.is_error_call_name(&name) {
                            return true;
                        }
                    }
                }
            }
        }
        // Recurse into blocks, statements, and other containers
        if child.kind() == "block"
            || child.kind() == "compound_statement"
            || child.kind() == "function_body"
            || child.kind() == "statements"
            && node_tree_contains_throw(child, source, config)
        {
            return true;
        }
    }
    false
}

/// Check if a block contains a panic!() macro invocation (Rust-specific).
fn block_contains_panic_macro(block: Node, source: &[u8]) -> bool {
    let mut cursor = block.walk();
    for child in block.children(&mut cursor) {
        if child.kind() == "macro_invocation" && is_panic_macro(child, source) {
            return true;
        }
        if child.kind() == "expression_statement" {
            let mut inner = child.walk();
            for grandchild in child.children(&mut inner) {
                if grandchild.kind() == "macro_invocation" && is_panic_macro(grandchild, source) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a macro_invocation is panic!(), todo!(), unreachable!(), etc.
fn is_panic_macro(node: Node, source: &[u8]) -> bool {
    if node.kind() != "macro_invocation" {
        return false;
    }
    if let Some(macro_node) = node.child_by_field_name("macro") {
        let name = get_node_text(macro_node, source);
        return matches!(name, "panic" | "todo" | "unreachable" | "unimplemented");
    }
    // Fallback: check the text
    let text = get_node_text(node, source);
    text.starts_with("panic!") || text.starts_with("todo!") || text.starts_with("unreachable!")
}

/// Check if a macro_invocation is an assert macro (Rust: assert!, debug_assert!, assert_eq!, etc.)
fn is_assert_macro(node: Node, source: &[u8]) -> bool {
    if node.kind() != "macro_invocation" {
        return false;
    }
    if let Some(macro_node) = node.child_by_field_name("macro") {
        let name = get_node_text(macro_node, source);
        return name.starts_with("assert") || name.starts_with("debug_assert");
    }
    let text = get_node_text(node, source);
    text.starts_with("assert") || text.starts_with("debug_assert")
}

/// Extract precondition from a guard clause by negating the condition (multi-language).
///
/// Pattern: `if x < 0: raise ValueError` -> precondition: `x >= 0`
/// Pattern: `if x < 0 { panic!("...") }` -> precondition: `x >= 0`
/// Pattern: `guard !data.isEmpty else { fatalError(...) }` -> precondition: `!data.isEmpty` (Swift)
fn precondition_from_guard(
    if_stmt: Node,
    source: &[u8],
    config: &LanguageConfig,
) -> Option<Condition> {
    // For Swift guard statements: the condition IS the precondition (not negated)
    // guard <condition> else { throw/fatalError }
    if if_stmt.kind() == "guard_statement" {
        let line = if_stmt.start_position().row as u32 + 1;
        if let Some(condition_node) = if_stmt.child_by_field_name(config.if_condition_field) {
            let condition_text = get_node_text(condition_node, source);
            return Some(Condition::high(
                condition_text.to_string(),
                condition_text.to_string(),
                line,
            ));
        }
        // Fallback: use the full guard text before "else"
        let full_text = get_node_text(if_stmt, source);
        if let Some(else_pos) = full_text.find("else") {
            let guard_cond = full_text[5..else_pos].trim(); // skip "guard "
            if !guard_cond.is_empty() {
                return Some(Condition::high(
                    guard_cond.to_string(),
                    guard_cond.to_string(),
                    line,
                ));
            }
        }
        return None;
    }

    let condition_node = if_stmt.child_by_field_name(config.if_condition_field)?;
    let line = if_stmt.start_position().row as u32 + 1;
    let condition_text = get_node_text(condition_node, source);

    match condition_node.kind() {
        "comparison_operator" | "binary_expression" => {
            // if x < 0: raise -> precondition x >= 0
            if let Some(negated) = negate_comparison(condition_node, source) {
                let var = extract_left_operand(condition_node, source);
                return Some(Condition::high(var, negated, line));
            }
            // Fallback for binary expressions we can't negate
            let negated = format!("{} ({})", config.negation_prefix, condition_text);
            return Some(Condition::medium(condition_text.to_string(), negated, line));
        }
        "not_operator" => {
            // Python: if not isinstance(x, T): raise -> precondition isinstance(x, T)
            if let Some(operand) = condition_node.child_by_field_name("argument") {
                let operand_text = get_node_text(operand, source);
                if operand.kind() == "call"
                    && config.has_isinstance
                    && is_isinstance_call(operand, source)
                {
                    let var = extract_isinstance_var(operand, source);
                    return Some(Condition::high(var, operand_text.to_string(), line));
                } else {
                    return Some(Condition::medium(
                        operand_text.to_string(),
                        operand_text.to_string(),
                        line,
                    ));
                }
            }
        }
        "unary_expression" | "prefix_expression" => {
            // C/Java/Rust/TS: if (!expr) { throw ... } -> precondition expr
            // The operand is typically the second child (after the operator)
            let mut cursor = condition_node.walk();
            for child in condition_node.children(&mut cursor) {
                if child.kind() != "!" && child.kind() != "not" {
                    let operand_text = get_node_text(child, source);
                    if !operand_text.is_empty() {
                        return Some(Condition::medium(
                            operand_text.to_string(),
                            operand_text.to_string(),
                            line,
                        ));
                    }
                }
            }
            // Try the operand field
            if let Some(operand) = condition_node.child_by_field_name("operand") {
                let operand_text = get_node_text(operand, source);
                return Some(Condition::medium(
                    operand_text.to_string(),
                    operand_text.to_string(),
                    line,
                ));
            }
        }
        "call" | "call_expression" => {
            if config.has_isinstance && is_isinstance_call(condition_node, source) {
                let negated = format!("{} ({})", config.negation_prefix, condition_text);
                return Some(Condition::medium(condition_text.to_string(), negated, line));
            }
            let negated = format!("{} ({})", config.negation_prefix, condition_text);
            return Some(Condition::medium(condition_text.to_string(), negated, line));
        }
        "parenthesized_expression" => {
            // Unwrap parentheses and recurse on inner expression
            let mut cursor = condition_node.walk();
            for child in condition_node.children(&mut cursor) {
                if child.kind() != "(" && child.kind() != ")" {
                    // Create a synthetic if-like analysis on the inner expression
                    let inner_text = get_node_text(child, source);
                    if child.kind() == "comparison_operator" || child.kind() == "binary_expression"
                    {
                        if let Some(negated) = negate_comparison(child, source) {
                            let var = extract_left_operand(child, source);
                            return Some(Condition::high(var, negated, line));
                        }
                    }
                    let negated = format!("{} ({})", config.negation_prefix, inner_text);
                    return Some(Condition::medium(inner_text.to_string(), negated, line));
                }
            }
        }
        _ => {
            // Generic: if <expr>: raise -> precondition not (<expr>)
            let negated = format!("{} ({})", config.negation_prefix, condition_text);
            return Some(Condition::medium(condition_text.to_string(), negated, line));
        }
    }

    None
}

/// Extract precondition from an assert statement or macro (multi-language).
///
/// Pattern: `assert x > 0` -> precondition: `x > 0`
/// Pattern: `assert!(x > 0)` -> precondition: `x > 0` (Rust)
fn precondition_from_assert(
    assert_stmt: Node,
    source: &[u8],
    config: &LanguageConfig,
) -> Option<Condition> {
    let line = assert_stmt.start_position().row as u32 + 1;

    if config.assert_is_macro {
        // Rust: assert!(), debug_assert!(), assert_eq!(), etc.
        if !is_assert_macro(assert_stmt, source) {
            return None;
        }
        // Extract the condition from the macro arguments
        // The token_tree child contains the arguments
        let condition_text = extract_macro_args(assert_stmt, source)?;
        return Some(Condition::high(
            condition_text.clone(),
            condition_text,
            line,
        ));
    }

    // Standard assert statement (Python, Java, OCaml)
    let condition_node = extract_assert_condition(assert_stmt, source)?;
    let condition_text = get_node_text(condition_node, source);

    match condition_node.kind() {
        "call" if config.has_isinstance && is_isinstance_call(condition_node, source) => {
            // assert isinstance(x, T)
            let var = extract_isinstance_var(condition_node, source);
            Some(Condition::high(var, condition_text.to_string(), line))
        }
        "comparison_operator" | "binary_expression" => {
            // assert x > 0
            let var = extract_left_operand(condition_node, source);
            Some(Condition::high(var, condition_text.to_string(), line))
        }
        _ => {
            // Generic assert
            Some(Condition::medium(
                condition_text.to_string(),
                condition_text.to_string(),
                line,
            ))
        }
    }
}

/// Extract the condition node from an assert statement.
fn extract_assert_condition<'a>(assert_stmt: Node<'a>, _source: &[u8]) -> Option<Node<'a>> {
    let mut cursor = assert_stmt.walk();
    for child in assert_stmt.children(&mut cursor) {
        // Skip keywords like "assert"
        let kind = child.kind();
        if kind != "assert" && kind != "assert_keyword" && !kind.starts_with("assert") {
            return Some(child);
        }
    }
    None
}

/// Extract the argument text from a macro invocation (Rust assert!(...)).
fn extract_macro_args(macro_node: Node, source: &[u8]) -> Option<String> {
    // Look for token_tree child which contains the arguments
    let mut cursor = macro_node.walk();
    for child in macro_node.children(&mut cursor) {
        if child.kind() == "token_tree" {
            let text = get_node_text(child, source);
            // Strip surrounding parens
            let trimmed = text.trim();
            if trimmed.starts_with('(') && trimmed.ends_with(')') {
                return Some(trimmed[1..trimmed.len() - 1].trim().to_string());
            }
            return Some(trimmed.to_string());
        }
    }
    // Fallback: extract from the full text
    let text = get_node_text(macro_node, source);
    // Pattern: assert!(condition) or assert_eq!(a, b)
    if let Some(start) = text.find('(') {
        if let Some(end) = text.rfind(')') {
            return Some(text[start + 1..end].trim().to_string());
        }
    }
    None
}

// =============================================================================
// Postcondition Extraction
// =============================================================================

/// Extract postconditions from assertions after result assignment (multi-language).
fn extract_postconditions(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    depth: usize,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    check_ast_depth(depth, &PathBuf::from("<source>"))?;

    let body = match get_function_body(func, config) {
        Some(b) => b,
        None => return Ok(()),
    };

    let mut result_assigned = false;
    let mut cursor = body.walk();

    for stmt in body.children(&mut cursor) {
        // Track if 'result' has been assigned
        if config.is_assignment(stmt.kind()) && has_result_assignment(stmt, source, config) {
            result_assigned = true;
        }

        // assert after result assignment -> postcondition
        if result_assigned {
            if config.is_assert(stmt.kind()) {
                if let Some(cond) = postcondition_from_assert(stmt, source, config) {
                    conditions.push(cond);
                }
            } else if config.assert_is_macro && stmt.kind() == "expression_statement" {
                // Rust: assert!() in expression_statement
                let mut inner = stmt.walk();
                for child in stmt.children(&mut inner) {
                    if config.is_assert(child.kind()) && is_assert_macro(child, source) {
                        if let Some(cond) = postcondition_from_assert(child, source, config) {
                            conditions.push(cond);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Check if a statement assigns to a variable named 'result' (multi-language).
fn has_result_assignment(stmt: Node, source: &[u8], config: &LanguageConfig) -> bool {
    // Check "left" field (Python assignment)
    if let Some(left) = stmt.child_by_field_name("left") {
        let text = get_node_text(left, source);
        if text == "result" {
            return true;
        }
    }

    // Check "pattern" field (Rust let binding)
    if let Some(pattern) = stmt.child_by_field_name("pattern") {
        let text = get_node_text(pattern, source);
        if text == "result" {
            return true;
        }
    }

    // Check "name" field (various let/var declarations)
    if let Some(name) = stmt.child_by_field_name("name") {
        let text = get_node_text(name, source);
        if text == "result" {
            return true;
        }
    }

    // Handle children containing assignment
    let mut cursor = stmt.walk();
    for child in stmt.children(&mut cursor) {
        if config.is_assignment(child.kind()) {
            if let Some(left) = child.child_by_field_name("left") {
                let text = get_node_text(left, source);
                if text == "result" {
                    return true;
                }
            }
        }
        // Check identifiers that look like result assignment
        if child.kind() == "identifier" || child.kind() == "name" {
            let text = get_node_text(child, source);
            if text == "result" {
                // Make sure this is an assignment context (has = after it)
                if let Some(next) = child.next_sibling() {
                    let next_text = get_node_text(next, source);
                    if next_text == "=" || next_text == ":=" {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Extract postcondition from an assert statement after result assignment (multi-language).
fn postcondition_from_assert(
    assert_stmt: Node,
    source: &[u8],
    config: &LanguageConfig,
) -> Option<Condition> {
    let line = assert_stmt.start_position().row as u32 + 1;

    if config.assert_is_macro {
        // Rust: assert!(), assert_eq!() etc.
        if !is_assert_macro(assert_stmt, source) {
            return None;
        }
        let condition_text = extract_macro_args(assert_stmt, source)?;
        let var = if condition_text.contains("result") {
            "result".to_string()
        } else {
            condition_text.clone()
        };
        return Some(Condition::high(var, condition_text, line));
    }

    let condition_node = extract_assert_condition(assert_stmt, source)?;
    let condition_text = get_node_text(condition_node, source);

    // Find if 'result' is referenced
    let var = find_result_var(condition_node, source);

    Some(Condition::high(var, condition_text.to_string(), line))
}

/// Find a variable containing 'result' in an expression.
fn find_result_var(node: Node, source: &[u8]) -> String {
    // Check this node
    if node.kind() == "identifier" {
        let text = get_node_text(node, source);
        if text.contains("result") {
            return text.to_string();
        }
    }

    // Check children recursively
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let found = find_result_var(child, source);
        if found.contains("result") {
            return found;
        }
    }

    // Fall back to the full expression
    get_node_text(node, source).to_string()
}

// =============================================================================
// Type Annotation Extraction
// =============================================================================

/// Extract preconditions from function parameter type annotations (multi-language).
///
/// Pattern: `def f(x: int)` -> precondition: `isinstance(x, int)` (low confidence, Python)
/// Pattern: `fn f(x: i32)` -> precondition: `x: i32` (low confidence, Rust)
/// Pattern: `void f(int x)` -> precondition: `x: int` (low confidence, Java/C)
fn extract_type_annotation_preconditions(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    let params = match func.child_by_field_name(config.func_params_field) {
        Some(p) => p,
        None => return Ok(()),
    };

    let line = func.start_position().row as u32 + 1;

    // Recursively search for typed parameters
    extract_typed_params_recursive(params, source, conditions, config, line);

    Ok(())
}

/// Recursively extract typed parameter information.
fn extract_typed_params_recursive(
    node: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
    line: u32,
) {
    let mut cursor = node.walk();

    for param in node.children(&mut cursor) {
        let kind = param.kind();

        // Check if this is a typed parameter kind for the language
        if config.typed_param_kinds.contains(&kind) {
            let name_node = param
                .child_by_field_name("name")
                .or_else(|| param.child_by_field_name("pattern"))
                .or_else(|| {
                    // Find first identifier child
                    find_first_identifier(param)
                });

            let type_node = param
                .child_by_field_name("type")
                .or_else(|| param.child_by_field_name("type_annotation"));

            if let (Some(name_node), Some(type_node)) = (name_node, type_node) {
                let name = get_node_text(name_node, source);
                let type_str = get_node_text(type_node, source);

                // Skip self/cls/this
                if name == "self" || name == "cls" || name == "this" {
                    continue;
                }

                let constraint = if config.has_isinstance {
                    format!("isinstance({}, {})", name, type_str)
                } else {
                    format!("{}: {}", name, type_str)
                };
                conditions.push(Condition::low(name.to_string(), constraint, line));
            }
        }

        // For C/C++ function_declarator -> parameter_list
        if kind == "function_declarator" || kind == "parameter_list" {
            extract_typed_params_recursive(param, source, conditions, config, line);
        }
    }
}

/// Extract postcondition from return type annotation (multi-language).
///
/// Pattern: `def f() -> int` -> postcondition: `isinstance(return, int)` (low confidence, Python)
/// Pattern: `fn f() -> i32` -> postcondition: `return: i32` (low confidence, Rust)
fn extract_return_type_postconditions(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    if config.return_type_field.is_empty() {
        return Ok(());
    }

    let return_type = match func.child_by_field_name(config.return_type_field) {
        Some(rt) => rt,
        None => return Ok(()),
    };

    let line = func.start_position().row as u32 + 1;
    let type_str = get_node_text(return_type, source);

    // Skip void/None/unit return types
    if type_str == "None" || type_str == "void" || type_str == "()" || type_str.is_empty() {
        return Ok(());
    }

    let constraint = if config.has_isinstance {
        format!("isinstance(return, {})", type_str)
    } else {
        format!("return: {}", type_str)
    };
    conditions.push(Condition::low("return".to_string(), constraint, line));

    Ok(())
}

// =============================================================================
// Untyped Parameter Extraction
// =============================================================================

/// Extract preconditions from untyped function parameters.
///
/// For languages where parameters can appear without type annotations (e.g., Python, JavaScript),
/// this extracts basic "parameter is required" preconditions from the parameter list.
///
/// Handles:
/// - Plain identifiers: `def f(x, y)` -> preconditions for x, y
/// - Default parameters: `def f(x=1)` -> precondition with default info
/// - Splat/rest params: `def f(*args, **kwargs)` -> preconditions for args, kwargs
///
/// These are low confidence since they only indicate parameter existence, not constraints.
fn extract_untyped_param_preconditions(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    let params = match func.child_by_field_name(config.func_params_field) {
        Some(p) => p,
        None => return Ok(()),
    };

    let line = func.start_position().row as u32 + 1;

    // Collect names already covered by typed_param extraction to avoid duplicates
    let existing_vars: HashSet<String> = conditions.iter().map(|c| c.variable.clone()).collect();

    extract_untyped_params_recursive(params, source, conditions, config, line, &existing_vars);

    Ok(())
}

/// Recursively extract untyped parameter names from the parameter list.
fn extract_untyped_params_recursive(
    node: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
    line: u32,
    existing_vars: &HashSet<String>,
) {
    let mut cursor = node.walk();

    for param in node.children(&mut cursor) {
        let kind = param.kind();

        // Skip typed parameters (already handled by extract_type_annotation_preconditions)
        if config.typed_param_kinds.contains(&kind) {
            continue;
        }

        // Skip punctuation and keywords
        if kind == "(" || kind == ")" || kind == "," || kind == ":" {
            continue;
        }

        match kind {
            // Plain untyped parameter (Python: identifier, JS: identifier)
            "identifier" | "name" => {
                let name = get_node_text(param, source);
                // Skip self/cls/this
                if name == "self" || name == "cls" || name == "this" {
                    continue;
                }
                if existing_vars.contains(name) {
                    continue;
                }
                let constraint = format!("parameter {} is required", name);
                conditions.push(Condition::low(name.to_string(), constraint, line));
            }

            // Default parameter: Python `x=1`, JS `x = 1`
            "default_parameter" => {
                if let Some(name_node) = param.child_by_field_name("name") {
                    let name = get_node_text(name_node, source);
                    if name == "self" || name == "cls" || name == "this" {
                        continue;
                    }
                    if existing_vars.contains(name) {
                        continue;
                    }
                    let default_val = param
                        .child_by_field_name("value")
                        .map(|v| get_node_text(v, source))
                        .unwrap_or("?");
                    let constraint = format!("parameter {} (default: {})", name, default_val);
                    conditions.push(Condition::low(name.to_string(), constraint, line));
                }
            }

            // Python *args: list_splat_pattern
            "list_splat_pattern" => {
                let mut inner = param.walk();
                for child in param.children(&mut inner) {
                    if child.kind() == "identifier" {
                        let name = get_node_text(child, source);
                        if !existing_vars.contains(name) {
                            let constraint = format!("parameter *{} (variadic positional)", name);
                            conditions.push(Condition::low(name.to_string(), constraint, line));
                        }
                        break;
                    }
                }
            }

            // Python **kwargs: dictionary_splat_pattern
            "dictionary_splat_pattern" => {
                let mut inner = param.walk();
                for child in param.children(&mut inner) {
                    if child.kind() == "identifier" {
                        let name = get_node_text(child, source);
                        if !existing_vars.contains(name) {
                            let constraint = format!("parameter **{} (variadic keyword)", name);
                            conditions.push(Condition::low(name.to_string(), constraint, line));
                        }
                        break;
                    }
                }
            }

            // JS/TS rest parameter: rest_pattern / rest_element
            "rest_pattern" | "rest_element" => {
                let mut inner = param.walk();
                for child in param.children(&mut inner) {
                    if child.kind() == "identifier" {
                        let name = get_node_text(child, source);
                        if !existing_vars.contains(name) {
                            let constraint = format!("parameter ...{} (rest)", name);
                            conditions.push(Condition::low(name.to_string(), constraint, line));
                        }
                        break;
                    }
                }
            }

            // Recurse into sub-containers (e.g., formal_parameters wrapper)
            "formal_parameters" | "parameters" | "parameter_list" => {
                extract_untyped_params_recursive(
                    param,
                    source,
                    conditions,
                    config,
                    line,
                    existing_vars,
                );
            }

            _ => {}
        }
    }
}

// =============================================================================
// Docstring / JSDoc Extraction
// =============================================================================

/// Extract contracts from docstrings (Python) and JSDoc comments (JS/TS).
///
/// Detects:
/// - Python: `:param name:`, `:type name:`, `:return:`, `:rtype:` (Sphinx-style)
/// - Python: `@param name`, `@return` (alternative style)
/// - JS/TS: `@param {type} name`, `@returns {type}` (JSDoc)
///
/// These produce low-confidence conditions since docstrings are documentation,
/// not executable checks.
fn extract_docstring_contracts(
    func: Node,
    source: &[u8],
    preconditions: &mut Vec<Condition>,
    postconditions: &mut Vec<Condition>,
    config: &LanguageConfig,
    language: Language,
) -> ContractsResult<()> {
    let line = func.start_position().row as u32 + 1;

    // Collect existing variables to avoid duplicates
    let existing_pre_vars: HashSet<String> =
        preconditions.iter().map(|c| c.variable.clone()).collect();
    let existing_post_vars: HashSet<String> =
        postconditions.iter().map(|c| c.variable.clone()).collect();

    match language {
        Language::Python => {
            // Python docstrings are expression_statement > string as first statement in body
            let docstring = extract_python_docstring(func, source, config);
            if let Some(doc_text) = docstring {
                extract_sphinx_params(
                    &doc_text,
                    preconditions,
                    postconditions,
                    line,
                    &existing_pre_vars,
                    &existing_post_vars,
                );
            }
        }
        Language::JavaScript | Language::TypeScript => {
            // JSDoc comments appear as comment nodes before the function
            let jsdoc = extract_jsdoc_comment(func, source);
            if let Some(doc_text) = jsdoc {
                extract_jsdoc_params(
                    &doc_text,
                    preconditions,
                    postconditions,
                    line,
                    &existing_pre_vars,
                    &existing_post_vars,
                );
            }
        }
        _ => {}
    }

    Ok(())
}

/// Extract a Python docstring from a function definition.
///
/// In tree-sitter, the docstring is the first `expression_statement` child
/// of the function body, containing a `string` node.
fn extract_python_docstring<'a>(
    func: Node<'a>,
    source: &'a [u8],
    config: &LanguageConfig,
) -> Option<String> {
    let body = func.child_by_field_name(config.func_body_field)?;

    let mut cursor = body.walk();
    if let Some(stmt) = body.children(&mut cursor).next() {
        if stmt.kind() == "expression_statement" {
            // Check for string child (docstring)
            let mut inner = stmt.walk();
            for child in stmt.children(&mut inner) {
                if child.kind() == "string" || child.kind() == "concatenated_string" {
                    let text = get_node_text(child, source);
                    // Strip triple quotes
                    let stripped = text
                        .trim_start_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_start_matches("r\"\"\"")
                        .trim_start_matches("r'''")
                        .trim_end_matches("\"\"\"")
                        .trim_end_matches("'''");
                    return Some(stripped.to_string());
                }
            }
        }
    }

    None
}

/// Extract a JSDoc comment that precedes a function declaration.
///
/// In tree-sitter, comments are typically siblings before the function node.
/// JSDoc comments start with `/**`.
fn extract_jsdoc_comment(func: Node, source: &[u8]) -> Option<String> {
    // Walk backward through preceding siblings to find a comment
    let mut node = func;

    // First check if the function is inside a wrapper (export_statement, etc.)
    // by looking at parent's preceding children
    if let Some(parent) = func.parent() {
        if parent.kind() == "export_statement"
            || parent.kind() == "export_default_declaration"
            || parent.kind() == "decorated_definition"
        {
            node = parent;
        }
    }

    let mut prev = node.prev_sibling();
    if let Some(sibling) = prev {
        if sibling.kind() == "comment" {
            let text = get_node_text(sibling, source);
            if text.starts_with("/**") {
                // Strip comment markers
                let stripped = text
                    .trim_start_matches("/**")
                    .trim_end_matches("*/")
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').trim())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(stripped);
            }
        }
    }

    // Also check if there is a leading comment in the parent scope
    // For `program > comment, function_declaration` pattern
    prev = node.prev_sibling();
    while let Some(sibling) = prev {
        let kind = sibling.kind();
        if kind == "comment" {
            let text = get_node_text(sibling, source);
            if text.starts_with("/**") {
                let stripped = text
                    .trim_start_matches("/**")
                    .trim_end_matches("*/")
                    .lines()
                    .map(|l| l.trim().trim_start_matches('*').trim())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Some(stripped);
            }
        }
        prev = sibling.prev_sibling();
    }

    None
}

/// Extract Sphinx-style docstring parameters and return types.
///
/// Patterns:
/// - `:param name: description` -> precondition
/// - `:type name: typename` -> type precondition
/// - `:return: description` or `:returns: description` -> postcondition
/// - `:rtype: typename` -> return type postcondition
fn extract_sphinx_params(
    docstring: &str,
    preconditions: &mut Vec<Condition>,
    postconditions: &mut Vec<Condition>,
    line: u32,
    existing_pre_vars: &HashSet<String>,
    existing_post_vars: &HashSet<String>,
) {
    // :param name: description
    let param_re = Regex::new(r":param\s+(\w+)\s*:(.*)").unwrap();
    // :type name: typename
    let type_re = Regex::new(r":type\s+(\w+)\s*:\s*(.+)").unwrap();
    // :return: or :returns: description
    let return_re = Regex::new(r":returns?\s*:(.*)").unwrap();
    // :rtype: typename
    let rtype_re = Regex::new(r":rtype\s*:\s*(.+)").unwrap();

    // Track params we add from :param so we can merge with :type
    let mut param_types: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // First pass: collect :type annotations
    for cap in type_re.captures_iter(docstring) {
        let name = cap[1].trim().to_string();
        let type_str = cap[2].trim().to_string();
        param_types.insert(name, type_str);
    }

    // Second pass: extract :param entries
    for cap in param_re.captures_iter(docstring) {
        let name = cap[1].trim().to_string();
        let desc = cap[2].trim().to_string();

        if existing_pre_vars.contains(&name) {
            continue;
        }

        let constraint = if let Some(type_str) = param_types.get(&name) {
            format!("{}: {} ({})", name, type_str, desc)
        } else {
            format!("{}: {}", name, desc)
        };

        preconditions.push(Condition::low(name, constraint, line));
    }

    // Extract :rtype postcondition
    for cap in rtype_re.captures_iter(docstring) {
        let type_str = cap[1].trim().to_string();
        if !type_str.is_empty() && !existing_post_vars.contains("return") {
            let constraint = format!("isinstance(return, {})", type_str);
            postconditions.push(Condition::low("return".to_string(), constraint, line));
        }
    }

    // Extract :return: description (only if no :rtype already added a postcondition)
    let has_rtype = postconditions.iter().any(|c| c.variable == "return");
    if !has_rtype {
        for cap in return_re.captures_iter(docstring) {
            let desc = cap[1].trim().to_string();
            if !desc.is_empty() && !existing_post_vars.contains("return") {
                let constraint = format!("returns: {}", desc);
                postconditions.push(Condition::low("return".to_string(), constraint, line));
                break; // Only take the first :return:
            }
        }
    }
}

/// Extract JSDoc @param and @returns entries.
///
/// Patterns:
/// - `@param {type} name - description` -> precondition
/// - `@param {type} name description` -> precondition (no dash)
/// - `@param name description` -> precondition (no type)
/// - `@returns {type} description` or `@return {type} description` -> postcondition
fn extract_jsdoc_params(
    jsdoc: &str,
    preconditions: &mut Vec<Condition>,
    postconditions: &mut Vec<Condition>,
    line: u32,
    existing_pre_vars: &HashSet<String>,
    existing_post_vars: &HashSet<String>,
) {
    // @param {type} name - description (type is optional)
    let param_with_type_re = Regex::new(r"@param\s+\{([^}]+)\}\s+(\w+)\s*[-:]?\s*(.*)").unwrap();
    let param_no_type_re = Regex::new(r"@param\s+(\w+)\s*[-:]?\s*(.*)").unwrap();
    // @returns {type} description or @return {type} description
    let returns_with_type_re = Regex::new(r"@returns?\s+\{([^}]+)\}\s*(.*)").unwrap();
    let returns_no_type_re = Regex::new(r"@returns?\s+(.+)").unwrap();

    // Extract @param with type first (more specific pattern)
    let mut seen_params: HashSet<String> = HashSet::new();

    for cap in param_with_type_re.captures_iter(jsdoc) {
        let type_str = cap[1].trim().to_string();
        let name = cap[2].trim().to_string();
        let desc = cap[3].trim().to_string();

        if existing_pre_vars.contains(&name) || seen_params.contains(&name) {
            continue;
        }
        seen_params.insert(name.clone());

        let constraint = if desc.is_empty() {
            format!("{}: {}", name, type_str)
        } else {
            format!("{}: {} ({})", name, type_str, desc)
        };
        preconditions.push(Condition::low(name, constraint, line));
    }

    // Extract @param without type (only for params not already found)
    for cap in param_no_type_re.captures_iter(jsdoc) {
        let name = cap[1].trim().to_string();
        let desc = cap[2].trim().to_string();

        if existing_pre_vars.contains(&name) || seen_params.contains(&name) {
            continue;
        }
        seen_params.insert(name.clone());

        let constraint = if desc.is_empty() {
            format!("parameter {} is required", name)
        } else {
            format!("{}: {}", name, desc)
        };
        preconditions.push(Condition::low(name, constraint, line));
    }

    // Extract @returns with type
    let mut has_return = existing_post_vars.contains("return");
    for cap in returns_with_type_re.captures_iter(jsdoc) {
        if has_return {
            break;
        }
        let type_str = cap[1].trim().to_string();
        let desc = cap[2].trim().to_string();

        let constraint = if desc.is_empty() {
            format!("return: {}", type_str)
        } else {
            format!("return: {} ({})", type_str, desc)
        };
        postconditions.push(Condition::low("return".to_string(), constraint, line));
        has_return = true;
    }

    // Extract @returns without type (fallback)
    if !has_return {
        for cap in returns_no_type_re.captures_iter(jsdoc) {
            let desc = cap[1].trim().to_string();
            // Skip if it looks like a type annotation we already matched
            if desc.starts_with('{') {
                continue;
            }
            if !desc.is_empty() {
                let constraint = format!("returns: {}", desc);
                postconditions.push(Condition::low("return".to_string(), constraint, line));
                break;
            }
        }
    }
}

// =============================================================================
// Invariant Extraction
// =============================================================================

/// Extract invariants from assertions inside loops (multi-language).
fn extract_invariants(
    func: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    depth: usize,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    check_ast_depth(depth, &PathBuf::from("<source>"))?;

    let body = match get_function_body(func, config) {
        Some(b) => b,
        None => return Ok(()),
    };

    extract_invariants_from_block(body, source, conditions, depth + 1, config)?;

    Ok(())
}

/// Recursively extract invariants from a block, looking for loops (multi-language).
fn extract_invariants_from_block(
    block: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    depth: usize,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    if depth > MAX_AST_DEPTH {
        return Ok(());
    }

    let mut cursor = block.walk();

    for stmt in block.children(&mut cursor) {
        let kind = stmt.kind();

        if config.is_loop(kind) {
            // Look for asserts inside this loop
            if let Some(loop_body) = stmt.child_by_field_name(config.loop_body_field) {
                extract_asserts_as_invariants(loop_body, source, conditions, config)?;
            }
        } else if config.is_if(kind) {
            // Check nested blocks
            if let Some(consequence) = stmt.child_by_field_name(config.if_consequence_field) {
                extract_invariants_from_block(consequence, source, conditions, depth + 1, config)?;
            }
            if let Some(alternative) = stmt.child_by_field_name(config.if_alternative_field) {
                extract_invariants_from_block(alternative, source, conditions, depth + 1, config)?;
            }
        }
    }

    Ok(())
}

/// Extract assert statements/macros as invariants from a loop body (multi-language).
fn extract_asserts_as_invariants(
    block: Node,
    source: &[u8],
    conditions: &mut Vec<Condition>,
    config: &LanguageConfig,
) -> ContractsResult<()> {
    let mut cursor = block.walk();

    for stmt in block.children(&mut cursor) {
        let kind = stmt.kind();

        if config.is_assert(kind) {
            if config.assert_is_macro {
                // Rust: assert!() macro
                if is_assert_macro(stmt, source) {
                    if let Some(args) = extract_macro_args(stmt, source) {
                        let line = stmt.start_position().row as u32 + 1;
                        conditions.push(Condition::medium(args.clone(), args, line));
                    }
                }
            } else {
                // Standard assert statement
                let mut inner_cursor = stmt.walk();
                for child in stmt.children(&mut inner_cursor) {
                    if child.kind() != "assert" && child.kind() != "assert_keyword" {
                        let constraint = get_node_text(child, source);
                        let line = stmt.start_position().row as u32 + 1;
                        conditions.push(Condition::medium(
                            constraint.to_string(),
                            constraint.to_string(),
                            line,
                        ));
                        break;
                    }
                }
            }
        } else if config.assert_is_macro && kind == "expression_statement" {
            // Rust: assert!() inside expression_statement
            let mut inner = stmt.walk();
            for child in stmt.children(&mut inner) {
                if config.is_assert(child.kind()) && is_assert_macro(child, source) {
                    if let Some(args) = extract_macro_args(child, source) {
                        let line = stmt.start_position().row as u32 + 1;
                        conditions.push(Condition::medium(args.clone(), args, line));
                    }
                }
            }
        }
    }

    Ok(())
}

// =============================================================================
// Condition Negation
// =============================================================================

/// Negate a comparison operator expression.
///
/// `x < 0` -> `x >= 0`
fn negate_comparison(node: Node, source: &[u8]) -> Option<String> {
    // Find the operator
    let mut cursor = node.walk();
    let mut left = None;
    let mut op = None;
    let mut right = None;

    for child in node.children(&mut cursor) {
        let kind = child.kind();
        match kind {
            "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                op = Some(get_node_text(child, source));
            }
            _ => {
                if left.is_none() {
                    left = Some(child);
                } else if right.is_none() {
                    right = Some(child);
                }
            }
        }
    }

    let left_node = left?;
    let right_node = right?;
    let op_text = op?;

    let left_text = get_node_text(left_node, source);
    let right_text = get_node_text(right_node, source);

    let negated_op = negate_operator(op_text)?;
    Some(format!("{} {} {}", left_text, negated_op, right_text))
}

/// Negate a comparison operator.
fn negate_operator(op: &str) -> Option<&'static str> {
    match op {
        "<" => Some(">="),
        "<=" => Some(">"),
        ">" => Some("<="),
        ">=" => Some("<"),
        "==" => Some("!="),
        "!=" => Some("=="),
        "is" => Some("is not"),
        "is not" => Some("is"),
        "in" => Some("not in"),
        "not in" => Some("in"),
        _ => None,
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Check if a call node is an isinstance call.
fn is_isinstance_call(node: Node, source: &[u8]) -> bool {
    if node.kind() != "call" {
        return false;
    }

    if let Some(func) = node.child_by_field_name("function") {
        let func_name = get_node_text(func, source);
        return func_name == "isinstance";
    }

    false
}

/// Extract the variable being checked in an isinstance call.
fn extract_isinstance_var(node: Node, source: &[u8]) -> String {
    if let Some(args) = node.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for arg in args.children(&mut cursor) {
            let kind = arg.kind();
            if kind != "(" && kind != ")" && kind != "," {
                return get_node_text(arg, source).to_string();
            }
        }
    }
    "?".to_string()
}

/// Extract the left operand from a comparison.
fn extract_left_operand(node: Node, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Return the first non-operator child
        let kind = child.kind();
        match kind {
            "<" | ">" | "<=" | ">=" | "==" | "!=" | "is" | "is not" | "in" | "not in" => continue,
            _ => return get_node_text(child, source).to_string(),
        }
    }
    get_node_text(node, source).to_string()
}

/// Deduplicate conditions by (variable, constraint) tuple.
fn deduplicate_conditions(mut conditions: Vec<Condition>) -> Vec<Condition> {
    let mut seen = HashSet::new();
    conditions.retain(|c| {
        let key = (c.variable.clone(), c.constraint.clone());
        seen.insert(key)
    });
    conditions
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format a contracts report as human-readable text.
pub fn format_contracts_text(report: &ContractsReport) -> String {
    let mut output = String::new();

    output.push_str(&format!("Function: {}\n", report.function));

    let mut any_contracts = false;

    for (label, conds) in [
        ("Preconditions", &report.preconditions),
        ("Postconditions", &report.postconditions),
        ("Invariants", &report.invariants),
    ] {
        if !conds.is_empty() {
            any_contracts = true;
            output.push_str(&format!("  {}:\n", label));
            for c in conds {
                output.push_str(&format!(
                    "    - {} ({}, line {}, {})\n",
                    c.constraint, c.variable, c.source_line, c.confidence
                ));
            }
        }
    }

    if !any_contracts {
        output.push_str("  (none detected)\n");
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

    const PYTHON_GUARD_CLAUSES: &str = r#"
def process_data(x, data):
    if x < 0:
        raise ValueError("x must be non-negative")
    if not isinstance(data, list):
        raise TypeError("data must be a list")
    result = sum(data) + x
    return result
"#;

    const PYTHON_ASSERTS: &str = r#"
def calculate(a, b):
    assert a > 0
    assert isinstance(b, int)
    result = a * b
    return result
"#;

    const PYTHON_POSTCONDITIONS: &str = r#"
def divide(a, b):
    if b == 0:
        raise ZeroDivisionError("Cannot divide by zero")
    result = a / b
    assert result is not None
    return result
"#;

    #[test]
    fn test_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "process_data", Language::Python, 100).unwrap();

        // Should detect preconditions from guard clauses
        assert!(
            !report.preconditions.is_empty(),
            "Should detect preconditions"
        );

        // Check for x >= 0 (negation of x < 0)
        let has_x_precond = report
            .preconditions
            .iter()
            .any(|p| p.variable.contains("x") && p.constraint.contains(">="));
        assert!(has_x_precond, "Should detect x >= 0 precondition");
    }

    #[test]
    fn test_assert_extraction() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("asserts.py");
        fs::write(&file_path, PYTHON_ASSERTS).unwrap();

        let report = run_contracts(&file_path, "calculate", Language::Python, 100).unwrap();

        assert!(
            report.preconditions.len() >= 2,
            "Should detect at least 2 preconditions"
        );

        // Check for a > 0
        let has_a_precond = report
            .preconditions
            .iter()
            .any(|p| p.constraint.contains("a > 0") || p.constraint.contains("a>0"));
        assert!(has_a_precond, "Should detect a > 0 precondition");
    }

    #[test]
    fn test_postcondition_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("postcond.py");
        fs::write(&file_path, PYTHON_POSTCONDITIONS).unwrap();

        let report = run_contracts(&file_path, "divide", Language::Python, 100).unwrap();

        // Should detect postcondition: result is not None
        assert!(
            !report.postconditions.is_empty(),
            "Should detect postconditions"
        );

        let has_result_postcond = report
            .postconditions
            .iter()
            .any(|p| p.variable.contains("result") && p.constraint.contains("None"));
        assert!(
            has_result_postcond,
            "Should detect result is not None postcondition"
        );
    }

    #[test]
    fn test_confidence_scoring() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.py");
        fs::write(&file_path, PYTHON_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "process_data", Language::Python, 100).unwrap();

        // Guard clause preconditions should have high confidence
        for precond in &report.preconditions {
            if precond.constraint.contains(">=") || precond.constraint.contains("isinstance") {
                assert_eq!(
                    precond.confidence,
                    Confidence::High,
                    "Guard clause should have High confidence"
                );
            }
        }
    }

    #[test]
    fn test_function_not_found() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.py");
        fs::write(&file_path, "def foo(): pass").unwrap();

        let result = run_contracts(&file_path, "nonexistent", Language::Python, 100);
        assert!(result.is_err());

        match result.unwrap_err() {
            ContractsError::FunctionNotFound { function, .. } => {
                assert_eq!(function, "nonexistent");
            }
            e => panic!("Expected FunctionNotFound, got {:?}", e),
        }
    }

    #[test]
    fn test_empty_function() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("empty.py");
        fs::write(&file_path, "def empty(): pass").unwrap();

        let report = run_contracts(&file_path, "empty", Language::Python, 100).unwrap();

        assert!(report.preconditions.is_empty());
        assert!(report.postconditions.is_empty());
        assert!(report.invariants.is_empty());
    }

    #[test]
    fn test_deduplicate_conditions() {
        let conditions = vec![
            Condition::high("x", "x > 0", 1),
            Condition::high("x", "x > 0", 2), // Duplicate
            Condition::high("y", "y > 0", 3),
        ];

        let deduped = deduplicate_conditions(conditions);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_negate_operator() {
        assert_eq!(negate_operator("<"), Some(">="));
        assert_eq!(negate_operator("<="), Some(">"));
        assert_eq!(negate_operator(">"), Some("<="));
        assert_eq!(negate_operator(">="), Some("<"));
        assert_eq!(negate_operator("=="), Some("!="));
        assert_eq!(negate_operator("!="), Some("=="));
        assert_eq!(negate_operator("is"), Some("is not"));
        assert_eq!(negate_operator("is not"), Some("is"));
    }

    // =========================================================================
    // Multi-language tests
    // =========================================================================

    const RUST_GUARD_CLAUSES: &str = r#"
fn process_data(x: i32, data: &[i32]) -> i32 {
    if x < 0 {
        panic!("x must be non-negative");
    }
    assert!(data.len() > 0);
    let result = data.iter().sum::<i32>() + x;
    assert!(result >= 0);
    result
}
"#;

    #[test]
    fn test_rust_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.rs");
        fs::write(&file_path, RUST_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "process_data", Language::Rust, 100).unwrap();

        // Should detect preconditions from guard clause (panic) and assert!
        assert!(
            !report.preconditions.is_empty(),
            "Rust: Should detect preconditions, got: {:?}",
            report.preconditions
        );
    }

    #[test]
    fn test_rust_assert_macro() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("asserts.rs");
        fs::write(&file_path, RUST_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "process_data", Language::Rust, 100).unwrap();

        // Should detect assert!() as preconditions
        let has_assert = report
            .preconditions
            .iter()
            .any(|p| p.constraint.contains("data.len() > 0") || p.constraint.contains("len"));
        assert!(
            has_assert,
            "Rust: Should detect assert!(data.len() > 0), got: {:?}",
            report.preconditions
        );
    }

    #[test]
    fn test_rust_postcondition() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("postcond.rs");
        fs::write(&file_path, RUST_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "process_data", Language::Rust, 100).unwrap();

        // Should detect postcondition: assert!(result >= 0) after result assignment
        assert!(
            !report.postconditions.is_empty(),
            "Rust: Should detect postconditions after result assignment, got: {:?}",
            report.postconditions
        );
    }

    const GO_GUARD_CLAUSES: &str = r#"
package main

import "errors"

func processData(x int, data []int) (int, error) {
    if x < 0 {
        return 0, errors.New("x must be non-negative")
    }
    if len(data) == 0 {
        return 0, errors.New("data must not be empty")
    }
    result := 0
    for _, v := range data {
        result += v
    }
    return result + x, nil
}
"#;

    #[test]
    fn test_go_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.go");
        fs::write(&file_path, GO_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::Go, 100).unwrap();

        // Should detect preconditions from guard clauses with early return
        assert!(
            !report.preconditions.is_empty(),
            "Go: Should detect preconditions from guard clauses, got: {:?}",
            report.preconditions
        );

        // Check for x >= 0 (negation of x < 0)
        let has_x_precond = report
            .preconditions
            .iter()
            .any(|p| p.constraint.contains(">=") || p.constraint.contains("x"));
        assert!(
            has_x_precond,
            "Go: Should detect x >= 0 precondition, got: {:?}",
            report.preconditions
        );
    }

    const JAVA_GUARD_CLAUSES: &str = r#"
public class Processor {
    public int processData(int x, int[] data) {
        if (x < 0) {
            throw new IllegalArgumentException("x must be non-negative");
        }
        assert data.length > 0;
        int result = 0;
        for (int v : data) {
            result += v;
        }
        return result + x;
    }
}
"#;

    #[test]
    fn test_java_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("Processor.java");
        fs::write(&file_path, JAVA_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::Java, 100).unwrap();

        // Should detect preconditions from guard clauses with throw
        assert!(
            !report.preconditions.is_empty(),
            "Java: Should detect preconditions, got: {:?}",
            report.preconditions
        );
    }

    #[test]
    fn test_java_assert_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("Assert.java");
        fs::write(&file_path, JAVA_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::Java, 100).unwrap();

        // Should detect assert statement
        let has_assert = report
            .preconditions
            .iter()
            .any(|p| p.constraint.contains("data.length") || p.constraint.contains("length"));
        assert!(
            has_assert,
            "Java: Should detect assert data.length > 0, got: {:?}",
            report.preconditions
        );
    }

    const TS_GUARD_CLAUSES: &str = r#"
function processData(x: number, data: number[]): number {
    if (x < 0) {
        throw new Error("x must be non-negative");
    }
    if (data.length === 0) {
        throw new Error("data must not be empty");
    }
    let result = data.reduce((a, b) => a + b, 0) + x;
    return result;
}
"#;

    #[test]
    fn test_ts_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.ts");
        fs::write(&file_path, TS_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::TypeScript, 100).unwrap();

        // Should detect preconditions from guard clauses with throw
        assert!(
            !report.preconditions.is_empty(),
            "TypeScript: Should detect preconditions from throw guards, got: {:?}",
            report.preconditions
        );
    }

    #[test]
    fn test_ts_type_annotations() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("typed.ts");
        fs::write(&file_path, TS_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::TypeScript, 100).unwrap();

        // Should detect type annotations as low-confidence preconditions
        let has_type_precond = report
            .preconditions
            .iter()
            .any(|p| p.confidence == Confidence::Low && p.constraint.contains("number"));
        assert!(
            has_type_precond,
            "TypeScript: Should detect type annotation preconditions, got: {:?}",
            report.preconditions
        );
    }

    const CPP_GUARD_CLAUSES: &str = r#"
#include <stdexcept>
#include <vector>

int processData(int x, const std::vector<int>& data) {
    if (x < 0) {
        throw std::invalid_argument("x must be non-negative");
    }
    int result = 0;
    for (int v : data) {
        result += v;
    }
    return result + x;
}
"#;

    #[test]
    fn test_cpp_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("guards.cpp");
        fs::write(&file_path, CPP_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "processData", Language::Cpp, 100).unwrap();

        // Should detect preconditions from guard clauses with throw
        assert!(
            !report.preconditions.is_empty(),
            "C++: Should detect preconditions from throw guards, got: {:?}",
            report.preconditions
        );
    }

    const CSHARP_GUARD_CLAUSES: &str = r#"
public class Processor {
    public int ProcessData(int x, int[] data) {
        if (x < 0) {
            throw new ArgumentException("x must be non-negative");
        }
        if (data.Length == 0) {
            throw new ArgumentException("data must not be empty");
        }
        int result = 0;
        foreach (int v in data) {
            result += v;
        }
        return result + x;
    }
}
"#;

    #[test]
    fn test_csharp_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("Processor.cs");
        fs::write(&file_path, CSHARP_GUARD_CLAUSES).unwrap();

        let report = run_contracts(&file_path, "ProcessData", Language::CSharp, 100).unwrap();

        // Should detect preconditions from guard clauses with throw
        assert!(
            !report.preconditions.is_empty(),
            "C#: Should detect preconditions from throw guards, got: {:?}",
            report.preconditions
        );
    }

    // Test that all major languages successfully parse (no panics)
    #[test]
    fn test_multi_language_no_panic() {
        let temp = TempDir::new().unwrap();

        let test_cases: Vec<(&str, Language, &str, &str)> = vec![
            ("test.py", Language::Python, "foo", "def foo(x):\n    if x < 0:\n        raise ValueError('bad')\n    return x\n"),
            ("test.go", Language::Go, "foo", "package main\nfunc foo(x int) int {\n    if x < 0 {\n        return -1\n    }\n    return x\n}\n"),
            ("test.rs", Language::Rust, "foo", "fn foo(x: i32) -> i32 {\n    if x < 0 {\n        panic!(\"bad\");\n    }\n    x\n}\n"),
            ("test.java", Language::Java, "foo", "class T {\n    int foo(int x) {\n        if (x < 0) {\n            throw new RuntimeException(\"bad\");\n        }\n        return x;\n    }\n}\n"),
            ("test.ts", Language::TypeScript, "foo", "function foo(x: number): number {\n    if (x < 0) {\n        throw new Error('bad');\n    }\n    return x;\n}\n"),
            ("test.c", Language::C, "foo", "int foo(int x) {\n    if (x < 0) {\n        return -1;\n    }\n    return x;\n}\n"),
            ("test.cpp", Language::Cpp, "foo", "int foo(int x) {\n    if (x < 0) {\n        throw -1;\n    }\n    return x;\n}\n"),
            ("test.cs", Language::CSharp, "Foo", "class T {\n    int Foo(int x) {\n        if (x < 0) {\n            throw new Exception(\"bad\");\n        }\n        return x;\n    }\n}\n"),
        ];

        for (filename, lang, func_name, source) in test_cases {
            let file_path = temp.path().join(filename);
            fs::write(&file_path, source).unwrap();

            let result = run_contracts(&file_path, func_name, lang, 100);
            assert!(
                result.is_ok(),
                "{:?}: Should parse without error, got: {:?}",
                lang,
                result.err()
            );

            let report = result.unwrap();
            // Every test case has a guard clause, so should detect at least one precondition
            assert!(
                !report.preconditions.is_empty(),
                "{:?}: Should detect at least one precondition from guard clause, got: {:?}",
                lang,
                report
            );
        }
    }

    #[test]
    fn test_find_ts_arrow_function_contracts() {
        let ts_source = r#"
const getDuration = (start: Date, end: Date): number => {
    if (!start || !end) {
        throw new Error("invalid arguments");
    }
    return end.getTime() - start.getTime();
};
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("arrow.ts");
        fs::write(&file_path, ts_source).unwrap();

        let result = run_contracts(&file_path, "getDuration", Language::TypeScript, 100);
        assert!(
            result.is_ok(),
            "Should find TS arrow function 'getDuration' for contracts analysis, got: {:?}",
            result.err()
        );
    }

    // =========================================================================
    // P3 Bug Fix Tests: Untyped parameters and docstrings
    // =========================================================================

    /// Python function with untyped parameters should produce basic preconditions
    /// from parameter names alone (the function clearly takes `method`, `url`, `**kwargs`).
    #[test]
    fn test_python_untyped_params_produce_preconditions() {
        let source = r#"
def request(method, url, **kwargs):
    """Sends a request."""
    with sessions.Session() as session:
        return session.request(method=method, url=url, **kwargs)
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("api.py");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "request", Language::Python, 100).unwrap();

        // Should detect at least basic preconditions from parameter names
        let has_method_param = report.preconditions.iter().any(|p| p.variable == "method");
        let has_url_param = report.preconditions.iter().any(|p| p.variable == "url");

        assert!(
            has_method_param,
            "Should detect 'method' parameter as precondition, got: {:?}",
            report.preconditions
        );
        assert!(
            has_url_param,
            "Should detect 'url' parameter as precondition, got: {:?}",
            report.preconditions
        );
    }

    /// Python function with typed parameters should produce preconditions
    /// with type info even when there are no guard clauses.
    #[test]
    fn test_python_typed_params_no_guards() {
        let source = r#"
def greet(name: str, count: int = 1) -> str:
    return (name + "! ") * count
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("greet.py");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "greet", Language::Python, 100).unwrap();

        // Should have type annotation preconditions (low confidence)
        let has_name_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "name" && p.constraint.contains("str"));
        let has_count_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "count" && p.constraint.contains("int"));
        let has_return_type = report
            .postconditions
            .iter()
            .any(|p| p.constraint.contains("str"));

        assert!(
            has_name_type,
            "Should detect name: str type precondition, got: {:?}",
            report.preconditions
        );
        assert!(
            has_count_type,
            "Should detect count: int type precondition, got: {:?}",
            report.preconditions
        );
        assert!(
            has_return_type,
            "Should detect -> str return type postcondition, got: {:?}",
            report.postconditions
        );
    }

    /// Python docstring with :param and :return: should produce contracts.
    #[test]
    fn test_python_docstring_param_extraction() {
        let source = r#"
def request(method, url, **kwargs):
    """Constructs and sends a Request.

    :param method: method for the new Request object.
    :param url: URL for the new Request object.
    :param params: (optional) Dictionary to send in the query string.
    :return: Response object
    :rtype: requests.Response
    """
    with sessions.Session() as session:
        return session.request(method=method, url=url, **kwargs)
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("api.py");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "request", Language::Python, 100).unwrap();

        // Should extract docstring params as preconditions
        let has_method = report.preconditions.iter().any(|p| p.variable == "method");
        let has_url = report.preconditions.iter().any(|p| p.variable == "url");

        assert!(
            has_method,
            "Should extract :param method from docstring, got: {:?}",
            report.preconditions
        );
        assert!(
            has_url,
            "Should extract :param url from docstring, got: {:?}",
            report.preconditions
        );

        // Should extract :rtype as postcondition
        let has_return = report.postconditions.iter().any(|p| {
            p.constraint.contains("Response") || p.constraint.contains("requests.Response")
        });
        assert!(
            has_return,
            "Should extract :rtype from docstring as postcondition, got: {:?}",
            report.postconditions
        );
    }

    /// Python **kwargs should be recognized as a parameter
    #[test]
    fn test_python_kwargs_parameter() {
        let source = r#"
def request(method, url, **kwargs):
    pass
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("kwargs.py");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "request", Language::Python, 100).unwrap();

        // Should detect kwargs as a parameter
        let has_kwargs = report
            .preconditions
            .iter()
            .any(|p| p.variable == "kwargs" || p.variable == "**kwargs");

        assert!(
            has_kwargs,
            "Should detect **kwargs parameter, got: {:?}",
            report.preconditions
        );
    }

    /// TypeScript function with typed params but no guard clauses should produce contracts.
    #[test]
    fn test_typescript_typed_params_no_guards() {
        let source = r#"
function processData(x: number, data: string[]): string {
    return data.join(", ") + x.toString();
}
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("process.ts");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "processData", Language::TypeScript, 100).unwrap();

        // Should detect type annotations as preconditions
        let has_x_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "x" && p.constraint.contains("number"));
        let has_data_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "data" && p.constraint.contains("string"));

        assert!(
            has_x_type,
            "Should detect x: number type precondition, got: {:?}",
            report.preconditions
        );
        assert!(
            has_data_type,
            "Should detect data: string[] type precondition, got: {:?}",
            report.preconditions
        );

        // Should detect return type postcondition
        let has_return = report
            .postconditions
            .iter()
            .any(|p| p.constraint.contains("string"));
        assert!(
            has_return,
            "Should detect return type string postcondition, got: {:?}",
            report.postconditions
        );
    }

    /// JSDoc @param and @returns should produce contracts for JS/TS functions.
    #[test]
    fn test_jsdoc_param_extraction() {
        let source = r#"
/**
 * Sends a request to the server.
 * @param {string} method - The HTTP method.
 * @param {string} url - The request URL.
 * @returns {Promise<Response>} The server response.
 */
function request(method, url) {
    return fetch(url, { method: method });
}
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("api.js");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "request", Language::JavaScript, 100).unwrap();

        // Should extract JSDoc @param as preconditions
        let has_method = report.preconditions.iter().any(|p| p.variable == "method");
        let has_url = report.preconditions.iter().any(|p| p.variable == "url");

        assert!(
            has_method,
            "Should extract @param method from JSDoc, got: {:?}",
            report.preconditions
        );
        assert!(
            has_url,
            "Should extract @param url from JSDoc, got: {:?}",
            report.preconditions
        );

        // Should extract @returns as postcondition
        let has_return = report
            .postconditions
            .iter()
            .any(|p| p.constraint.contains("Promise") || p.constraint.contains("Response"));
        assert!(
            has_return,
            "Should extract @returns from JSDoc as postcondition, got: {:?}",
            report.postconditions
        );
    }

    /// Rust function with `return Err(...)` guard clauses should produce preconditions.
    ///
    /// This tests the pattern:
    /// ```rust
    /// fn transfer(amount: f64) -> Result<(), String> {
    ///     if amount <= 0.0 { return Err("Amount must be positive".to_string()); }
    /// }
    /// ```
    /// The `return Err(...)` is the Rust equivalent of `throw`/`raise`, and should
    /// be detected as a guard clause that produces a HIGH-confidence precondition.
    const RUST_RETURN_ERR_GUARDS: &str = r#"
fn transfer(amount: f64) -> Result<(), String> {
    if amount <= 0.0 {
        return Err("Amount must be positive".to_string());
    }
    if amount > 10000.0 {
        return Err("Amount exceeds limit".to_string());
    }
    Ok(())
}
"#;

    #[test]
    fn test_rust_return_err_guard_clause_detection() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("transfer.rs");
        fs::write(&file_path, RUST_RETURN_ERR_GUARDS).unwrap();

        let report = run_contracts(&file_path, "transfer", Language::Rust, 100).unwrap();

        // Should detect preconditions from `return Err(...)` guard clauses
        assert!(
            !report.preconditions.is_empty(),
            "Rust: Should detect preconditions from `return Err(...)` guard clauses, got: {:?}",
            report.preconditions
        );

        // `if amount <= 0.0 { return Err(...) }` -> precondition: amount > 0.0
        let has_amount_pos = report
            .preconditions
            .iter()
            .any(|p| p.variable.contains("amount") && p.constraint.contains(">"));
        assert!(
            has_amount_pos,
            "Rust: Should detect `amount > 0.0` precondition (negation of `amount <= 0.0`), got: {:?}",
            report.preconditions
        );

        // `if amount > 10000.0 { return Err(...) }` -> precondition: amount <= 10000.0
        let has_amount_limit = report
            .preconditions
            .iter()
            .any(|p| p.variable.contains("amount") && p.constraint.contains("<="));
        assert!(
            has_amount_limit,
            "Rust: Should detect `amount <= 10000.0` precondition (negation of `amount > 10000.0`), got: {:?}",
            report.preconditions
        );

        // All guard clause preconditions should have High confidence
        for precond in &report.preconditions {
            if precond.constraint.contains(">") || precond.constraint.contains("<=") {
                assert_eq!(
                    precond.confidence,
                    Confidence::High,
                    "Rust `return Err(...)` guard should produce High confidence, got: {:?}",
                    precond
                );
            }
        }
    }

    /// Go function with typed parameters but no error checks should still produce contracts.
    #[test]
    fn test_go_typed_params_no_guards() {
        let source = r#"
package main

func add(x int, y int) int {
    return x + y
}
"#;
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("add.go");
        fs::write(&file_path, source).unwrap();

        let report = run_contracts(&file_path, "add", Language::Go, 100).unwrap();

        // Should detect parameter types as preconditions
        let has_x_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "x" && p.constraint.contains("int"));
        let has_y_type = report
            .preconditions
            .iter()
            .any(|p| p.variable == "y" && p.constraint.contains("int"));

        assert!(
            has_x_type,
            "Should detect x: int parameter type, got: {:?}",
            report.preconditions
        );
        assert!(
            has_y_type,
            "Should detect y: int parameter type, got: {:?}",
            report.preconditions
        );

        // Should detect return type
        let has_return = report
            .postconditions
            .iter()
            .any(|p| p.constraint.contains("int"));
        assert!(
            has_return,
            "Should detect int return type postcondition, got: {:?}",
            report.postconditions
        );
    }
}
