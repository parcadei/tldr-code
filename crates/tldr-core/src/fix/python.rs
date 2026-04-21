//! Python error analyzers -- all 22 analyzers ported from FastEdit.
//!
//! Each analyzer is a pure function that takes a `ParsedError`, source code,
//! a tree-sitter `Tree`, and an optional `ApiSurface`, and returns an
//! `Option<Diagnosis>`.
//!
//! The analyzers are dispatched by error type via `diagnose_python()`.
//!
//! # Analyzer Inventory (22 total)
//!
//! | # | Error Class           | Function                          |
//! |---|-----------------------|-----------------------------------|
//! |  1| UnboundLocalError     | analyze_unbound_local             |
//! |  2| TypeError (callable)  | analyze_type_error_callable       |
//! |  3| TypeError (JSON)      | analyze_type_error_serialization  |
//! |  4| NameError             | analyze_name_error                |
//! |  5| ImportError           | analyze_import_error              |
//! |  6| AttributeError        | analyze_attribute_error           |
//! |  7| ValueError            | analyze_value_error               |
//! |  8| IndexError            | analyze_index_error               |
//! |  9| KeyError              | analyze_key_error                 |
//! | 10| ZeroDivisionError     | analyze_zero_division             |
//! | 11| RecursionError        | analyze_recursion_error           |
//! | 12| StopIteration         | analyze_stop_iteration            |
//! | 13| AssertionError        | analyze_assertion_error           |
//! | 14| NotImplementedError   | analyze_not_implemented           |
//! | 15| OSError               | analyze_os_error                  |
//! | 16| UnicodeError          | analyze_unicode_error             |
//! | 17| SyntaxError           | analyze_syntax_error              |
//! | 18| IndentationError      | analyze_indentation_error         |
//! | 19| CircularImportError   | analyze_circular_import           |
//! | 20| TypeError (other)     | analyze_type_error_general        |
//! | 21| RuntimeError          | analyze_runtime_error             |
//! | 22| Exception (generic)   | analyze_generic_exception         |

use regex::Regex;
use tree_sitter::Tree;

use super::error_parser::extract_variable_name;
use super::types::{Diagnosis, EditKind, Fix, FixConfidence, FixLocation, ParsedError, TextEdit};

// ============================================================================
// Known stdlib imports table (ported from FastEdit names.py _STDLIB_IMPORTS)
// ============================================================================

/// Maps a name to the import statement needed to make it available.
static STDLIB_IMPORTS: &[(&str, &str)] = &[
    ("os", "import os"),
    ("sys", "import sys"),
    ("json", "import json"),
    ("re", "import re"),
    ("math", "import math"),
    ("time", "import time"),
    ("datetime", "import datetime"),
    ("pathlib", "import pathlib"),
    ("Path", "from pathlib import Path"),
    ("tempfile", "import tempfile"),
    ("io", "import io"),
    ("copy", "import copy"),
    ("deepcopy", "from copy import deepcopy"),
    ("defaultdict", "from collections import defaultdict"),
    ("OrderedDict", "from collections import OrderedDict"),
    ("Counter", "from collections import Counter"),
    ("namedtuple", "from collections import namedtuple"),
    ("deque", "from collections import deque"),
    ("dataclass", "from dataclasses import dataclass"),
    ("field", "from dataclasses import field"),
    ("asdict", "from dataclasses import asdict"),
    ("pytest", "import pytest"),
    ("mock", "from unittest import mock"),
    ("patch", "from unittest.mock import patch"),
    ("MagicMock", "from unittest.mock import MagicMock"),
    ("csv", "import csv"),
    ("sqlite3", "import sqlite3"),
    ("threading", "import threading"),
    ("functools", "import functools"),
    ("itertools", "import itertools"),
    ("subprocess", "import subprocess"),
    ("hashlib", "import hashlib"),
    ("logging", "import logging"),
    ("typing", "import typing"),
    ("abc", "import abc"),
    ("ABC", "from abc import ABC"),
    ("abstractmethod", "from abc import abstractmethod"),
    ("Enum", "from enum import Enum"),
    ("contextmanager", "from contextlib import contextmanager"),
    ("sleep", "from time import sleep"),
    ("random", "import random"),
    ("collections", "import collections"),
    ("struct", "import struct"),
    ("shutil", "import shutil"),
    ("glob", "import glob"),
    ("argparse", "import argparse"),
    ("traceback", "import traceback"),
    ("warnings", "import warnings"),
    ("inspect", "import inspect"),
    ("textwrap", "import textwrap"),
    ("uuid", "import uuid"),
    ("decimal", "import decimal"),
    ("Decimal", "from decimal import Decimal"),
];

// ============================================================================
// Top-level dispatcher
// ============================================================================

/// Dispatch to the correct Python analyzer based on error type.
///
/// Returns `Some(Diagnosis)` if an analyzer handled the error, `None` otherwise.
/// The generic exception analyzer always returns a diagnosis as the floor.
pub fn diagnose_python(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
    _api_surface: Option<&()>, // Placeholder for ApiSurface type from Phase 1
) -> Option<Diagnosis> {
    let error_type = error.error_type.as_str();
    let message = &error.message;

    // Priority-ordered dispatch (matching FastEdit base.py diagnose() order)
    // 1. Scope errors
    if error_type == "UnboundLocalError"
        || message.contains("referenced before assignment")
        || message.contains("cannot access local variable")
    {
        if let Some(d) = analyze_unbound_local(error, source, tree) {
            return Some(d);
        }
    }

    // 2-3. Type errors (serialization first, then callable, then general)
    if error_type == "TypeError" {
        if message.contains("not JSON serializable") {
            if let Some(d) = analyze_type_error_serialization(error, source, tree) {
                return Some(d);
            }
        }
        if message.contains("is not callable") {
            if let Some(d) = analyze_type_error_callable(error, source, tree) {
                return Some(d);
            }
        }
        // General TypeError at lower priority (after name/import/attribute)
    }

    // 4. NameError
    if error_type == "NameError" || (error_type != "UnboundLocalError" && message.contains("is not defined")) {
        if let Some(d) = analyze_name_error(error, source, tree) {
            return Some(d);
        }
    }

    // 5. ImportError / CircularImport
    if error_type == "ImportError" || error_type == "ModuleNotFoundError" {
        if message.contains("partially initialized module") {
            if let Some(d) = analyze_circular_import(error, source, tree) {
                return Some(d);
            }
        }
        if let Some(d) = analyze_import_error(error, source, tree) {
            return Some(d);
        }
    }

    // 6. AttributeError
    if error_type == "AttributeError" || message.contains("has no attribute") {
        if let Some(d) = analyze_attribute_error(error, source, tree) {
            return Some(d);
        }
    }

    // 9. KeyError
    if error_type == "KeyError" {
        if let Some(d) = analyze_key_error(error, source, tree) {
            return Some(d);
        }
    }

    // 8. IndexError
    if error_type == "IndexError" {
        if let Some(d) = analyze_index_error(error, source, tree) {
            return Some(d);
        }
    }

    // 16. UnicodeError
    if error_type == "UnicodeError"
        || error_type == "UnicodeDecodeError"
        || error_type == "UnicodeEncodeError"
        || message.contains("codec can't")
    {
        if let Some(d) = analyze_unicode_error(error, source, tree) {
            return Some(d);
        }
    }

    // 7. ValueError
    if error_type == "ValueError" {
        if let Some(d) = analyze_value_error(error, source, tree) {
            return Some(d);
        }
    }

    // 10. ZeroDivisionError
    if error_type == "ZeroDivisionError"
        || message.contains("division by zero")
    {
        if let Some(d) = analyze_zero_division(error, source, tree) {
            return Some(d);
        }
    }

    // 15. OSError family
    if error_type == "OSError"
        || error_type == "FileNotFoundError"
        || error_type == "PermissionError"
        || error_type == "IsADirectoryError"
        || error_type == "NotADirectoryError"
        || error_type == "FileExistsError"
    {
        if let Some(d) = analyze_os_error(error, source, tree) {
            return Some(d);
        }
    }

    // 11. RecursionError
    if error_type == "RecursionError" || message.contains("maximum recursion depth") {
        if let Some(d) = analyze_recursion_error(error, source, tree) {
            return Some(d);
        }
    }

    // 12. StopIteration
    if error_type == "StopIteration" || error_type == "StopAsyncIteration" {
        if let Some(d) = analyze_stop_iteration(error, source, tree) {
            return Some(d);
        }
    }

    // 17. SyntaxError
    if error_type == "SyntaxError" || message.contains("invalid syntax") || message.contains("expected ':'") {
        if let Some(d) = analyze_syntax_error(error, source, tree) {
            return Some(d);
        }
    }

    // 18. IndentationError / TabError
    if error_type == "IndentationError"
        || error_type == "TabError"
        || message.contains("unexpected indent")
        || message.contains("inconsistent use of tabs")
        || message.contains("unindent does not match")
        || message.contains("expected an indented block")
    {
        if let Some(d) = analyze_indentation_error(error, source, tree) {
            return Some(d);
        }
    }

    // 13. AssertionError
    if error_type == "AssertionError" {
        if let Some(d) = analyze_assertion_error(error, source, tree) {
            return Some(d);
        }
    }

    // 14. NotImplementedError
    if error_type == "NotImplementedError" {
        if let Some(d) = analyze_not_implemented(error, source, tree) {
            return Some(d);
        }
    }

    // 20. General TypeError (catch-all for remaining TypeError patterns)
    if error_type == "TypeError" {
        if let Some(d) = analyze_type_error_general(error, source, tree) {
            return Some(d);
        }
    }

    // 21. RuntimeError
    if error_type == "RuntimeError" {
        if let Some(d) = analyze_runtime_error(error, source, tree) {
            return Some(d);
        }
    }

    // 22. Generic exception (floor -- always returns Some)
    analyze_generic_exception(error, source, tree)
}

// ============================================================================
// Tree-sitter helper functions
// ============================================================================

/// Find the tree-sitter node for a function definition by name.
///
/// Walks the tree looking for `function_definition` or `decorated_definition`
/// nodes whose name matches.
fn find_function_node<'a>(
    tree: &'a Tree,
    source: &'a str,
    func_name: &str,
) -> Option<tree_sitter::Node<'a>> {
    find_function_node_recursive(tree.root_node(), source, func_name)
}

fn find_function_node_recursive<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a str,
    func_name: &str,
) -> Option<tree_sitter::Node<'a>> {
    let kind = node.kind();

    if kind == "function_definition" {
        // Check the name child
        if let Some(name_node) = node.child_by_field_name("name") {
            let name = &source[name_node.byte_range()];
            if name == func_name {
                return Some(node);
            }
        }
    }

    // Also check decorated_definition wrapping a function
    if kind == "decorated_definition" {
        if let Some(def_node) = node.child_by_field_name("definition") {
            if def_node.kind() == "function_definition" {
                if let Some(name_node) = def_node.child_by_field_name("name") {
                    let name = &source[name_node.byte_range()];
                    if name == func_name {
                        return Some(def_node);
                    }
                }
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_function_node_recursive(child, source, func_name) {
            return Some(found);
        }
    }

    None
}

/// Find the innermost enclosing `function_definition` node for a given 1-indexed line number.
///
/// Walks the tree depth-first, finding the tightest function definition whose span
/// contains the target line. Returns the function node and its name.
fn find_enclosing_function_at_line<'a>(
    tree: &'a Tree,
    source: &'a str,
    line: usize,
) -> Option<(tree_sitter::Node<'a>, String)> {
    let row = line.saturating_sub(1); // tree-sitter uses 0-indexed rows
    find_enclosing_func_recursive(tree.root_node(), source, row)
}

fn find_enclosing_func_recursive<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a str,
    row: usize,
) -> Option<(tree_sitter::Node<'a>, String)> {
    let mut best: Option<(tree_sitter::Node<'a>, String)> = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let start = child.start_position().row;
        let end = child.end_position().row;

        // Only descend into nodes whose range contains the target row
        if row < start || row > end {
            continue;
        }

        if child.kind() == "function_definition" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = source[name_node.byte_range()].to_string();
                best = Some((child, name));
            }
        } else if child.kind() == "decorated_definition" {
            if let Some(def_node) = child.child_by_field_name("definition") {
                if def_node.kind() == "function_definition" {
                    let def_start = def_node.start_position().row;
                    let def_end = def_node.end_position().row;
                    if row >= def_start && row <= def_end {
                        if let Some(name_node) = def_node.child_by_field_name("name") {
                            let name = source[name_node.byte_range()].to_string();
                            best = Some((def_node, name));
                        }
                    }
                }
            }
        }

        // Recurse to find a tighter (more nested) enclosing function
        if let Some(inner) = find_enclosing_func_recursive(child, source, row) {
            best = Some(inner);
        }
    }

    best
}

/// Get the indentation of a line (the leading whitespace).
fn get_line_indent(source: &str, line_num: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if line_num == 0 || line_num > lines.len() {
        return String::new();
    }
    let line = lines[line_num - 1];
    let trimmed = line.trim_start();
    line[..line.len() - trimmed.len()].to_string()
}

/// Get the body indent of a function (indent of first statement in body).
fn get_function_body_indent(source: &str, tree: &Tree, func_name: &str) -> Option<String> {
    let func_node = find_function_node(tree, source, func_name)?;
    let body = func_node.child_by_field_name("body")?;

    // Get the first statement in the body
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.is_named() {
            let start_line = child.start_position().row + 1; // 1-indexed
            return Some(get_line_indent(source, start_line));
        }
    }

    // Fallback: function indent + 4 spaces
    let func_line = func_node.start_position().row + 1;
    let func_indent = get_line_indent(source, func_line);
    Some(format!("{}    ", func_indent))
}

/// Find the line number of the first statement in a function body.
///
/// Tries to find the first named child in the body node. If the body has no
/// named children (e.g., parse errors, unusual formatting), falls back to
/// the line after the function definition (`def` line + 1).
fn find_function_body_start(source: &str, tree: &Tree, func_name: &str) -> Option<usize> {
    let func_node = find_function_node(tree, source, func_name)?;
    let body = func_node.child_by_field_name("body");

    // Primary path: find the first named child in the body
    if let Some(ref body_node) = body {
        let mut cursor = body_node.walk();
        for child in body_node.children(&mut cursor) {
            if child.is_named() {
                return Some(child.start_position().row + 1); // 1-indexed
            }
        }
    }

    // Fallback: use the line after the function definition.
    // The `def func(...):` line is at func_node.start_position().row (0-indexed),
    // so the body starts at row + 2 (1-indexed, next line).
    Some(func_node.start_position().row + 2)
}

/// Check if a function already has a `global <var>` declaration.
fn has_global_declaration(source: &str, tree: &Tree, func_name: &str, var_name: &str) -> bool {
    let func_node = match find_function_node(tree, source, func_name) {
        Some(n) => n,
        None => return false,
    };
    has_global_in_node(func_node, source, var_name)
}

fn has_global_in_node(node: tree_sitter::Node, source: &str, var_name: &str) -> bool {
    if node.kind() == "global_statement" {
        let text = &source[node.byte_range()];
        // Check if var_name is among the names in the global statement
        let re = Regex::new(&format!(r"\bglobal\b.*\b{}\b", regex::escape(var_name))).ok();
        if let Some(re) = re {
            if re.is_match(text) {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip nested function definitions
        if child.kind() == "function_definition" || child.kind() == "decorated_definition" {
            continue;
        }
        if has_global_in_node(child, source, var_name) {
            return true;
        }
    }
    false
}

/// Check if a variable is assigned in the direct body of a function
/// (excluding nested function definitions).
fn var_assigned_in_function(
    node: tree_sitter::Node,
    source: &str,
    var_name: &str,
) -> bool {
    let kind = node.kind();

    // Direct assignment: x = ...
    if kind == "assignment" {
        if let Some(left) = node.child_by_field_name("left") {
            let text = &source[left.byte_range()];
            if text == var_name {
                return true;
            }
        }
    }

    // Augmented assignment: x += ...
    if kind == "augmented_assignment" {
        if let Some(left) = node.child_by_field_name("left") {
            let text = &source[left.byte_range()];
            if text == var_name {
                return true;
            }
        }
    }

    // For loop target: for x in ...
    if kind == "for_statement" {
        if let Some(left) = node.child_by_field_name("left") {
            let text = &source[left.byte_range()];
            if text == var_name {
                return true;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip nested function/class definitions
        if child.kind() == "function_definition"
            || child.kind() == "decorated_definition"
            || child.kind() == "class_definition"
        {
            continue;
        }
        if var_assigned_in_function(child, source, var_name) {
            return true;
        }
    }

    false
}

/// Find the first function definition that assigns to `var_name`.
///
/// This is a last-resort fallback for when the error has no line number and
/// no function name (e.g., single-line CLI error). It scans all top-level and
/// nested function definitions looking for one that assigns to the variable.
/// Returns the function name if found.
fn find_function_assigning_var(tree: &Tree, source: &str, var_name: &str) -> Option<String> {
    find_function_assigning_var_recursive(tree.root_node(), source, var_name)
}

fn find_function_assigning_var_recursive(
    node: tree_sitter::Node,
    source: &str,
    var_name: &str,
) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        // Check function definitions
        let func_node = if kind == "function_definition" {
            Some(child)
        } else if kind == "decorated_definition" {
            child
                .child_by_field_name("definition")
                .filter(|d| d.kind() == "function_definition")
        } else {
            None
        };

        if let Some(func) = func_node {
            if let Some(name_node) = func.child_by_field_name("name") {
                let name = &source[name_node.byte_range()];
                // Check if this function assigns to the variable
                if let Some(body) = func.child_by_field_name("body") {
                    if var_assigned_in_function(body, source, var_name) {
                        return Some(name.to_string());
                    }
                }
            }
            // Also check nested functions inside this one
            if let Some(body) = func.child_by_field_name("body") {
                if let Some(inner) =
                    find_function_assigning_var_recursive(body, source, var_name)
                {
                    return Some(inner);
                }
            }
        } else {
            // Recurse into other nodes (e.g., class bodies, if blocks)
            if let Some(found) =
                find_function_assigning_var_recursive(child, source, var_name)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Collect module-scope variable names from the source.
fn collect_module_scope_vars(tree: &Tree, source: &str) -> Vec<String> {
    let root = tree.root_node();
    let mut vars = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "expression_statement" {
            // Check for assignment inside expression statement
            let mut inner_cursor = child.walk();
            for inner in child.children(&mut inner_cursor) {
                if inner.kind() == "assignment" {
                    if let Some(left) = inner.child_by_field_name("left") {
                        if left.kind() == "identifier" {
                            let name = &source[left.byte_range()];
                            vars.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    vars
}

/// Find the last import line number in the source.
fn find_last_import_line(source: &str) -> Option<usize> {
    let mut last_import = None;
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
            last_import = Some(idx + 1); // 1-indexed
        }
    }
    last_import
}

/// Check if a specific import already exists in the source.
fn has_import(source: &str, import_line: &str) -> bool {
    let import_trimmed = import_line.trim();
    for line in source.lines() {
        if line.trim() == import_trimmed {
            return true;
        }
    }
    false
}

/// Find subscript usage of a variable (e.g., `d[key]`) and return the
/// dict variable name and the subscript expression text.
fn find_dict_subscript(
    node: tree_sitter::Node,
    source: &str,
) -> Option<(String, String, usize)> {
    if node.kind() == "subscript" {
        if let Some(value) = node.child_by_field_name("value") {
            if value.kind() == "identifier" {
                let dict_name = source[value.byte_range()].to_string();
                if let Some(subscript) = node.child_by_field_name("subscript") {
                    let key_text = source[subscript.byte_range()].to_string();
                    let line = node.start_position().row + 1;
                    return Some((dict_name, key_text, line));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_dict_subscript(child, source) {
            return Some(found);
        }
    }
    None
}

/// Find `next(...)` calls inside a function.
fn find_next_call(
    node: tree_sitter::Node,
    source: &str,
) -> Option<usize> {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            let text = &source[func.byte_range()];
            if text == "next" {
                return Some(node.start_position().row + 1);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_next_call(child, source) {
            return Some(found);
        }
    }
    None
}

/// Check if a function has a recursive self-call.
fn has_recursive_call(node: tree_sitter::Node, source: &str, func_name: &str) -> bool {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            let text = &source[func.byte_range()];
            if text == func_name {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if has_recursive_call(child, source, func_name) {
            return true;
        }
    }
    false
}

/// Check if a function body has any return statement.
fn has_return_statement(node: tree_sitter::Node) -> bool {
    if node.kind() == "return_statement" {
        return true;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Skip nested functions
        if child.kind() == "function_definition" || child.kind() == "decorated_definition" {
            continue;
        }
        if has_return_statement(child) {
            return true;
        }
    }
    false
}

/// Check if a function has an `open()` call without `encoding=` kwarg.
fn find_bare_open_call(
    node: tree_sitter::Node,
    source: &str,
) -> Option<usize> {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            let text = &source[func.byte_range()];
            if text == "open" {
                // Check if encoding= keyword is present
                if let Some(args) = node.child_by_field_name("arguments") {
                    let args_text = &source[args.byte_range()];
                    if !args_text.contains("encoding") {
                        return Some(node.start_position().row + 1);
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_bare_open_call(child, source) {
            return Some(found);
        }
    }
    None
}

/// Find an `open()` call in write mode ('w', 'a', 'x', 'wb', etc.) in the tree.
///
/// Returns the 1-indexed line number of the open call if found.
/// Write modes are any mode string containing 'w', 'a', or 'x'.
fn find_write_mode_open_call(
    node: tree_sitter::Node,
    source: &str,
) -> Option<usize> {
    if node.kind() == "call" {
        if let Some(func) = node.child_by_field_name("function") {
            let text = &source[func.byte_range()];
            if text == "open" {
                if let Some(args) = node.child_by_field_name("arguments") {
                    // Check positional arguments for a write mode
                    let mut cursor = args.walk();
                    let mut positional_idx = 0;
                    let mut has_write_mode = false;

                    for child in args.children(&mut cursor) {
                        // Skip non-named children (commas, parens)
                        if !child.is_named() {
                            continue;
                        }
                        // Check keyword arguments for mode=
                        if child.kind() == "keyword_argument" {
                            if let Some(name_node) = child.child_by_field_name("name") {
                                let kw_name = &source[name_node.byte_range()];
                                if kw_name == "mode" {
                                    if let Some(val) = child.child_by_field_name("value") {
                                        let val_text = &source[val.byte_range()];
                                        let unquoted = val_text.trim_matches('\'').trim_matches('"');
                                        if unquoted.contains('w') || unquoted.contains('a') || unquoted.contains('x') {
                                            has_write_mode = true;
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        // Second positional arg is the mode
                        if positional_idx == 1 {
                            let arg_text = &source[child.byte_range()];
                            let unquoted = arg_text.trim_matches('\'').trim_matches('"');
                            if unquoted.contains('w') || unquoted.contains('a') || unquoted.contains('x') {
                                has_write_mode = true;
                            }
                        }
                        positional_idx += 1;
                    }

                    if has_write_mode {
                        return Some(node.start_position().row + 1);
                    }
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_write_mode_open_call(child, source) {
            return Some(found);
        }
    }
    None
}

/// Find the first function definition that references a given identifier name.
///
/// Scans all top-level and nested function definitions looking for one whose
/// body contains an `identifier` node matching the target name. Returns the
/// function name and the line of its first body statement (for inserting
/// the local import).
fn find_function_using_name(
    tree: &Tree,
    source: &str,
    target_name: &str,
) -> Option<(String, usize)> {
    find_function_using_name_recursive(tree.root_node(), source, target_name)
}

fn find_function_using_name_recursive(
    node: tree_sitter::Node,
    source: &str,
    target_name: &str,
) -> Option<(String, usize)> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();

        let func_node = if kind == "function_definition" {
            Some(child)
        } else if kind == "decorated_definition" {
            child
                .child_by_field_name("definition")
                .filter(|d| d.kind() == "function_definition")
        } else {
            None
        };

        if let Some(func) = func_node {
            if let Some(name_node) = func.child_by_field_name("name") {
                let name = &source[name_node.byte_range()];
                if let Some(body) = func.child_by_field_name("body") {
                    if node_contains_identifier(body, source, target_name) {
                        // Find the first body statement line
                        let mut body_cursor = body.walk();
                        let body_start = body.children(&mut body_cursor)
                            .filter(|c| c.is_named())
                            .map(|c| c.start_position().row + 1)
                            .next()
                            .unwrap_or(func.start_position().row + 2);
                        return Some((name.to_string(), body_start));
                    }
                }
            }
            // Also check nested functions
            if let Some(body) = func.child_by_field_name("body") {
                if let Some(found) = find_function_using_name_recursive(body, source, target_name) {
                    return Some(found);
                }
            }
        } else {
            // Recurse into other nodes (class bodies, if blocks, etc.)
            if let Some(found) = find_function_using_name_recursive(child, source, target_name) {
                return Some(found);
            }
        }
    }
    None
}

/// Check if a tree-sitter node contains an identifier matching `target_name`.
fn node_contains_identifier(node: tree_sitter::Node, source: &str, target_name: &str) -> bool {
    if node.kind() == "identifier" {
        let text = &source[node.byte_range()];
        if text == target_name {
            return true;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if node_contains_identifier(child, source, target_name) {
            return true;
        }
    }
    false
}

/// Find the line number of a top-level import statement that imports a
/// specific name from a specific module.
///
/// Searches for `from <module> import <name>` or `import <module>` at the
/// module level. Returns the 1-indexed line number if found.
fn find_top_level_import_line(source: &str, tree: &Tree, module: &str, name: &str) -> Option<usize> {
    let root = tree.root_node();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        // import_from_statement: from X import Y
        if child.kind() == "import_from_statement" {
            let text = &source[child.byte_range()];
            // Check if this imports from the right module and includes the name
            let pattern = format!("from {} import", module);
            if text.contains(&pattern) && text.contains(name) {
                return Some(child.start_position().row + 1);
            }
            // Also check without spaces variations
            let trimmed = text.trim();
            if trimmed.starts_with("from ")
                && trimmed.contains(module)
                && trimmed.contains(name)
            {
                return Some(child.start_position().row + 1);
            }
        }
        // import_statement: import X
        if child.kind() == "import_statement" {
            let text = &source[child.byte_range()];
            if text.contains(module) && module == name {
                return Some(child.start_position().row + 1);
            }
        }
    }
    None
}

/// Find assert statements in a function.
fn find_assert_statement(
    node: tree_sitter::Node,
    source: &str,
) -> Option<(usize, String)> {
    if node.kind() == "assert_statement" {
        let line = node.start_position().row + 1;
        let text = source[node.byte_range()].to_string();
        return Some((line, text));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" || child.kind() == "decorated_definition" {
            continue;
        }
        if let Some(found) = find_assert_statement(child, source) {
            return Some(found);
        }
    }
    None
}

/// Check if a function body has a `raise NotImplementedError` statement.
fn has_raise_not_implemented(node: tree_sitter::Node, source: &str) -> bool {
    if node.kind() == "raise_statement" {
        let text = &source[node.byte_range()];
        if text.contains("NotImplementedError") {
            return true;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "function_definition" || child.kind() == "decorated_definition" {
            continue;
        }
        if has_raise_not_implemented(child, source) {
            return true;
        }
    }
    false
}

// ============================================================================
// Analyzer #1: UnboundLocalError (ScopeAnalyzer)
// ============================================================================

/// Resolve the function name from the error context.
///
/// Tries the explicit `function_name` field first, then falls back to finding
/// the enclosing function at the error line using tree-sitter.
fn resolve_function_name(error: &ParsedError, source: &str, tree: &Tree) -> Option<String> {
    // Use explicit function name if provided and non-empty
    if let Some(ref name) = error.function_name {
        if !name.is_empty() {
            return Some(name.clone());
        }
    }

    // Fall back to finding the enclosing function from the error line
    if let Some(line) = error.line {
        if let Some((_node, name)) = find_enclosing_function_at_line(tree, source, line) {
            return Some(name);
        }
    }

    None
}

/// Create a fallback UnboundLocalError diagnosis without a fix.
///
/// Used when the enclosing function cannot be determined from the error context.
fn make_unbound_local_fallback(var_name: &str, error: &ParsedError) -> Diagnosis {
    Diagnosis {
        language: "python".to_string(),
        error_code: "UnboundLocalError".to_string(),
        message: format!(
            "Variable '{}' is modified inside a function without `global {}`. \
             Add `global {}` at the top of the function body.",
            var_name, var_name, var_name
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix: None,
    }
}

/// Analyze UnboundLocalError: missing `global` declaration.
///
/// When a variable is defined at module scope and modified inside a function
/// without a `global` declaration, Python raises UnboundLocalError.
/// Fix: inject `global <var>` at the top of the function body.
fn analyze_unbound_local(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let var_name = extract_variable_name(&error.message)?;

    // Resolve the function name: use the one from the error if available,
    // otherwise find the enclosing function from the error line number.
    // Last resort: scan all functions for one that assigns to the variable.
    let resolved_func_name = resolve_function_name(error, source, tree)
        .or_else(|| find_function_assigning_var(tree, source, &var_name));

    let func_name = match resolved_func_name {
        Some(ref name) => name.as_str(),
        None => {
            // Cannot determine function -- return diagnosis without fix
            return Some(make_unbound_local_fallback(&var_name, error));
        }
    };

    // Verify the variable exists at module scope
    let module_vars = collect_module_scope_vars(tree, source);
    let var_at_module = module_vars.iter().any(|v| v == &var_name);

    // Verify the variable is assigned in the function (excluding nested funcs)
    let func_node = find_function_node(tree, source, func_name);
    let assigned_in_func = func_node
        .map(|n| var_assigned_in_function(n, source, &var_name))
        .unwrap_or(false);

    // Check if already has global declaration
    if has_global_declaration(source, tree, func_name, &var_name) {
        return None;
    }

    // Build the fix
    let body_start = find_function_body_start(source, tree, func_name)
        .or_else(|| {
            // Secondary fallback: if find_function_body_start failed entirely
            // (e.g., function node not found by name after rename), compute from
            // the function node we already resolved.
            func_node.map(|n| n.start_position().row + 2) // def line (0-indexed) + 2 = next line (1-indexed)
        });
    let indent = get_function_body_indent(source, tree, func_name)
        .unwrap_or_else(|| "    ".to_string());

    let confidence = if var_at_module && assigned_in_func {
        FixConfidence::High
    } else {
        FixConfidence::Medium
    };

    // body_start is guaranteed non-None here when func_name is resolved, because
    // find_function_body_start has its own fallback, plus the .or_else above.
    let fix = body_start.map(|line| Fix {
        description: format!("Inject `global {}` at top of `{}()`", var_name, func_name),
        edits: vec![TextEdit {
            line,
            column: None,
            kind: EditKind::InsertBefore,
            new_text: format!("{}global {}", indent, var_name),
        }],
    });

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "UnboundLocalError".to_string(),
        message: format!(
            "Variable '{}' is defined at module scope but modified inside '{}()' \
             without `global {}`. Add `global {}` at the top of the function.",
            var_name, func_name, var_name, var_name
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence,
        fix,
    })
}

// ============================================================================
// Analyzer #2: TypeError (not callable)
// ============================================================================

/// Analyze TypeError when an object is not callable.
///
/// Common cause: calling a property as a method (e.g., `obj.prop()`).
/// Fix: remove the `()` from the call if the target is a property.
fn analyze_type_error_callable(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    if !error.message.contains("is not callable") {
        return None;
    }

    // Extract the type name from "'X' object is not callable"
    let type_re = Regex::new(r"'(\w+)' object is not callable").ok()?;
    let type_name = type_re
        .captures(&error.message)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    // Try to find the obj.attr() pattern in the offending line or traceback
    let attr_re = Regex::new(r"(\w+)\.(\w+)\(\)").ok()?;
    let attr_match = error
        .offending_line
        .as_deref()
        .and_then(|line| attr_re.captures(line))
        .or_else(|| attr_re.captures(&error.raw_text));

    let fix = if let Some(caps) = &attr_match {
        let _obj = caps.get(1).unwrap().as_str();
        let attr = caps.get(2).unwrap().as_str();
        let full_match = caps.get(0).unwrap().as_str();

        // Find the line in source where this pattern appears
        let fix_line = source
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains(full_match))
            .map(|(idx, line)| {
                let new_line = line.replace(&format!(".{}()", attr), &format!(".{}", attr));
                TextEdit {
                    line: idx + 1,
                    column: None,
                    kind: EditKind::ReplaceLine,
                    new_text: new_line,
                }
            });

        fix_line.map(|edit| Fix {
            description: format!(
                "Remove `()` from `.{}()` -- it is a property, not a method",
                attr
            ),
            edits: vec![edit],
        })
    } else {
        None
    };

    let type_desc = type_name
        .as_deref()
        .unwrap_or("object");

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "TypeError".to_string(),
        message: format!(
            "'{}' is not callable. If accessing a property, remove the parentheses `()`.",
            type_desc
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: if fix.is_some() {
            FixConfidence::High
        } else {
            FixConfidence::Medium
        },
        fix,
    })
}

// ============================================================================
// Analyzer #3: TypeError (not JSON serializable)
// ============================================================================

/// Analyze TypeError for JSON serialization errors.
///
/// Common cause: passing a dataclass/pydantic model to `json.dumps()` or
/// `jsonify()` without converting to dict first.
/// Fix: wrap with `asdict()` for dataclasses or `.dict()` for pydantic.
fn analyze_type_error_serialization(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    if !error.message.contains("not JSON serializable") {
        return None;
    }

    // Extract the type name: "Object of type X is not JSON serializable"
    let type_re = Regex::new(r"Object of type (\w+) is not JSON serializable").ok()?;
    let type_name = type_re
        .captures(&error.message)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    let is_dataclass = source.contains("@dataclass") || source.contains("from dataclasses import");

    let fix = if is_dataclass {
        // Find jsonify() or json.dumps() calls and wrap their argument with asdict()
        let json_call_re = Regex::new(r"(jsonify|json\.dumps)\((\w+)\)").ok()?;
        let mut edits = Vec::new();

        for (idx, line) in source.lines().enumerate() {
            if let Some(caps) = json_call_re.captures(line) {
                let func = caps.get(1).unwrap().as_str();
                let arg = caps.get(2).unwrap().as_str();
                // Don't wrap if already wrapped
                if line.contains("asdict") {
                    continue;
                }
                // Don't wrap builtins
                if matches!(arg, "True" | "False" | "None" | "dict" | "list") {
                    continue;
                }
                let new_line =
                    line.replace(&format!("{}({})", func, arg), &format!("{}(asdict({}))", func, arg));
                edits.push(TextEdit {
                    line: idx + 1,
                    column: None,
                    kind: EditKind::ReplaceLine,
                    new_text: new_line,
                });
            }
        }

        // Ensure asdict import exists
        if !source.contains("asdict") {
            if let Some(import_caps) =
                Regex::new(r"from\s+dataclasses\s+import\s+(.+)")
                    .ok()
                    .and_then(|re| {
                        source.lines().enumerate().find_map(|(idx, line)| {
                            re.captures(line).map(|c| (idx, c))
                        })
                    })
            {
                let (idx, caps) = import_caps;
                let m = caps.get(1).unwrap();
                let existing = m.as_str().trim();
                if !existing.contains("asdict") {
                    // Use match positions to replace only the import-list portion,
                    // avoiding a naive str::replace that would match "dataclass"
                    // inside "dataclasses" and produce "from dataclass, asdictes ...".
                    let line = source.lines().nth(idx).unwrap_or("");
                    let start = m.start();
                    let end = m.end();
                    let new_line = format!(
                        "{}{}, asdict{}",
                        &line[..start],
                        existing.trim_end(),
                        &line[end..],
                    );
                    edits.push(TextEdit {
                        line: idx + 1,
                        column: None,
                        kind: EditKind::ReplaceLine,
                        new_text: new_line,
                    });
                }
            }
        }

        if edits.is_empty() {
            None
        } else {
            Some(Fix {
                description: "Wrap dataclass objects with `asdict()` before JSON serialization"
                    .to_string(),
                edits,
            })
        }
    } else {
        None
    };

    let type_desc = type_name.as_deref().unwrap_or("object");

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "TypeError".to_string(),
        message: format!(
            "Object of type '{}' is not JSON serializable. \
             Use `asdict()` for dataclasses or `.dict()` for pydantic models \
             before passing to `json.dumps()` or `jsonify()`.",
            type_desc
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: if fix.is_some() {
            FixConfidence::High
        } else {
            FixConfidence::Medium
        },
        fix,
    })
}

// ============================================================================
// Analyzer #4: NameError (not defined)
// ============================================================================

/// Analyze NameError: name not defined.
///
/// Fix: inject the missing import statement from stdlib table or api-surface.
fn analyze_name_error(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    if !error.message.contains("is not defined") {
        return None;
    }

    let var_name = extract_variable_name(&error.message)?;

    // Look up in stdlib imports table
    let import_line = STDLIB_IMPORTS
        .iter()
        .find(|(name, _)| *name == var_name)
        .map(|(_, imp)| *imp);

    let fix = if let Some(imp) = import_line {
        if has_import(source, imp) {
            None
        } else {
            let insert_line = find_last_import_line(source)
                .map(|l| l + 1)
                .unwrap_or(1);
            Some(Fix {
                description: format!("Add `{}` (stdlib auto-import)", imp),
                edits: vec![TextEdit {
                    line: insert_line,
                    column: None,
                    kind: EditKind::InsertBefore,
                    new_text: imp.to_string(),
                }],
            })
        }
    } else {
        None
    };

    let confidence = if fix.is_some() {
        FixConfidence::High
    } else {
        FixConfidence::Low
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "NameError".to_string(),
        message: format!(
            "Name '{}' is not defined. {}",
            var_name,
            if let Some(imp) = import_line {
                format!("Add `{}`.", imp)
            } else {
                "Check spelling or add the missing import.".to_string()
            }
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence,
        fix,
    })
}

// ============================================================================
// Analyzer #5: ImportError
// ============================================================================

/// Analyze ImportError / ModuleNotFoundError.
///
/// Handles:
/// - `cannot import name 'X' from 'Y'`: suggests correct module
/// - `No module named 'X'`: suggests installation or stdlib import
fn analyze_import_error(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;

    // Sub-pattern: cannot import name 'X' from 'Y'
    if let Some(caps) = Regex::new(r"cannot import name '(\w+)' from '([\w.]+)'")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let bad_name = caps.get(1).unwrap().as_str();
        let wrong_module = caps.get(2).unwrap().as_str();

        // Check if we know the correct import path from stdlib
        let correct_import = STDLIB_IMPORTS
            .iter()
            .find(|(name, _)| *name == bad_name)
            .map(|(_, imp)| *imp);

        let fix = correct_import.and_then(|imp| {
            // Build a fix that replaces the bad import line
            let bad_pattern = format!("from {} import", wrong_module);
            source
                .lines()
                .enumerate()
                .find(|(_, line)| line.contains(&bad_pattern) && line.contains(bad_name))
                .map(|(idx, line)| {
                    // If the line imports multiple names, just remove the bad one
                    let names_part = line.split("import").nth(1).unwrap_or("").trim();
                    let names: Vec<&str> = names_part.split(',').map(|s| s.trim()).collect();
                    let mut edits = Vec::new();

                    if names.len() > 1 {
                        // Remove bad_name from the multi-import, keep the rest
                        let remaining: Vec<&str> =
                            names.iter().filter(|n| **n != bad_name).copied().collect();
                        let indent = get_line_indent(source, idx + 1);
                        let new_line =
                            format!("{}from {} import {}", indent, wrong_module, remaining.join(", "));
                        edits.push(TextEdit {
                            line: idx + 1,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: new_line,
                        });
                    } else {
                        // Replace the entire line
                        edits.push(TextEdit {
                            line: idx + 1,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: imp.to_string(),
                        });
                    }

                    Fix {
                        description: format!(
                            "Rewrite import: `{}` is not in '{}', use `{}`",
                            bad_name, wrong_module, imp
                        ),
                        edits,
                    }
                })
        });

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "ImportError".to_string(),
            message: format!(
                "Cannot import '{}' from '{}'. {}",
                bad_name,
                wrong_module,
                correct_import
                    .map(|i| format!("Use `{}` instead.", i))
                    .unwrap_or_else(|| "Check the module's exports.".to_string())
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: if fix.is_some() {
                FixConfidence::High
            } else {
                FixConfidence::Medium
            },
            fix,
        });
    }

    // Sub-pattern: No module named 'X'
    if let Some(caps) = Regex::new(r"No module named '([\w.]+)'")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let missing = caps.get(1).unwrap().as_str();

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "ImportError".to_string(),
            message: format!(
                "Module '{}' is not installed or not on the import path. \
                 Install the package or check the import spelling.",
                missing
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "ImportError".to_string(),
        message: format!("Import error: {}", msg),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #6: AttributeError
// ============================================================================

/// Analyze AttributeError: object has no attribute.
///
/// Without an api-surface, provides a diagnostic hint.
/// With api-surface (future), does fuzzy matching to suggest correct name.
fn analyze_attribute_error(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    if !error.message.contains("has no attribute") {
        return None;
    }

    // Extract receiver type and bad attribute from error message
    let obj_re = Regex::new(r"'([\w.]+)' object has no attribute '(\w+)'").ok()?;
    let mod_re = Regex::new(r"module '([\w.]+)' has no attribute '(\w+)'").ok()?;

    let (receiver, bad_attr) = if let Some(caps) = obj_re.captures(&error.message) {
        (
            caps.get(1).unwrap().as_str().to_string(),
            caps.get(2).unwrap().as_str().to_string(),
        )
    } else if let Some(caps) = mod_re.captures(&error.message) {
        (
            caps.get(1).unwrap().as_str().to_string(),
            caps.get(2).unwrap().as_str().to_string(),
        )
    } else {
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "AttributeError".to_string(),
            message: format!("AttributeError: {}", error.message),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "AttributeError".to_string(),
        message: format!(
            "'{}' has no attribute '{}'. Check spelling or verify the API. \
             Use `--api-surface` for auto-correction via fuzzy matching.",
            receiver, bad_attr
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix: None,
    })
}

// ============================================================================
// Analyzer #7: ValueError
// ============================================================================

/// Analyze ValueError with context-dependent hints.
///
/// Handles common sub-patterns:
/// - invalid literal for int()
/// - not enough / too many values to unpack
/// - substring not found
fn analyze_value_error(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;
    let func = error.function_name.as_deref().unwrap_or("unknown");

    let hint = if msg.contains("invalid literal for int") {
        format!(
            "Invalid literal for int() in '{}()'. \
             Validate the input or wrap with try/except ValueError.",
            func
        )
    } else if msg.contains("not enough values to unpack") {
        format!(
            "Tuple unpack mismatch in '{}()' -- not enough values. \
             Check the source iterable length.",
            func
        )
    } else if msg.contains("too many values to unpack") {
        format!(
            "Tuple unpack mismatch in '{}()' -- too many values. \
             Use a catch-all `*rest` or fewer unpack targets.",
            func
        )
    } else if msg.contains("substring not found") {
        format!(
            "Substring not found in '{}()'. Use `str.find()` (returns -1 on miss) \
             instead of `str.index()`, or guard with `in`.",
            func
        )
    } else {
        format!(
            "ValueError in '{}()': {}. Validate the input at the call site.",
            func, msg
        )
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "ValueError".to_string(),
        message: hint,
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #8: IndexError
// ============================================================================

/// Analyze IndexError: list/string/tuple index out of range.
///
/// Provides a hint to add bounds checking. The fix is semantic (requires
/// understanding the intent), so no auto-fix.
fn analyze_index_error(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;
    let func = error.function_name.as_deref().unwrap_or("unknown");

    let kind = if msg.contains("string") {
        "string"
    } else if msg.contains("tuple") {
        "tuple"
    } else {
        "list"
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "IndexError".to_string(),
        message: format!(
            "{} index out of range in '{}()'. \
             Guard the subscript with a length check, or use a slice \
             (`items[:1]` instead of `items[0]`).",
            kind, func
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix: None,
    })
}

// ============================================================================
// Analyzer #9: KeyError
// ============================================================================

/// Analyze KeyError: missing dict key.
///
/// Fix: rewrite `d[key]` to `d.get(key)` when the dict is a known literal
/// and the subscript is a variable (runtime key).
fn analyze_key_error(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    // Try to find dict subscript in the function
    let subscript_info = if !func.is_empty() {
        find_function_node(tree, source, func)
            .and_then(|node| find_dict_subscript(node, source))
    } else {
        find_dict_subscript(tree.root_node(), source)
    };

    if let Some((dict_var, key_expr, line)) = subscript_info {
        // Build fix: replace d[key] with d.get(key)
        let source_line = source.lines().nth(line - 1).unwrap_or("");
        let old_pattern = format!("{}[{}]", dict_var, key_expr);
        let new_pattern = format!("{}.get({})", dict_var, key_expr);

        let fix = if source_line.contains(&old_pattern) {
            let new_line = source_line.replace(&old_pattern, &new_pattern);
            Some(Fix {
                description: format!("Rewrite `{}` to `{}`", old_pattern, new_pattern),
                edits: vec![TextEdit {
                    line,
                    column: None,
                    kind: EditKind::ReplaceLine,
                    new_text: new_line,
                }],
            })
        } else {
            None
        };

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "KeyError".to_string(),
            message: format!(
                "KeyError on `{}[{}]` in '{}()'. \
                 Use `.get({})` with a default value.",
                dict_var, key_expr, func, key_expr
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line,
                column: None,
            }),
            confidence: if fix.is_some() {
                FixConfidence::Medium
            } else {
                FixConfidence::Low
            },
            fix,
        });
    }

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "KeyError".to_string(),
        message: format!(
            "KeyError in '{}()': the key is missing from the dict. \
             Use `dict.get(key)` with a default, or guard with `if key in dict:`.",
            if func.is_empty() { "module" } else { func }
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #10: ZeroDivisionError
// ============================================================================

/// Analyze ZeroDivisionError.
///
/// Provides a hint to guard the denominator. Identifying the exact division
/// expression requires semantic analysis, so the fix is hint-only.
fn analyze_zero_division(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("unknown");

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "ZeroDivisionError".to_string(),
        message: format!(
            "Division / modulo by zero in '{}()'. \
             Guard the denominator with a zero-check, or return a sentinel \
             value when the divisor is zero.",
            func
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix: None,
    })
}

// ============================================================================
// Analyzer #11: RecursionError
// ============================================================================

/// Analyze RecursionError: maximum recursion depth exceeded.
///
/// Checks for:
/// - Recursive self-call in the function
/// - Whether a base case (return before recursion) exists
fn analyze_recursion_error(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    let (has_recursion, missing_base) = if !func.is_empty() {
        if let Some(func_node) = find_function_node(tree, source, func) {
            let recursive = has_recursive_call(func_node, source, func);
            let has_ret = has_return_statement(func_node);
            (recursive, recursive && !has_ret)
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    let suffix = if missing_base {
        " No base-case `return` detected before the recursive call -- \
         add a base case that returns without recursing."
    } else if has_recursion {
        " Verify the base case terminates the recursion."
    } else {
        " Verify that the function has a base case that returns \
         before the recursive call."
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "RecursionError".to_string(),
        message: format!(
            "RecursionError: '{}()' recursively calls itself.{}",
            if func.is_empty() { "unknown" } else { func },
            suffix
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #12: StopIteration
// ============================================================================

/// Analyze StopIteration: iterator exhausted.
///
/// Locates the `next()` call site in the function and suggests using
/// `next(it, default)` with a default value.
fn analyze_stop_iteration(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    let call_site = if !func.is_empty() {
        find_function_node(tree, source, func)
            .and_then(|node| find_next_call(node, source))
    } else {
        find_next_call(tree.root_node(), source)
    };

    let suffix = call_site
        .map(|line| format!(" Call site: next(...) at line {}.", line))
        .unwrap_or_default();

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "StopIteration".to_string(),
        message: format!(
            "StopIteration in '{}()' -- the iterator was exhausted. \
             Use `next(it, default)` with a default value, or wrap in \
             try/except StopIteration.{}",
            if func.is_empty() { "unknown" } else { func },
            suffix
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: call_site.or(error.line).unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix: None,
    })
}

// ============================================================================
// Analyzer #13: AssertionError
// ============================================================================

/// Analyze AssertionError from production code.
///
/// Filters out test_ functions (those are test failures, not production bugs).
/// For production asserts, surfaces the invariant as a hint.
fn analyze_assertion_error(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    // Filter out test assertions
    if func.starts_with("test_") {
        return None;
    }

    // Try to find the assert statement in the function
    let assert_info = if !func.is_empty() {
        find_function_node(tree, source, func)
            .and_then(|node| find_assert_statement(node, source))
    } else {
        None
    };

    let invariant_suffix = assert_info
        .as_ref()
        .map(|(_, text)| format!(" Invariant: `{}`.", text))
        .unwrap_or_default();

    let msg_tail = if !error.message.is_empty() {
        format!(" Message: {}.", error.message)
    } else {
        String::new()
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "AssertionError".to_string(),
        message: format!(
            "Production AssertionError in '{}()' -- the input or intermediate \
             state violated an invariant.{}{}",
            if func.is_empty() { "unknown" } else { func },
            invariant_suffix,
            msg_tail,
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: assert_info.as_ref().map(|(l, _)| *l).or(error.line).unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #14: NotImplementedError
// ============================================================================

/// Analyze NotImplementedError: function is still a stub.
///
/// This signals that the function body was left as a TODO. Hint-only.
fn analyze_not_implemented(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    // Verify the function has a `raise NotImplementedError`
    let is_stub = if !func.is_empty() {
        find_function_node(tree, source, func)
            .map(|node| has_raise_not_implemented(node, source))
            .unwrap_or(false)
    } else {
        false
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "NotImplementedError".to_string(),
        message: format!(
            "NotImplementedError in '{}()' -- {}. \
             Fill in the function body with the intended implementation.",
            if func.is_empty() { "unknown" } else { func },
            if is_stub {
                "the function is still a stub (raise NotImplementedError)"
            } else {
                "this is a TODO, not a bug"
            }
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #15: OSError
// ============================================================================

/// Analyze OSError family (FileNotFoundError, PermissionError, etc.).
///
/// For FileNotFoundError on write operations, injects `os.makedirs()` before
/// the `open()` call and adds `import os` if missing.
/// For other OSError subtypes, provides hints.
fn analyze_os_error(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("unknown");
    let msg = &error.message;

    // FileNotFoundError with a path -- check for write context
    if (error.error_type == "FileNotFoundError" || msg.contains("No such file or directory"))
        && msg.contains("No such file or directory")
    {
        // Extract the path from the error message
        let path_re = Regex::new(r"No such file or directory: '([^']+)'").ok();
        let path_literal = path_re
            .and_then(|re| re.captures(msg))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string());

        // Check if there's an open() call in write mode in the function
        let write_open_line = if !func.is_empty() && func != "unknown" {
            find_function_node(tree, source, func)
                .and_then(|node| find_write_mode_open_call(node, source))
        } else {
            None
        };

        if let (Some(open_line), Some(ref path)) = (write_open_line, &path_literal) {
            // Build fix: insert os.makedirs before the open call
            let indent = get_line_indent(source, open_line);
            let mut edits = Vec::new();

            // Edit 1: Add `import os` if not present
            if !has_import(source, "import os") {
                let insert_line = find_last_import_line(source)
                    .map(|l| l + 1)
                    .unwrap_or(1);
                edits.push(TextEdit {
                    line: insert_line,
                    column: None,
                    kind: EditKind::InsertBefore,
                    new_text: "import os".to_string(),
                });
            }

            // Edit 2: Insert os.makedirs before the open call
            edits.push(TextEdit {
                line: open_line,
                column: None,
                kind: EditKind::InsertBefore,
                new_text: format!(
                    "{}os.makedirs(os.path.dirname('{}'), exist_ok=True)",
                    indent, path
                ),
            });

            return Some(Diagnosis {
                language: "python".to_string(),
                error_code: "OSError".to_string(),
                message: format!(
                    "FileNotFoundError on write to '{}' -- parent directory does not exist. \
                     Inserting `os.makedirs()` before the open call.",
                    path
                ),
                location: error.file.as_ref().map(|f| FixLocation {
                    file: f.clone(),
                    line: error.line.unwrap_or(0),
                    column: None,
                }),
                confidence: FixConfidence::Medium,
                fix: Some(Fix {
                    description: format!(
                        "Create parent directory for '{}' before writing",
                        path
                    ),
                    edits,
                }),
            });
        }

        // No write-mode open found -- read context or unresolved function
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "OSError".to_string(),
            message: format!(
                "FileNotFoundError in '{}()': {}. Verify the path or create the file.",
                func, msg
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "OSError".to_string(),
        message: format!(
            "{} in '{}()': {}. Check file permissions, path validity, and the operation mode.",
            error.error_type, func, msg
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #16: UnicodeError
// ============================================================================

/// Analyze UnicodeError / UnicodeDecodeError / UnicodeEncodeError.
///
/// Fix: add `encoding='utf-8'` to bare `open()` calls in the function.
fn analyze_unicode_error(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("");

    // Check for bare open() calls without encoding=
    let open_line = if !func.is_empty() {
        find_function_node(tree, source, func)
            .and_then(|node| find_bare_open_call(node, source))
    } else {
        find_bare_open_call(tree.root_node(), source)
    };

    if let Some(line) = open_line {
        let source_line = source.lines().nth(line - 1).unwrap_or("");

        // Build fix: add encoding='utf-8' to the open call
        let fix = if source_line.contains("open(") && !source_line.contains("encoding") {
            // Find the open( call and its matching close paren, then insert
            // encoding='utf-8' before the close paren.
            let new_line = insert_encoding_into_open(source_line);

            if new_line != source_line {
                Some(Fix {
                    description: "Add `encoding='utf-8'` to `open()` call".to_string(),
                    edits: vec![TextEdit {
                        line,
                        column: None,
                        kind: EditKind::ReplaceLine,
                        new_text: new_line,
                    }],
                })
            } else {
                None
            }
        } else {
            None
        };

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "UnicodeError".to_string(),
            message: format!(
                "Unicode error on `open(...)` in '{}()'. \
                 Adding `encoding='utf-8'` to the open call.",
                func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line,
                column: None,
            }),
            confidence: if fix.is_some() {
                FixConfidence::High
            } else {
                FixConfidence::Low
            },
            fix,
        });
    }

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "UnicodeError".to_string(),
        message: format!(
            "Unicode codec error in '{}()': {}. \
             Ensure the source bytes are decoded with the correct encoding, \
             or use `errors='replace'`.",
            if func.is_empty() { "unknown" } else { func },
            error.message
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

/// Insert `encoding='utf-8'` into an `open(...)` call on a source line.
///
/// Finds the `open(` substring, then walks forward counting parens to find
/// the matching close paren (handling nested parens and string literals).
/// Inserts `, encoding='utf-8'` just before that close paren.
fn insert_encoding_into_open(line: &str) -> String {
    // Find "open(" in the line
    let open_start = match line.find("open(") {
        Some(pos) => pos,
        None => return line.to_string(),
    };
    let args_start = open_start + 5; // position right after "open("

    // Walk forward to find the matching close paren
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 1i32;
    let mut i = args_start;
    let mut in_single = false;
    let mut in_double = false;

    while i < chars.len() && depth > 0 {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        } else if !in_single && !in_double {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                depth -= 1;
                if depth == 0 {
                    // Found the matching close paren at position i
                    let before: String = chars[..i].iter().collect();
                    let after: String = chars[i..].iter().collect();
                    let trimmed = before.trim_end();
                    if trimmed.ends_with('(') {
                        // open() with no args -- should not happen but handle it
                        return format!("{}encoding='utf-8'{}", before, after);
                    } else {
                        return format!("{}, encoding='utf-8'{}", before, after);
                    }
                }
            }
        }
        i += 1;
    }

    // Could not find matching paren -- return unchanged
    line.to_string()
}

// ============================================================================
// Analyzer #17: SyntaxError
// ============================================================================

/// Analyze SyntaxError: detect missing colons, unclosed parens, etc.
///
/// Fix: insert missing `:` on compound statement headers.
fn analyze_syntax_error(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;

    // Sub-pattern: global-after-use ("name 'X' is used prior to global declaration")
    if let Some(caps) = Regex::new(r"name '(\w+)' is used prior to global declaration")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let var_name = caps.get(1).unwrap().as_str();
        let func = error.function_name.as_deref().unwrap_or("");

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "SyntaxError".to_string(),
            message: format!(
                "`global {}` must precede the first use of `{}` inside '{}()'. \
                 Move the `global` declaration to the top of the function body.",
                var_name, var_name, func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Medium,
            fix: None, // Reordering is complex; hint-only
        });
    }

    // Sub-pattern: return outside function
    if msg.contains("'return' outside function") {
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "SyntaxError".to_string(),
            message: "`return` statement appears outside any function. \
                      Wrap the statement in a function or remove it."
                .to_string(),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    // Sub-pattern: missing colon
    if msg.contains("expected ':'") || msg.contains("invalid syntax") {
        // Try to fix by adding colons to compound statement headers
        let header_re = Regex::new(
            r"^(\s*)(if|elif|else|while|for|def|async\s+def|class|try|except|finally|with|async\s+with)\b(.*)$"
        ).ok()?;

        let mut edits = Vec::new();
        for (idx, line) in source.lines().enumerate() {
            if let Some(_caps) = header_re.captures(line) {
                let trimmed = line.trim_end();
                // Skip lines that already end with ':'
                if !trimmed.ends_with(':') && !trimmed.is_empty() {
                    // Check bracket balance
                    let balanced = brackets_balanced(trimmed);
                    if balanced {
                        edits.push(TextEdit {
                            line: idx + 1,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: format!("{}:", trimmed),
                        });
                    }
                }
            }
        }

        let fix = if !edits.is_empty() {
            Some(Fix {
                description: "Add missing `:` on compound statement headers".to_string(),
                edits,
            })
        } else {
            None
        };

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "SyntaxError".to_string(),
            message: if msg.contains("expected ':'") {
                "Compound statement header is missing a trailing `:`. \
                 Add `:` at the end of the header line."
                    .to_string()
            } else {
                format!("SyntaxError: {}. Check for missing colons, unclosed brackets, or syntax issues.", msg)
            },
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: if fix.is_some() {
                FixConfidence::Medium
            } else {
                FixConfidence::Low
            },
            fix,
        });
    }

    None
}

/// Check if brackets/parens/braces are balanced in a string.
fn brackets_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
        } else if ch == '"' && !in_single {
            in_double = !in_double;
        } else if !in_single && !in_double {
            if ch == '(' || ch == '[' || ch == '{' {
                depth += 1;
            } else if ch == ')' || ch == ']' || ch == '}' {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
        }
        i += 1;
    }

    depth == 0
}

// ============================================================================
// Analyzer #18: IndentationError
// ============================================================================

/// Analyze IndentationError and TabError.
///
/// Fix: normalize mixed tabs/spaces, fix off-by-one indentation.
fn analyze_indentation_error(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;

    // Sub-pattern: tabs/spaces mixture
    if msg.contains("inconsistent use of tabs") || error.error_type == "TabError" {
        // Fix: replace all tabs with 4 spaces
        let mut edits = Vec::new();
        for (idx, line) in source.lines().enumerate() {
            if line.contains('\t') {
                let new_line = line.replace('\t', "    ");
                edits.push(TextEdit {
                    line: idx + 1,
                    column: None,
                    kind: EditKind::ReplaceLine,
                    new_text: new_line,
                });
            }
        }

        let fix = if !edits.is_empty() {
            Some(Fix {
                description: "Normalize mixed indentation: replace tabs with 4 spaces".to_string(),
                edits,
            })
        } else {
            None
        };

        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "IndentationError".to_string(),
            message: "Inconsistent use of tabs and spaces in indentation. \
                      Normalizing mixed indentation to spaces."
                .to_string(),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: if fix.is_some() {
                FixConfidence::High
            } else {
                FixConfidence::Medium
            },
            fix,
        });
    }

    // Sub-pattern: unindent does not match
    if msg.contains("unindent does not match") {
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "IndentationError".to_string(),
            message: "Indentation level does not match any outer level. \
                      Likely an off-by-one indent -- re-align the function body."
                .to_string(),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Medium,
            fix: None,
        });
    }

    // Sub-pattern: expected indented block
    if msg.contains("expected an indented block") {
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "IndentationError".to_string(),
            message: "Expected an indented block after a compound statement header. \
                      The function or block body is empty -- add a placeholder `pass` \
                      or fill in the body."
                .to_string(),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Medium,
            fix: None,
        });
    }

    // Sub-pattern: unexpected indent
    if msg.contains("unexpected indent") {
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "IndentationError".to_string(),
            message: "Unexpected indent. A line is indented further than its \
                      enclosing block expects -- check the outer block structure."
                .to_string(),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    None
}

// ============================================================================
// Analyzer #19: CircularImportError
// ============================================================================

/// Analyze circular import: `partially initialized module`.
///
/// Fix: when the offending top-level import and a function that uses the
/// imported name can both be found, produce a fix that deletes the
/// top-level import and inserts it inside the function body.
fn analyze_circular_import(
    error: &ParsedError,
    source: &str,
    tree: &Tree,
) -> Option<Diagnosis> {
    if !error.message.contains("partially initialized module") {
        return None;
    }

    let caps = Regex::new(r"cannot import name '(\w+)' from partially initialized module '([\w.]+)'")
        .ok()
        .and_then(|re| re.captures(&error.message));

    let (name, module) = if let Some(c) = caps {
        (
            c.get(1).unwrap().as_str().to_string(),
            c.get(2).unwrap().as_str().to_string(),
        )
    } else {
        ("?".to_string(), "?".to_string())
    };

    // Try to produce a fix: find the top-level import and a function that uses the name
    let fix = if name != "?" && module != "?" {
        // Find the top-level import line
        let import_line = find_top_level_import_line(source, tree, &module, &name);

        // Find a function that references the imported name
        let using_func = find_function_using_name(tree, source, &name);

        match (import_line, using_func) {
            (Some(imp_line), Some((_func_name, body_start))) => {
                // Get the indent for the function body
                let indent = get_line_indent(source, body_start);

                let edits = vec![
                    // Delete the top-level import
                    TextEdit {
                        line: imp_line,
                        column: None,
                        kind: EditKind::DeleteLine,
                        new_text: String::new(),
                    },
                    // Insert the import inside the function body
                    TextEdit {
                        line: body_start,
                        column: None,
                        kind: EditKind::InsertBefore,
                        new_text: format!("{}from {} import {}", indent, module, name),
                    },
                ];

                Some(Fix {
                    description: format!(
                        "Move `from {} import {}` inside the function that uses it",
                        module, name
                    ),
                    edits,
                })
            }
            _ => None,
        }
    } else {
        None
    };

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "ImportError".to_string(),
        message: format!(
            "Circular import detected: cannot import '{}' from partially \
             initialized module '{}'. Break the cycle by moving the import \
             inside the function that needs it, or by restructuring the \
             module dependencies.",
            name, module
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Medium,
        fix,
    })
}

// ============================================================================
// Analyzer #20: TypeError (general -- other patterns)
// ============================================================================

/// Analyze remaining TypeError patterns not caught by callable/serialization.
///
/// Handles:
/// - unexpected keyword argument
/// - missing required argument
/// - not subscriptable
/// - not iterable
/// - unhashable type
fn analyze_type_error_general(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let msg = &error.message;
    let func = error.function_name.as_deref().unwrap_or("unknown");

    // unexpected keyword argument
    if let Some(caps) = Regex::new(r"(\w+)\(\) got an unexpected keyword argument '(\w+)'")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let called_func = caps.get(1).unwrap().as_str();
        let bad_kwarg = caps.get(2).unwrap().as_str();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "'{}()' does not accept keyword argument '{}'. \
                 Check spelling or remove the argument.",
                called_func, bad_kwarg
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Medium,
            fix: None,
        });
    }

    // missing required argument
    if let Some(caps) =
        Regex::new(r"(\w+)\(\) missing (\d+) required positional arguments?: (.+)")
            .ok()
            .and_then(|re| re.captures(msg))
    {
        let called_func = caps.get(1).unwrap().as_str();
        let missing_args = caps.get(3).unwrap().as_str().trim();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "'{}()' requires argument(s): {}. Add the missing arguments.",
                called_func, missing_args
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Medium,
            fix: None,
        });
    }

    // not subscriptable
    if let Some(caps) = Regex::new(r"'(\w+)' object is not subscriptable")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let type_name = caps.get(1).unwrap().as_str();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "'{}' is not subscriptable in '{}()'. The value is likely None or \
                 a scalar -- guard with a None-check, or ensure the call returns \
                 a dict/list.",
                type_name, func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    // not iterable
    if let Some(caps) = Regex::new(r"'(\w+)' object is not iterable")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let type_name = caps.get(1).unwrap().as_str();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "'{}' is not iterable in '{}()'. Wrap the scalar in a list, \
                 or verify the source is a collection before iterating.",
                type_name, func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    // unhashable type
    if let Some(caps) = Regex::new(r"unhashable type: '(\w+)'")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let type_name = caps.get(1).unwrap().as_str();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "Unhashable type '{}' used as a dict key or set member in '{}()'. \
                 Use a tuple instead of a list, or a frozenset instead of a set.",
                type_name, func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    // argument of type not iterable
    if let Some(caps) = Regex::new(r"argument of type '(\w+)' is not iterable")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        let type_name = caps.get(1).unwrap().as_str();
        return Some(Diagnosis {
            language: "python".to_string(),
            error_code: "TypeError".to_string(),
            message: format!(
                "`in` operator applied to a non-iterable '{}' in '{}()'. \
                 Ensure the right-hand operand of `in` is a collection.",
                type_name, func
            ),
            location: error.file.as_ref().map(|f| FixLocation {
                file: f.clone(),
                line: error.line.unwrap_or(0),
                column: None,
            }),
            confidence: FixConfidence::Low,
            fix: None,
        });
    }

    // Generic TypeError fallback
    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "TypeError".to_string(),
        message: format!(
            "TypeError in '{}()': {}. Check argument types and function signatures.",
            func, msg
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #21: RuntimeError
// ============================================================================

/// Analyze RuntimeError: generic runtime error handling.
fn analyze_runtime_error(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("unknown");

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: "RuntimeError".to_string(),
        message: format!(
            "RuntimeError in '{}()': {}. Review the runtime conditions and \
             add appropriate error handling.",
            func, error.message
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer #22: GenericException (floor -- always returns Some)
// ============================================================================

/// Analyze any exception type as a catch-all floor.
///
/// This analyzer ALWAYS returns Some(Diagnosis). It is the last analyzer
/// in the dispatch chain, ensuring that every error gets at least a
/// structured hint for the heal loop to consume.
fn analyze_generic_exception(
    error: &ParsedError,
    _source: &str,
    _tree: &Tree,
) -> Option<Diagnosis> {
    let func = error.function_name.as_deref().unwrap_or("module");

    // Pull the offending line from the traceback if available
    let offending = error
        .offending_line
        .as_deref()
        .map(|l| format!(" Offending line: `{}`.", l))
        .unwrap_or_default();

    Some(Diagnosis {
        language: "python".to_string(),
        error_code: error.error_type.clone(),
        message: format!(
            "{} in '{}()': {}.{} No deterministic fix available -- \
             re-prompt the model with the exception class and message as context.",
            error.error_type,
            func,
            error.message,
            offending,
        ),
        location: error.file.as_ref().map(|f| FixLocation {
            file: f.clone(),
            line: error.line.unwrap_or(0),
            column: None,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    

    /// Helper to parse Python source and create a tree for testing.
    fn parse_python(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    // ---- Validation gate: all 22 analyzers registered ----

    #[test]
    fn test_all_22_python_analyzers_registered() {
        // This test verifies that all 22 error types dispatch to a real analyzer.
        // Each tuple is (error_type, sample_message, expected_error_code_in_diagnosis).
        let test_cases: Vec<(&str, &str, &str)> = vec![
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
        let tree = parse_python(source);

        let mut handled_count = 0;
        for (error_type, message, expected_code) in &test_cases {
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

            let result = diagnose_python(&error, source, &tree, None);
            assert!(
                result.is_some(),
                "Analyzer for {} should return Some, got None (message: {})",
                error_type,
                message
            );
            let diag = result.unwrap();
            assert_eq!(
                diag.error_code, *expected_code,
                "Expected error_code '{}' for {}, got '{}'",
                expected_code, error_type, diag.error_code
            );
            handled_count += 1;
        }

        assert_eq!(
            handled_count, 22,
            "Expected 22 handled cases, got {}",
            handled_count
        );
    }

    // ---- Per-analyzer tests ----

    #[test]
    fn test_python_unbound_local_fix() {
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: Some("inc".to_string()),
            offending_line: Some("    counter += 1".to_string()),
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnboundLocalError");
        assert_eq!(diag.confidence, FixConfidence::High);
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        assert!(fix.edits[0].new_text.contains("global counter"));
        assert_eq!(fix.edits[0].line, 3); // InsertBefore first stmt in function body (line 3)
        assert_eq!(fix.edits[0].kind, EditKind::InsertBefore);
    }

    #[test]
    fn test_python_name_error_stdlib_fix() {
        let source = "def f():\n    data = json.loads('{}')\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "NameError".to_string(),
            message: "name 'json' is not defined".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "NameError: name 'json' is not defined".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "NameError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits[0].new_text, "import json");
    }

    #[test]
    fn test_python_key_error_fix() {
        let source = "def lookup(name):\n    d = {'a': 1, 'b': 2}\n    return d[name]\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "KeyError".to_string(),
            message: "'name'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "KeyError: 'name'".to_string(),
            function_name: Some("lookup".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "KeyError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        assert!(fix.edits[0].new_text.contains(".get(name)"));
    }

    #[test]
    fn test_python_zero_division() {
        let source = "def divide(a, b):\n    return a / b\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "ZeroDivisionError".to_string(),
            message: "division by zero".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "ZeroDivisionError: division by zero".to_string(),
            function_name: Some("divide".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "ZeroDivisionError");
        assert!(diag.message.contains("zero-check"));
    }

    #[test]
    fn test_python_type_error_callable() {
        let source = "class Foo:\n    @property\n    def bar(self):\n        return 1\n\nfoo = Foo()\nresult = foo.bar()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "'int' object is not callable".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(7),
            column: None,
            language: "python".to_string(),
            raw_text: "TypeError: 'int' object is not callable".to_string(),
            function_name: None,
            offending_line: Some("result = foo.bar()".to_string()),
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "TypeError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        assert!(fix.edits[0].new_text.contains("foo.bar") && !fix.edits[0].new_text.contains("foo.bar()"));
    }

    #[test]
    fn test_python_indentation_error_tabs() {
        let source = "def f():\n\tx = 1\n    y = 2\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TabError".to_string(),
            message: "inconsistent use of tabs and spaces in indentation".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "TabError: inconsistent use of tabs and spaces in indentation"
                .to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "IndentationError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        // Verify tabs were replaced with spaces
        assert!(fix.edits.iter().any(|e| !e.new_text.contains('\t')));
    }

    #[test]
    fn test_python_syntax_error_missing_colon() {
        let source = "def f()\n    pass\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "SyntaxError".to_string(),
            message: "expected ':'".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "SyntaxError: expected ':'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "SyntaxError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        assert!(fix.edits[0].new_text.ends_with(':'));
    }

    #[test]
    fn test_python_import_error() {
        let source = "from os import something_wrong\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "ImportError".to_string(),
            message: "cannot import name 'something_wrong' from 'os'".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ImportError: cannot import name 'something_wrong' from 'os'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "ImportError");
    }

    #[test]
    fn test_python_recursion_error() {
        let source = "def fib(n):\n    return fib(n-1) + fib(n-2)\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "RecursionError".to_string(),
            message: "maximum recursion depth exceeded".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "RecursionError: maximum recursion depth exceeded".to_string(),
            function_name: Some("fib".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "RecursionError");
        // fib has no return before recursive call -> should detect missing base case
        assert!(diag.message.contains("base") || diag.message.contains("recursion"));
    }

    #[test]
    fn test_python_stop_iteration() {
        let source = "def first(items):\n    it = iter(items)\n    return next(it)\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "StopIteration".to_string(),
            message: "".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "StopIteration".to_string(),
            function_name: Some("first".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "StopIteration");
        assert!(diag.message.contains("next(it, default)"));
    }

    #[test]
    fn test_python_circular_import() {
        let source = "from mymodule import MyClass\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "ImportError".to_string(),
            message: "cannot import name 'MyClass' from partially initialized module 'mymodule'"
                .to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ImportError: cannot import name 'MyClass' from partially initialized module 'mymodule'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.message.contains("Circular import"));
    }

    #[test]
    fn test_python_assertion_error() {
        let source = "def validate(x):\n    assert x > 0\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "AssertionError".to_string(),
            message: "".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "AssertionError".to_string(),
            function_name: Some("validate".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "AssertionError");
        assert!(diag.message.contains("invariant"));
    }

    #[test]
    fn test_python_not_implemented() {
        let source = "def process():\n    raise NotImplementedError\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "NotImplementedError".to_string(),
            message: "".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "NotImplementedError".to_string(),
            function_name: Some("process".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "NotImplementedError");
        assert!(diag.message.contains("stub"));
    }

    #[test]
    fn test_python_generic_exception() {
        let source = "def f():\n    pass\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "SomeCustomException".to_string(),
            message: "something broke".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "SomeCustomException: something broke".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "SomeCustomException");
        assert!(diag.message.contains("No deterministic fix"));
    }

    #[test]
    fn test_python_os_error() {
        let source = "def save():\n    with open('/tmp/data/out.txt', 'w') as f:\n        f.write('hi')\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "FileNotFoundError".to_string(),
            message: "No such file or directory: '/tmp/data/out.txt'".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "FileNotFoundError: No such file or directory: '/tmp/data/out.txt'"
                .to_string(),
            function_name: Some("save".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "OSError");
    }

    #[test]
    fn test_python_unicode_error() {
        let source = "def read_file():\n    with open('data.txt') as f:\n        return f.read()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnicodeDecodeError".to_string(),
            message: "'utf-8' codec can't decode byte 0xff".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff".to_string(),
            function_name: Some("read_file".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnicodeError");
        assert!(diag.message.contains("encoding"));
    }

    #[test]
    fn test_python_value_error_patterns() {
        let source = "def f():\n    pass\n";
        let tree = parse_python(source);

        // Invalid literal
        let error = ParsedError {
            error_type: "ValueError".to_string(),
            message: "invalid literal for int() with base 10: 'abc'".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ValueError: invalid literal for int() with base 10".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };
        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.message.contains("int()"));

        // Not enough values to unpack
        let error2 = ParsedError {
            error_type: "ValueError".to_string(),
            message: "not enough values to unpack (expected 3, got 2)".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ValueError: not enough values to unpack".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };
        let diag2 = diagnose_python(&error2, source, &tree, None).unwrap();
        assert!(diag2.message.contains("unpack"));
    }

    #[test]
    fn test_python_index_error() {
        let source = "def f():\n    items = []\n    return items[0]\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "IndexError".to_string(),
            message: "list index out of range".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "IndexError: list index out of range".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "IndexError");
        assert!(diag.message.contains("list"));
    }

    #[test]
    fn test_python_attribute_error() {
        let source = "def f():\n    'hello'.frobnicate()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "AttributeError".to_string(),
            message: "'str' object has no attribute 'frobnicate'".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "AttributeError: 'str' object has no attribute 'frobnicate'".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "AttributeError");
        assert!(diag.message.contains("frobnicate"));
    }

    #[test]
    fn test_python_runtime_error() {
        let source = "def f():\n    pass\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "RuntimeError".to_string(),
            message: "something went wrong at runtime".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "RuntimeError: something went wrong at runtime".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "RuntimeError");
        assert!(diag.message.contains("runtime"));
    }

    #[test]
    fn test_python_type_error_general_unexpected_keyword() {
        let source = "def f():\n    dict(foo=1)\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "bar() got an unexpected keyword argument 'baz'".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "TypeError: bar() got an unexpected keyword argument 'baz'".to_string(),
            function_name: Some("f".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "TypeError");
        assert!(diag.message.contains("baz"));
    }

    #[test]
    fn test_python_serialization_fix() {
        let source = "from dataclasses import dataclass\n\n@dataclass\nclass Item:\n    name: str\n\ndef get():\n    item = Item('x')\n    return jsonify(item)\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Object of type Item is not JSON serializable".to_string(),
            file: None,
            line: Some(9),
            column: None,
            language: "python".to_string(),
            raw_text: "TypeError: Object of type Item is not JSON serializable".to_string(),
            function_name: Some("get".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "TypeError");
        assert!(diag.fix.is_some());
        let fix = diag.fix.unwrap();
        // Should wrap with asdict
        assert!(fix.edits.iter().any(|e| e.new_text.contains("asdict")));
    }

    // ---- Bug regression tests ----

    #[test]
    fn test_python_unbound_local_no_function_name_still_produces_fix() {
        // Bug: When function_name is None in the ParsedError, analyze_unbound_local
        // returns a Diagnosis with fix: None and Medium confidence. But if we have
        // the error line number and the source, we can find the enclosing function
        // using tree-sitter and still produce a fix.
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: None, // No function name provided
            offending_line: Some("    counter += 1".to_string()),
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnboundLocalError");

        // The fix should be present even without function_name
        assert!(
            diag.fix.is_some(),
            "Diagnosis should contain a fix even when function_name is None; \
             the analyzer should find the enclosing function from the line number"
        );
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits.len(), 1, "Fix should have exactly one edit");
        assert!(
            fix.edits[0].new_text.contains("global counter"),
            "Fix edit should inject 'global counter', got: {:?}",
            fix.edits[0].new_text
        );
        assert_eq!(
            fix.edits[0].kind,
            EditKind::InsertBefore,
            "Fix should use InsertBefore to inject before the first body statement"
        );
    }

    #[test]
    fn test_python_unbound_local_no_function_name_nested_function() {
        // Even with deeper nesting, should find the correct enclosing function.
        let source = "total = 0\ndef outer():\n    def inner():\n        total += 1\n    inner()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'total'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(4), // "total += 1" is on line 4
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'total'".to_string(),
            function_name: None,
            offending_line: Some("        total += 1".to_string()),
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnboundLocalError");
        assert!(
            diag.fix.is_some(),
            "Should produce a fix even for nested functions without function_name"
        );
        let fix = diag.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("global total"),
            "Fix should inject 'global total', got: {:?}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_python_unbound_local_no_function_name_confidence() {
        // When the variable IS at module scope AND assigned in the function,
        // confidence should be High even when function_name was not provided.
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(
            diag.confidence,
            FixConfidence::High,
            "Confidence should be High when variable is at module scope and assigned in function"
        );
    }

    #[test]
    fn test_python_unbound_local_no_line_no_function_name_produces_fix() {
        // This is the exact CLI scenario: single-line error with no traceback,
        // so ParsedError has line: None and function_name: None.
        // The analyzer should scan the tree for a function assigning to the variable.
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(std::path::PathBuf::from("test_scope.py")),
            line: None,       // No line number (single-line error)
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: None, // No function name (no traceback)
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnboundLocalError");

        // The fix MUST be present -- this was the original bug
        assert!(
            diag.fix.is_some(),
            "Diagnosis must contain a fix even when both line and function_name are None; \
             the analyzer should find the function by scanning for variable assignments"
        );
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits.len(), 1, "Fix should have exactly one edit");
        assert!(
            fix.edits[0].new_text.contains("global counter"),
            "Fix edit should inject 'global counter', got: {:?}",
            fix.edits[0].new_text
        );
        assert_eq!(
            fix.edits[0].kind,
            EditKind::InsertBefore,
            "Fix should use InsertBefore to inject before the first body statement"
        );
        assert_eq!(
            fix.edits[0].line, 3,
            "Fix should target line 3 (first statement in inc() body)"
        );
    }

    #[test]
    fn test_python_unbound_local_no_line_no_function_multiple_functions() {
        // When multiple functions exist, should find the one that assigns to the variable.
        let source = "counter = 0\ndef get():\n    return counter\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: None,
            line: None,
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(
            diag.fix.is_some(),
            "Should produce a fix when scanning finds the assigning function"
        );
        let fix = diag.fix.unwrap();
        assert!(
            fix.description.contains("inc"),
            "Fix should target 'inc()' (the function that assigns counter), got: {:?}",
            fix.description
        );
        assert!(fix.edits[0].new_text.contains("global counter"));
    }

    #[test]
    fn test_python_unbound_local_fallback_body_start() {
        // Test that the fallback computes body_start from the function definition
        // line when find_function_body_start() returns None. This exercises the
        // fallback path added to analyze_unbound_local.
        //
        // We test this indirectly: even if find_function_body_start can't iterate
        // named children, the fix must still be produced using the function def
        // line + 1 as the insertion point.
        //
        // Use a one-liner function body which some tree-sitter versions may parse
        // with the body statement on the same row as `def`, causing the named-
        // children iteration to produce a line that still works, but also validate
        // the general contract: fix is NEVER None when func_name is resolved.
        let source = "counter = 0\ndef inc(): counter += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'counter'".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: None,
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'counter'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnboundLocalError");

        // The fix MUST always be present when a function is resolved
        assert!(
            diag.fix.is_some(),
            "Fix must NEVER be None when the analyzer resolves a function name. \
             The fallback should compute body_start from the function def line."
        );
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        assert!(
            fix.edits[0].new_text.contains("global counter"),
            "Fix should inject 'global counter', got: {:?}",
            fix.edits[0].new_text
        );
        assert_eq!(fix.edits[0].kind, EditKind::InsertBefore);
    }

    #[test]
    fn test_python_unbound_local_fix_always_present_when_func_resolved() {
        // Regression guard: the fix field must NEVER be None when the analyzer
        // successfully resolves a function name. This test uses a decorated
        // function to exercise a different code path through find_function_node.
        let source = "total = 0\ndef process():\n    \"\"\"Process data.\"\"\"\n    total += 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnboundLocalError".to_string(),
            message: "cannot access local variable 'total'".to_string(),
            file: None,
            line: None,
            column: None,
            language: "python".to_string(),
            raw_text: "UnboundLocalError: cannot access local variable 'total'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(
            diag.fix.is_some(),
            "Fix must be present when function is resolved via find_function_assigning_var"
        );
        let fix = diag.fix.unwrap();
        assert!(fix.edits[0].new_text.contains("global total"));
        assert_eq!(fix.edits[0].kind, EditKind::InsertBefore);
    }

    #[test]
    fn test_find_function_body_start_returns_some_for_valid_function() {
        // Verify find_function_body_start always returns Some for a found function
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);

        let result = find_function_body_start(source, &tree, "inc");
        assert!(
            result.is_some(),
            "find_function_body_start must return Some when the function exists"
        );
        assert_eq!(result.unwrap(), 3, "Body starts at line 3 (counter += 1)");
    }

    #[test]
    fn test_find_function_body_start_fallback_for_oneliner() {
        // One-liner functions: `def inc(): counter += 1`
        // tree-sitter puts the body statement on the same row as def
        let source = "counter = 0\ndef inc(): counter += 1\n";
        let tree = parse_python(source);

        let result = find_function_body_start(source, &tree, "inc");
        assert!(
            result.is_some(),
            "find_function_body_start must return Some for one-liner functions"
        );
        // For a one-liner, the body statement is on the same line as def (line 2),
        // so the primary path should find it at line 2. If it doesn't, the
        // fallback computes def_line + 1 = also line 3.
        let line = result.unwrap();
        assert!(
            line == 2 || line == 3,
            "Body start should be line 2 (same as def) or line 3 (fallback), got: {}",
            line
        );
    }

    #[test]
    fn test_find_function_body_start_not_found_returns_none() {
        // If the function doesn't exist at all, should return None
        let source = "counter = 0\ndef inc():\n    counter += 1\n";
        let tree = parse_python(source);

        let result = find_function_body_start(source, &tree, "nonexistent_func");
        assert!(
            result.is_none(),
            "find_function_body_start should return None when function not found"
        );
    }

    #[test]
    fn test_find_function_assigning_var_helper() {
        let source = "x = 0\ndef foo():\n    x += 1\ndef bar():\n    print(x)\n";
        let tree = parse_python(source);
        let result = find_function_assigning_var(&tree, source, "x");
        assert_eq!(
            result,
            Some("foo".to_string()),
            "Should find 'foo' as the function that assigns to 'x'"
        );

        // Variable not assigned in any function
        let result_none = find_function_assigning_var(&tree, source, "y");
        assert_eq!(
            result_none, None,
            "Should return None when no function assigns to 'y'"
        );
    }

    // ================================================================
    // Fix closure tests -- UnicodeError
    // ================================================================

    #[test]
    fn test_unicode_error_fix_bare_open_single_arg() {
        // open('file.txt') -> open('file.txt', encoding='utf-8')
        let source = "def read_data():\n    data = open('file.txt').read()\n    return data\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnicodeDecodeError".to_string(),
            message: "'ascii' codec can't decode byte 0xc3".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "UnicodeDecodeError: 'ascii' codec can't decode byte 0xc3".to_string(),
            function_name: Some("read_data".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnicodeError");
        assert!(
            diag.fix.is_some(),
            "UnicodeError fix must be present when bare open() is found"
        );
        let fix = diag.fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        assert!(
            fix.edits[0].new_text.contains("encoding='utf-8'"),
            "Fix should add encoding='utf-8', got: {:?}",
            fix.edits[0].new_text
        );
        assert_eq!(fix.edits[0].kind, EditKind::ReplaceLine);
        assert_eq!(fix.edits[0].line, 2);
        assert_eq!(diag.confidence, FixConfidence::High);
    }

    #[test]
    fn test_unicode_error_fix_open_with_mode() {
        // open('file.txt', 'r') -> open('file.txt', 'r', encoding='utf-8')
        let source = "def load():\n    f = open('data.csv', 'r')\n    return f.read()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnicodeDecodeError".to_string(),
            message: "'utf-8' codec can't decode byte 0xff".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "UnicodeDecodeError: 'utf-8' codec can't decode byte 0xff".to_string(),
            function_name: Some("load".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnicodeError");
        assert!(
            diag.fix.is_some(),
            "UnicodeError fix must be present for open with mode arg"
        );
        let fix = diag.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("encoding='utf-8'"),
            "Fix should add encoding='utf-8', got: {:?}",
            fix.edits[0].new_text
        );
        // Should still have the mode arg
        assert!(
            fix.edits[0].new_text.contains("'r'"),
            "Fix should preserve the mode arg, got: {:?}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_unicode_error_no_fix_when_encoding_present() {
        // Already has encoding= -> no fix needed, should still get diagnosis
        let source = "def read():\n    f = open('data.txt', encoding='latin-1')\n    return f.read()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "UnicodeDecodeError".to_string(),
            message: "'latin-1' codec can't decode byte 0xff".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "UnicodeDecodeError: 'latin-1' codec can't decode byte 0xff".to_string(),
            function_name: Some("read".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "UnicodeError");
        // No bare open found, so no fix -- just the fallback hint
        assert!(diag.fix.is_none(), "Should not produce a fix when encoding= is already present");
    }

    // ================================================================
    // Fix closure tests -- OSError
    // ================================================================

    #[test]
    fn test_os_error_fix_mkdir_before_write() {
        // FileNotFoundError on write -> insert os.makedirs before open
        let source = "def save(data):\n    f = open('/tmp/subdir/out.txt', 'w')\n    f.write(data)\n    f.close()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "FileNotFoundError".to_string(),
            message: "No such file or directory: '/tmp/subdir/out.txt'".to_string(),
            file: Some(std::path::PathBuf::from("writer.py")),
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "FileNotFoundError: [Errno 2] No such file or directory: '/tmp/subdir/out.txt'".to_string(),
            function_name: Some("save".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "OSError");
        assert!(
            diag.fix.is_some(),
            "OSError fix must be present when write-mode open() is found with missing dir"
        );
        let fix = diag.fix.unwrap();
        // Should have at least 1 edit: the makedirs insertion
        assert!(
            !fix.edits.is_empty(),
            "Fix should have at least one edit"
        );
        // Check that one edit inserts os.makedirs
        assert!(
            fix.edits.iter().any(|e| e.new_text.contains("os.makedirs")),
            "Fix should contain os.makedirs, got edits: {:?}",
            fix.edits
        );
        // Check that one edit inserts before the open line
        assert!(
            fix.edits.iter().any(|e| e.kind == EditKind::InsertBefore),
            "Fix should use InsertBefore for the makedirs line"
        );
        assert_eq!(diag.confidence, FixConfidence::Medium);
    }

    #[test]
    fn test_os_error_fix_adds_import_os() {
        // When source has no `import os`, the fix should add it
        let source = "def save(data):\n    f = open('/tmp/subdir/out.txt', 'w')\n    f.write(data)\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "FileNotFoundError".to_string(),
            message: "No such file or directory: '/tmp/subdir/out.txt'".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "FileNotFoundError: No such file or directory: '/tmp/subdir/out.txt'".to_string(),
            function_name: Some("save".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.fix.is_some(), "Fix must be present");
        let fix = diag.fix.unwrap();
        // Should add import os
        assert!(
            fix.edits.iter().any(|e| e.new_text.contains("import os")),
            "Fix should add 'import os' when not present, got edits: {:?}",
            fix.edits
        );
    }

    #[test]
    fn test_os_error_no_fix_for_read_context() {
        // FileNotFoundError on read (no write mode) -> hint only, no fix
        let source = "def load():\n    f = open('/tmp/missing.txt', 'r')\n    return f.read()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "FileNotFoundError".to_string(),
            message: "No such file or directory: '/tmp/missing.txt'".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "FileNotFoundError: No such file or directory: '/tmp/missing.txt'".to_string(),
            function_name: Some("load".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "OSError");
        // Read context: cannot invent a missing file, so no fix
        assert!(diag.fix.is_none(), "Should not produce a fix for read-context FileNotFoundError");
    }

    // ================================================================
    // Fix closure tests -- IndentationError
    // ================================================================

    #[test]
    fn test_indentation_error_fix_single_line_tabs() {
        // Error on a specific line with tab -> replace that line's tabs with spaces
        let source = "def f():\n\tx = 1\n\ty = 2\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "IndentationError".to_string(),
            message: "inconsistent use of tabs and spaces in indentation".to_string(),
            file: Some(std::path::PathBuf::from("module.py")),
            line: Some(2),
            column: None,
            language: "python".to_string(),
            raw_text: "IndentationError: inconsistent use of tabs and spaces in indentation".to_string(),
            function_name: None,
            offending_line: Some("\tx = 1".to_string()),
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert_eq!(diag.error_code, "IndentationError");
        assert!(
            diag.fix.is_some(),
            "IndentationError fix must be present when tabs found"
        );
        let fix = diag.fix.unwrap();
        // All edits should replace tabs with spaces
        for edit in &fix.edits {
            assert!(
                !edit.new_text.contains('\t'),
                "Fixed line should not contain tabs: {:?}",
                edit.new_text
            );
            assert!(
                edit.new_text.contains("    "),
                "Fixed line should have 4-space indent: {:?}",
                edit.new_text
            );
        }
        assert_eq!(diag.confidence, FixConfidence::High);
    }

    #[test]
    fn test_indentation_error_fix_mixed_tabs_spaces() {
        // Source with a mix of tabs and spaces across lines
        let source = "def process():\n    x = 1\n\ty = 2\n    z = 3\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TabError".to_string(),
            message: "inconsistent use of tabs and spaces in indentation".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "python".to_string(),
            raw_text: "TabError: inconsistent use of tabs and spaces in indentation".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.fix.is_some(), "Fix should be present for mixed tabs/spaces");
        let fix = diag.fix.unwrap();
        // Only the tab-containing line should be in the edits
        assert!(
            fix.edits.iter().any(|e| e.line == 3),
            "Should have an edit for line 3 (the tab line)"
        );
        // Line 2 uses spaces already so should not be in edits
        // Line 3 has a tab so it should be edited
        for edit in &fix.edits {
            assert!(
                !edit.new_text.contains('\t'),
                "All edits should replace tabs with spaces"
            );
        }
    }

    // ================================================================
    // Fix closure tests -- CircularImportError
    // ================================================================

    #[test]
    fn test_circular_import_fix_moves_import_into_function() {
        // Top-level import causes circular dependency. Fix: move inside function.
        let source = "from mymodule import MyClass\n\ndef process():\n    obj = MyClass()\n    return obj.run()\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "ImportError".to_string(),
            message: "cannot import name 'MyClass' from partially initialized module 'mymodule'".to_string(),
            file: Some(std::path::PathBuf::from("handler.py")),
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ImportError: cannot import name 'MyClass' from partially initialized module 'mymodule'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.message.contains("Circular import"));
        assert!(
            diag.fix.is_some(),
            "CircularImport fix must be present when import and using function are found"
        );
        let fix = diag.fix.unwrap();
        // Should have 2 edits: delete the top-level import + insert inside function
        assert!(
            fix.edits.len() >= 2,
            "Fix should have at least 2 edits (delete + insert), got: {}",
            fix.edits.len()
        );
        // One edit should delete the top-level import line
        assert!(
            fix.edits.iter().any(|e| e.kind == EditKind::DeleteLine && e.line == 1),
            "Fix should delete the top-level import at line 1, got edits: {:?}",
            fix.edits
        );
        // One edit should insert the import inside the function
        assert!(
            fix.edits.iter().any(|e| e.kind == EditKind::InsertBefore
                && e.new_text.contains("from mymodule import MyClass")),
            "Fix should insert 'from mymodule import MyClass' inside the function, got edits: {:?}",
            fix.edits
        );
        assert_eq!(diag.confidence, FixConfidence::Medium);
    }

    #[test]
    fn test_circular_import_no_fix_when_no_using_function() {
        // Import exists but no function uses the imported name -> hint only
        let source = "from mymodule import MyClass\n\nx = 1\n";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "ImportError".to_string(),
            message: "cannot import name 'MyClass' from partially initialized module 'mymodule'".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "python".to_string(),
            raw_text: "ImportError: cannot import name 'MyClass' from partially initialized module 'mymodule'".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.message.contains("Circular import"));
        // No function uses the name, so no auto-fix possible
        assert!(
            diag.fix.is_none(),
            "Should not produce a fix when no function uses the imported name"
        );
    }

    #[test]
    fn test_python_serialization_asdict_import_no_typo() {
        // Regression: when source has `from dataclasses import dataclass` (no asdict),
        // the fix should add `asdict` to that import line, producing:
        //   `from dataclasses import dataclass, asdict`
        // NOT the broken:
        //   `from dataclass, asdictes import dataclass, asdict`
        // which happened because str::replace matched "dataclass" inside "dataclasses".
        let source = "\
from dataclasses import dataclass

@dataclass
class Item:
    name: str

def get():
    item = Item('x')
    return jsonify(item)
";
        let tree = parse_python(source);
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Object of type Item is not JSON serializable".to_string(),
            file: Some(std::path::PathBuf::from("app.py")),
            line: Some(9),
            column: None,
            language: "python".to_string(),
            raw_text: "TypeError: Object of type Item is not JSON serializable".to_string(),
            function_name: Some("get".to_string()),
            offending_line: None,
        };

        let diag = diagnose_python(&error, source, &tree, None).unwrap();
        assert!(diag.fix.is_some(), "Should produce a fix");
        let fix = diag.fix.unwrap();

        // Find the edit that modifies the import line
        let import_edit = fix.edits.iter().find(|e| e.new_text.contains("import"));
        assert!(
            import_edit.is_some(),
            "Fix should include an edit adding asdict to the import line"
        );
        let import_line = &import_edit.unwrap().new_text;

        // The import line must be correctly spelled
        assert!(
            import_line.contains("from dataclasses import"),
            "Import line should start with 'from dataclasses import', got: {}",
            import_line
        );
        assert!(
            import_line.contains("dataclass, asdict") || import_line.contains("dataclass,asdict"),
            "Import line should include both dataclass and asdict, got: {}",
            import_line
        );
        // Must NOT contain the broken form
        assert!(
            !import_line.contains("asdictes"),
            "Import line must not contain typo 'asdictes', got: {}",
            import_line
        );
        assert!(
            !import_line.contains("dataclass, asdictes"),
            "Import line must not contain broken 'dataclass, asdictes', got: {}",
            import_line
        );
    }
}
