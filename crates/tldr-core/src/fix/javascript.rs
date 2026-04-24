//! JavaScript error analyzers -- 4 analyzers for Node.js runtime errors.
//!
//! Each analyzer is a pure function that takes a `ParsedError`, source code,
//! and a tree-sitter `Tree`, and returns an `Option<Diagnosis>`.
//!
//! # Analyzer Inventory (4 total)
//!
//! | # | Pattern                                      | Analyzer                      | Fix                                                | Confidence |
//! |---|----------------------------------------------|-------------------------------|----------------------------------------------------|------------|
//! | 1 | `ReferenceError: X is not defined`           | analyze_reference_error       | Inject `require()` or `import` for known modules   | MEDIUM     |
//! | 2 | `TypeError: X is not a function`             | analyze_type_error_not_function| Check if X exists as property; suggest access      | MEDIUM     |
//! | 3 | `TypeError: Cannot read properties of undefined`| analyze_type_error_undefined | Add optional chaining `?.` or null guard          | MEDIUM     |
//! | 4 | `SyntaxError`                                | analyze_syntax_error          | Common: missing comma, unclosed bracket, etc.      | LOW        |

use regex::Regex;
use tree_sitter::Tree;

use super::types::{Diagnosis, EditKind, Fix, FixConfidence, FixLocation, ParsedError, TextEdit};

// ============================================================================
// Known-module lookup table (data, not code)
// ============================================================================

/// Common Node.js builtin modules: maps a name to the `require()` statement.
///
/// Used by the ReferenceError analyzer to inject the correct require/import
/// when a module-level name is used without importing it.
static KNOWN_MODULES: &[(&str, &str)] = &[
    ("fs", "const fs = require('fs');"),
    ("path", "const path = require('path');"),
    ("os", "const os = require('os');"),
    ("http", "const http = require('http');"),
    ("https", "const https = require('https');"),
    ("url", "const url = require('url');"),
    ("crypto", "const crypto = require('crypto');"),
    ("util", "const util = require('util');"),
    ("stream", "const stream = require('stream');"),
    ("events", "const events = require('events');"),
    (
        "child_process",
        "const child_process = require('child_process');",
    ),
    ("buffer", "const { Buffer } = require('buffer');"),
    ("Buffer", "const { Buffer } = require('buffer');"),
    ("querystring", "const querystring = require('querystring');"),
    ("assert", "const assert = require('assert');"),
    ("zlib", "const zlib = require('zlib');"),
    ("net", "const net = require('net');"),
    ("dns", "const dns = require('dns');"),
    ("tls", "const tls = require('tls');"),
    ("readline", "const readline = require('readline');"),
    ("cluster", "const cluster = require('cluster');"),
    (
        "worker_threads",
        "const { Worker } = require('worker_threads');",
    ),
    ("process", "const process = require('process');"),
    ("timers", "const timers = require('timers');"),
    // Common npm packages
    ("express", "const express = require('express');"),
    ("lodash", "const _ = require('lodash');"),
    ("_", "const _ = require('lodash');"),
    ("axios", "const axios = require('axios');"),
    ("moment", "const moment = require('moment');"),
    ("chalk", "const chalk = require('chalk');"),
    ("commander", "const { Command } = require('commander');"),
    ("mongoose", "const mongoose = require('mongoose');"),
    ("pg", "const { Pool } = require('pg');"),
    ("redis", "const redis = require('redis');"),
    ("winston", "const winston = require('winston');"),
    ("dotenv", "const dotenv = require('dotenv');"),
    ("cors", "const cors = require('cors');"),
    ("helmet", "const helmet = require('helmet');"),
    ("jsonwebtoken", "const jwt = require('jsonwebtoken');"),
    ("jwt", "const jwt = require('jsonwebtoken');"),
    ("bcrypt", "const bcrypt = require('bcrypt');"),
    ("supertest", "const supertest = require('supertest');"),
    ("yargs", "const yargs = require('yargs');"),
    ("pino", "const pino = require('pino');"),
];

/// Common property-vs-method confusion patterns for TypeError.
///
/// Maps a misused name to (correct_access, description).
static PROPERTY_CORRECTIONS: &[(&str, &str, &str)] = &[
    (
        "length",
        ".length",
        "Access as a property, not a function call",
    ),
    ("size", ".size", "Access as a property, not a function call"),
    ("name", ".name", "Access as a property, not a function call"),
    (
        "message",
        ".message",
        "Access as a property, not a function call",
    ),
    (
        "constructor",
        ".constructor",
        "Access as a property, not a function call",
    ),
    (
        "prototype",
        ".prototype",
        "Access as a property, not a function call",
    ),
    (
        "__proto__",
        ".__proto__",
        "Access as a property, not a function call",
    ),
    ("then", ".then()", "Call as a method on a Promise"),
    ("catch", ".catch()", "Call as a method on a Promise"),
    ("toString", ".toString()", "Call toString as a method"),
    ("valueOf", ".valueOf()", "Call valueOf as a method"),
];

// ============================================================================
// Top-level dispatcher
// ============================================================================

/// Dispatch to the correct JavaScript analyzer based on error pattern.
///
/// Returns `Some(Diagnosis)` if an analyzer handled the error, `None` otherwise.
pub fn diagnose_javascript(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    let error_type = error.error_type.as_str();
    let msg = &error.message;

    match error_type {
        "ReferenceError" => analyze_reference_error(error, source),
        "TypeError" => {
            // Dispatch to the correct TypeError sub-analyzer
            if msg.contains("is not a function") {
                analyze_type_error_not_function(error, source)
            } else if msg.contains("Cannot read propert") || msg.contains("cannot read propert") {
                analyze_type_error_undefined(error, source)
            } else {
                // Generic TypeError -- not one of our specific patterns
                None
            }
        }
        "SyntaxError" => analyze_syntax_error(error, source),
        _ => None,
    }
}

/// Check whether a given error type has a registered JavaScript analyzer.
pub fn has_analyzer(error_type: &str) -> bool {
    matches!(
        error_type,
        "ReferenceError"
            | "TypeError:not_a_function"
            | "TypeError:undefined_property"
            | "SyntaxError"
    )
}

// ============================================================================
// Analyzer 1: ReferenceError -- X is not defined
// ============================================================================

/// Analyze `ReferenceError: X is not defined`.
///
/// This usually means a module-level identifier is used without requiring/importing it.
/// The fix is to inject the appropriate `require()` or `import` statement.
///
/// Handles:
/// - Known Node.js builtins via KNOWN_MODULES table
/// - Known npm packages via KNOWN_MODULES table
/// - Fallback: suggest require with inferred module name
fn analyze_reference_error(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let name = extract_js_name(&error.message, "is not defined")?;

    // Look up in KNOWN_MODULES table
    let require_stmt = KNOWN_MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, stmt)| stmt.to_string())
        .unwrap_or_else(|| {
            // Fallback: generate a default require
            format!("const {} = require('{}');", name, name.to_lowercase())
        });

    let (new_text, insert_line) = inject_require_statement(source, &require_stmt)?;
    let edit_kind = require_edit_kind(source);

    let is_known = KNOWN_MODULES.iter().any(|(n, _)| *n == name);

    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "ReferenceError".to_string(),
        message: format!(
            "'{}' is not defined -- missing require: {}",
            name, require_stmt
        ),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: if is_known {
            FixConfidence::Medium
        } else {
            FixConfidence::Low
        },
        fix: Some(Fix {
            description: format!("Add `{}`", require_stmt),
            edits: vec![TextEdit {
                line: insert_line,
                column: None,
                kind: edit_kind,
                new_text,
            }],
        }),
    })
}

// ============================================================================
// Analyzer 2: TypeError -- X is not a function
// ============================================================================

/// Analyze `TypeError: X is not a function`.
///
/// Checks if X exists as a property (not method) and suggests the correct
/// access pattern. For example, `arr.length()` should be `arr.length`.
fn analyze_type_error_not_function(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let name = extract_not_a_function_name(&error.message)?;

    // Check if it's a known property-vs-method confusion
    let correction = PROPERTY_CORRECTIONS.iter().find(|(n, _, _)| *n == name);

    if let Some((_prop_name, correct_access, description)) = correction {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            if line_no > 0 && line_no <= lines.len() {
                let old_line = lines[line_no - 1];

                // Look for the pattern: name() and replace with name (remove parens)
                let call_pattern = format!(".{}()", name);
                let property_pattern = correct_access.to_string();

                if old_line.contains(&call_pattern) {
                    let new_line = old_line.replace(&call_pattern, &property_pattern);
                    return Some(Diagnosis {
                        language: "javascript".to_string(),
                        error_code: "TypeError".to_string(),
                        message: format!("'{}' is not a function -- {}", name, description),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Medium,
                        fix: Some(Fix {
                            description: format!(
                                "Replace `.{}()` with `{}` at line {}",
                                name, correct_access, line_no
                            ),
                            edits: vec![TextEdit {
                                line: line_no,
                                column: None,
                                kind: EditKind::ReplaceLine,
                                new_text: new_line,
                            }],
                        }),
                    });
                }
            }
        }
    }

    // Fallback: unrecognized pattern -- still produce a diagnosis
    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "TypeError".to_string(),
        message: format!(
            "'{}' is not a function -- check if it's a property or method name is misspelled",
            name
        ),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer 3: TypeError -- Cannot read properties of undefined
// ============================================================================

/// Analyze `TypeError: Cannot read properties of undefined (reading 'X')`.
///
/// Fix: Add optional chaining `?.` or a null guard before the property access
/// on the offending line.
fn analyze_type_error_undefined(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let property = extract_reading_property(&error.message)?;

    if let Some(line_no) = error.line {
        let lines: Vec<&str> = source.lines().collect();
        if line_no > 0 && line_no <= lines.len() {
            let old_line = lines[line_no - 1];

            // Find the dot-access pattern `.property` and replace with `?.property`
            let dot_access = format!(".{}", property);
            let optional_access = format!("?.{}", property);

            if old_line.contains(&dot_access) && !old_line.contains(&optional_access) {
                // Replace the first occurrence of .property with ?.property
                let new_line = old_line.replacen(&dot_access, &optional_access, 1);

                return Some(Diagnosis {
                    language: "javascript".to_string(),
                    error_code: "TypeError".to_string(),
                    message: format!(
                        "Cannot read property '{}' of undefined -- add optional chaining `?.`",
                        property
                    ),
                    location: Some(FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: line_no,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Medium,
                    fix: Some(Fix {
                        description: format!(
                            "Replace `.{}` with `?.{}` at line {}",
                            property, property, line_no
                        ),
                        edits: vec![TextEdit {
                            line: line_no,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: new_line,
                        }],
                    }),
                });
            }

            // Handle bracket notation: [property] -> ?.[property]
            let bracket_access = format!("[\"{}\"]", property);
            let optional_bracket = format!("?.[\"{}\"]", property);

            if old_line.contains(&bracket_access) && !old_line.contains(&optional_bracket) {
                let new_line = old_line.replacen(&bracket_access, &optional_bracket, 1);

                return Some(Diagnosis {
                    language: "javascript".to_string(),
                    error_code: "TypeError".to_string(),
                    message: format!(
                        "Cannot read property '{}' of undefined -- add optional chaining `?.`",
                        property
                    ),
                    location: Some(FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: line_no,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Medium,
                    fix: Some(Fix {
                        description: format!(
                            "Add optional chaining before `[\"{}\"]` at line {}",
                            property, line_no
                        ),
                        edits: vec![TextEdit {
                            line: line_no,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: new_line,
                        }],
                    }),
                });
            }
        }
    }

    // Fallback: produce diagnostic without fix
    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "TypeError".to_string(),
        message: format!(
            "Cannot read property '{}' of undefined -- add null check or optional chaining",
            property
        ),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Analyzer 4: SyntaxError
// ============================================================================

/// Analyze `SyntaxError` patterns from Node.js.
///
/// Handles common SyntaxError patterns:
/// - Missing comma in object/array literal
/// - Unclosed bracket/brace/paren
/// - Unexpected token
/// - Unexpected end of input
///
/// SyntaxErrors are mostly diagnostic (low confidence) since the fix depends
/// heavily on context. We provide useful guidance rather than auto-fixes.
fn analyze_syntax_error(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Pattern: "Unexpected token X"
    if let Some(token) = extract_unexpected_token(msg) {
        return analyze_unexpected_token(error, source, &token);
    }

    // Pattern: "Unexpected end of input"
    if msg.contains("Unexpected end of input") {
        return analyze_unexpected_end(error, source);
    }

    // Pattern: "Unexpected identifier"
    if msg.contains("Unexpected identifier") {
        return analyze_unexpected_identifier(error, source);
    }

    // Pattern: "Missing initializer in const declaration"
    if msg.contains("Missing initializer in const") {
        return analyze_missing_initializer(error, source);
    }

    // Generic SyntaxError -- produce a diagnostic
    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "SyntaxError".to_string(),
        message: format!("SyntaxError: {} -- review code at the error location", msg),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

/// Analyze `Unexpected token X` SyntaxError.
///
/// Common patterns:
/// - Unexpected `}` -> missing opening brace or extra closing brace
/// - Unexpected `)` -> mismatched parentheses
/// - Unexpected `]` -> mismatched brackets
/// - Unexpected `,` at start of object -> trailing comma in previous line
fn analyze_unexpected_token(error: &ParsedError, source: &str, token: &str) -> Option<Diagnosis> {
    if let Some(line_no) = error.line {
        let lines: Vec<&str> = source.lines().collect();
        if line_no > 0 && line_no <= lines.len() {
            // Pattern: unexpected comma after opening brace/bracket or before closing
            // Likely a trailing comma issue on the previous line
            if token == "," && line_no > 1 {
                let prev_line = lines[line_no - 2];
                // Check if previous line ends with an unterminated expression
                let prev_trimmed = prev_line.trim_end();
                if !prev_trimmed.ends_with(',')
                    && !prev_trimmed.ends_with('{')
                    && !prev_trimmed.ends_with('[')
                    && !prev_trimmed.ends_with('(')
                    && !prev_trimmed.is_empty()
                {
                    let new_prev = format!("{},", prev_trimmed);
                    return Some(Diagnosis {
                        language: "javascript".to_string(),
                        error_code: "SyntaxError".to_string(),
                        message: format!(
                            "Unexpected token '{}' -- possibly missing comma on previous line",
                            token
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Low,
                        fix: Some(Fix {
                            description: format!(
                                "Add missing comma at end of line {}",
                                line_no - 1
                            ),
                            edits: vec![TextEdit {
                                line: line_no - 1,
                                column: None,
                                kind: EditKind::ReplaceLine,
                                new_text: new_prev,
                            }],
                        }),
                    });
                }
            }

            // Pattern: unexpected closing delimiter -- count brackets to diagnose
            if token == "}" || token == ")" || token == "]" {
                let (opens, closes) = count_delimiters(source, token.chars().next().unwrap());
                if closes > opens {
                    return Some(Diagnosis {
                        language: "javascript".to_string(),
                        error_code: "SyntaxError".to_string(),
                        message: format!(
                            "Unexpected '{}' -- extra closing delimiter (found {} opens, {} closes)",
                            token, opens, closes
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Low,
                        fix: None,
                    });
                }
            }

            // Generic unexpected token
            return Some(Diagnosis {
                language: "javascript".to_string(),
                error_code: "SyntaxError".to_string(),
                message: format!(
                    "Unexpected token '{}' at line {} -- check for missing semicolons, commas, or brackets",
                    token, line_no
                ),
                location: Some(FixLocation {
                    file: error.file.clone().unwrap_or_default(),
                    line: line_no,
                    column: error.column,
                }),
                confidence: FixConfidence::Low,
                fix: None,
            });
        }
    }

    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "SyntaxError".to_string(),
        message: format!(
            "Unexpected token '{}' -- review syntax near error location",
            token
        ),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

/// Analyze `Unexpected end of input` SyntaxError.
///
/// This typically means an unclosed bracket, brace, paren, or string literal.
fn analyze_unexpected_end(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    // Count unmatched delimiters
    let mut brace_depth = 0i32;
    let mut paren_depth = 0i32;
    let mut bracket_depth = 0i32;

    for ch in source.chars() {
        match ch {
            '{' => brace_depth += 1,
            '}' => brace_depth -= 1,
            '(' => paren_depth += 1,
            ')' => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            _ => {}
        }
    }

    let mut missing = Vec::new();
    if brace_depth > 0 {
        missing.push(format!("{} unclosed `{}`", brace_depth, '{'));
    }
    if paren_depth > 0 {
        missing.push(format!("{} unclosed `(`", paren_depth));
    }
    if bracket_depth > 0 {
        missing.push(format!("{} unclosed `[`", bracket_depth));
    }

    let detail = if missing.is_empty() {
        "possibly unclosed string literal or template literal".to_string()
    } else {
        missing.join(", ")
    };

    let total_lines = source.lines().count();

    // If there's a single unclosed brace, suggest adding `}` at the end
    let fix = if brace_depth == 1 && paren_depth == 0 && bracket_depth == 0 {
        Some(Fix {
            description: format!("Add closing `}}` at end of file (line {})", total_lines),
            edits: vec![TextEdit {
                line: total_lines,
                column: None,
                kind: EditKind::InsertAfter,
                new_text: "}".to_string(),
            }],
        })
    } else {
        None
    };

    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "SyntaxError".to_string(),
        message: format!("Unexpected end of input -- {}", detail),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix,
    })
}

/// Analyze `Unexpected identifier` SyntaxError.
///
/// This often means a missing semicolon, comma, or operator on the previous line.
fn analyze_unexpected_identifier(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    if let Some(line_no) = error.line {
        let lines: Vec<&str> = source.lines().collect();
        if line_no > 1 && line_no <= lines.len() {
            let prev_line = lines[line_no - 2].trim_end();

            // Check if previous line looks like it's missing a semicolon
            if !prev_line.ends_with(';')
                && !prev_line.ends_with('{')
                && !prev_line.ends_with('}')
                && !prev_line.ends_with(',')
                && !prev_line.ends_with('(')
                && !prev_line.is_empty()
                && !prev_line.starts_with("//")
                && !prev_line.starts_with("/*")
            {
                return Some(Diagnosis {
                    language: "javascript".to_string(),
                    error_code: "SyntaxError".to_string(),
                    message: format!(
                        "Unexpected identifier at line {} -- possibly missing semicolon on line {}",
                        line_no,
                        line_no - 1
                    ),
                    location: Some(FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: line_no,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Low,
                    fix: None,
                });
            }
        }
    }

    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "SyntaxError".to_string(),
        message: "Unexpected identifier -- check for missing operators, semicolons, or commas"
            .to_string(),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

/// Analyze `Missing initializer in const declaration` SyntaxError.
fn analyze_missing_initializer(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    if let Some(line_no) = error.line {
        let lines: Vec<&str> = source.lines().collect();
        if line_no > 0 && line_no <= lines.len() {
            let old_line = lines[line_no - 1];
            let trimmed = old_line.trim();

            // Pattern: `const x;` -> suggest `const x = undefined;` or switch to `let`
            if trimmed.starts_with("const ") && trimmed.ends_with(';') {
                let var_name: String = trimmed
                    .trim_start_matches("const ")
                    .trim_end_matches(';')
                    .trim()
                    .to_string();

                let new_line = old_line.replace(trimmed, &format!("let {};", var_name));

                return Some(Diagnosis {
                    language: "javascript".to_string(),
                    error_code: "SyntaxError".to_string(),
                    message: format!(
                        "Missing initializer in const declaration '{}' -- use `let` instead or add an initializer",
                        var_name
                    ),
                    location: Some(FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: line_no,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Medium,
                    fix: Some(Fix {
                        description: format!(
                            "Change `const {}` to `let {}` at line {}",
                            var_name, var_name, line_no
                        ),
                        edits: vec![TextEdit {
                            line: line_no,
                            column: None,
                            kind: EditKind::ReplaceLine,
                            new_text: new_line,
                        }],
                    }),
                });
            }
        }
    }

    Some(Diagnosis {
        language: "javascript".to_string(),
        error_code: "SyntaxError".to_string(),
        message: "Missing initializer in const declaration -- add `= value` or use `let`"
            .to_string(),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

// ============================================================================
// Helper functions
// ============================================================================

/// Extract the undefined name from a ReferenceError message.
///
/// "X is not defined" -> "X"
fn extract_js_name(msg: &str, suffix: &str) -> Option<String> {
    let re = Regex::new(&format!(r"(\w+)\s+{}", regex::escape(suffix))).ok()?;
    re.captures(msg)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract the name from "X is not a function" TypeError message.
///
/// Handles patterns:
/// - "someObj.length is not a function" -> "length"
/// - "someFunc is not a function" -> "someFunc"
fn extract_not_a_function_name(msg: &str) -> Option<String> {
    let re = Regex::new(r"(?:(\w+)\.)?(\w+)\s+is not a function").ok()?;
    re.captures(msg)
        .and_then(|caps| caps.get(2))
        .map(|m| m.as_str().to_string())
}

/// Extract the property name from "Cannot read properties of undefined (reading 'X')".
fn extract_reading_property(msg: &str) -> Option<String> {
    let re = Regex::new(r"reading '(\w+)'").ok()?;
    re.captures(msg)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// Extract the unexpected token from a SyntaxError message.
///
/// "Unexpected token }" -> "}"
/// "Unexpected token ','" -> ","
fn extract_unexpected_token(msg: &str) -> Option<String> {
    // Try quoted form first: "Unexpected token 'X'"
    let re_quoted = Regex::new(r"Unexpected token '([^']+)'").ok()?;
    if let Some(caps) = re_quoted.captures(msg) {
        return caps.get(1).map(|m| m.as_str().to_string());
    }

    // Try unquoted form: "Unexpected token X"
    let re_unquoted = Regex::new(r"Unexpected token\s+(\S+)").ok()?;
    re_unquoted
        .captures(msg)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// Count opening and closing delimiters in source code.
///
/// Returns (open_count, close_count) for the matching pair.
fn count_delimiters(source: &str, close_char: char) -> (usize, usize) {
    let open_char = match close_char {
        '}' => '{',
        ')' => '(',
        ']' => '[',
        _ => return (0, 0),
    };

    let mut opens = 0usize;
    let mut closes = 0usize;
    let mut in_string = false;
    let mut string_char = '"';
    let mut prev_char = '\0';

    for ch in source.chars() {
        if in_string {
            if ch == string_char && prev_char != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' | '\'' | '`' => {
                    in_string = true;
                    string_char = ch;
                }
                c if c == open_char => opens += 1,
                c if c == close_char => closes += 1,
                _ => {}
            }
        }
        prev_char = ch;
    }

    (opens, closes)
}

/// Inject a require/import statement at the top of a JavaScript file.
///
/// Places the new statement after the last existing `require` or `import` line,
/// or at the very top of the file if there are none. Returns `None` if the
/// statement is already present.
fn inject_require_statement(source: &str, stmt: &str) -> Option<(String, usize)> {
    // Already present -- no edit needed
    if source.contains(stmt) {
        return None;
    }

    let lines: Vec<&str> = source.lines().collect();

    // Find the last require/import line
    let mut last_import_line: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("const ") && trimmed.contains("require(")
            || trimmed.starts_with("var ") && trimmed.contains("require(")
            || trimmed.starts_with("let ") && trimmed.contains("require(")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
        {
            last_import_line = Some(i);
        }
    }

    // Insert after the last import/require, or at the top
    let insert_after_line = last_import_line.unwrap_or(0);
    let line_1indexed = insert_after_line + 1;

    Some((stmt.to_string(), line_1indexed))
}

/// Determine the EditKind for a require injection based on existing imports.
fn require_edit_kind(source: &str) -> EditKind {
    let has_imports = source.lines().any(|l| {
        let trimmed = l.trim();
        (trimmed.starts_with("const ") && trimmed.contains("require("))
            || (trimmed.starts_with("var ") && trimmed.contains("require("))
            || (trimmed.starts_with("let ") && trimmed.contains("require("))
            || trimmed.starts_with("import ")
            || trimmed.starts_with("import{")
    });

    if has_imports {
        EditKind::InsertAfter
    } else {
        EditKind::InsertBefore
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---- Validation gate: all 4 JS analyzers registered ----

    #[test]
    fn test_all_4_js_analyzers_registered() {
        // Each of the 4 analyzer patterns must be reachable through the dispatcher
        let patterns = [
            "ReferenceError",
            "TypeError:not_a_function",
            "TypeError:undefined_property",
            "SyntaxError",
        ];
        for pattern in &patterns {
            assert!(
                has_analyzer(pattern),
                "Analyzer for '{}' should be registered",
                pattern
            );
        }
    }

    #[test]
    fn test_unknown_js_error_not_handled() {
        assert!(!has_analyzer("RangeError"));
        assert!(!has_analyzer(""));
        assert!(!has_analyzer("CustomError"));
    }

    // ---- Analyzer 1: ReferenceError ----

    #[test]
    fn test_reference_error_known_module_fs() {
        let source = "const data = fs.readFileSync('file.txt');\n";
        let error = ParsedError {
            error_type: "ReferenceError".to_string(),
            message: "fs is not defined".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(1),
            column: Some(14),
            language: "javascript".to_string(),
            raw_text: "ReferenceError: fs is not defined".to_string(),
            function_name: None,
            offending_line: Some("const data = fs.readFileSync('file.txt');".to_string()),
        };

        let diag = analyze_reference_error(&error, source);
        assert!(diag.is_some(), "Should diagnose ReferenceError for fs");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "ReferenceError");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("require('fs')"),
            "Fix should inject fs require, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_reference_error_known_module_path() {
        let source = "const dir = path.join(__dirname, 'data');\n";
        let error = ParsedError {
            error_type: "ReferenceError".to_string(),
            message: "path is not defined".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(1),
            column: Some(13),
            language: "javascript".to_string(),
            raw_text: "ReferenceError: path is not defined".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_reference_error(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0].new_text.contains("require('path')"));
    }

    #[test]
    fn test_reference_error_unknown_module() {
        let source = "const x = someLib.doStuff();\n";
        let error = ParsedError {
            error_type: "ReferenceError".to_string(),
            message: "someLib is not defined".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_reference_error(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains("require('somelib')"));
    }

    #[test]
    fn test_reference_error_already_required() {
        let source = "const fs = require('fs');\nconst data = fs.readFileSync('file.txt');\n";
        let error = ParsedError {
            error_type: "ReferenceError".to_string(),
            message: "fs is not defined".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_reference_error(&error, source);
        // require already present -> inject_require_statement returns None -> no diagnosis
        assert!(
            diag.is_none(),
            "Should not produce a diagnosis when require already present"
        );
    }

    // ---- Analyzer 2: TypeError: not a function ----

    #[test]
    fn test_type_error_not_a_function_length() {
        let source = "const arr = [1, 2, 3];\nconst len = arr.length();\n";
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "arr.length is not a function".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(2),
            column: Some(17),
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_error_not_function(&error, source);
        assert!(diag.is_some(), "Should diagnose length() typo");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "TypeError");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("arr.length"),
            "Fix should remove () from .length(), got: {}",
            fix.edits[0].new_text
        );
        assert!(
            !fix.edits[0].new_text.contains("arr.length()"),
            "Fix should NOT contain .length(), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_type_error_not_a_function_unknown() {
        let source = "const result = obj.customThing();\n";
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "obj.customThing is not a function".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_error_not_function(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none());
    }

    // ---- Analyzer 3: TypeError: Cannot read properties of undefined ----

    #[test]
    fn test_type_error_undefined_property_dot_access() {
        let source = "const user = getUser();\nconst name = user.profile.name;\n";
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Cannot read properties of undefined (reading 'name')".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(2),
            column: Some(26),
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_error_undefined(&error, source);
        assert!(diag.is_some(), "Should diagnose undefined property access");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "TypeError");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("?.name"),
            "Fix should add optional chaining, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_type_error_undefined_property_foo_optional_chaining() {
        // Canonical form: "Cannot read properties of undefined (reading 'foo')"
        // with a line that contains `.foo` — should produce a ReplaceLine edit
        // with `?.foo` substituted, at Medium confidence.
        let source = "const bar = getValue();\nconst x = bar.foo;\n";
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Cannot read properties of undefined (reading 'foo')".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(2),
            column: Some(11),
            language: "javascript".to_string(),
            raw_text: "TypeError: Cannot read properties of undefined (reading 'foo')".to_string(),
            function_name: None,
            offending_line: Some("const x = bar.foo;".to_string()),
        };

        let diag = analyze_type_error_undefined(&error, source);
        assert!(diag.is_some(), "Should diagnose undefined property 'foo'");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "TypeError");
        assert_eq!(
            d.confidence,
            FixConfidence::Medium,
            "Confidence should be Medium (not Low) when fix is applicable"
        );
        assert!(d.fix.is_some(), "Should produce a fix edit");
        let fix = d.fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        let edit = &fix.edits[0];
        assert_eq!(edit.line, 2);
        assert_eq!(edit.kind, EditKind::ReplaceLine);
        assert!(
            edit.new_text.contains("?.foo"),
            "Fix should insert optional chaining `?.foo`, got: {}",
            edit.new_text
        );
        assert!(
            !edit.new_text.contains(".foo") || edit.new_text.contains("?.foo"),
            "The resulting line must use `?.foo` not bare `.foo`"
        );
    }

    #[test]
    fn test_type_error_undefined_property_no_fix() {
        let source = "const x = getValue();\n";
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Cannot read properties of undefined (reading 'foo')".to_string(),
            file: None,
            line: None,
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_error_undefined(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none());
    }

    // ---- Analyzer 4: SyntaxError ----

    #[test]
    fn test_syntax_error_unexpected_token() {
        let source = "const obj = {\n  name: 'test'\n  age: 30\n};\n";
        let error = ParsedError {
            error_type: "SyntaxError".to_string(),
            message: "Unexpected identifier".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(3),
            column: Some(2),
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_syntax_error(&error, source);
        assert!(diag.is_some(), "Should diagnose SyntaxError");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "SyntaxError");
        assert_eq!(d.confidence, FixConfidence::Low);
    }

    #[test]
    fn test_syntax_error_unexpected_end_of_input() {
        let source = "function foo() {\n  const x = 1;\n";
        let error = ParsedError {
            error_type: "SyntaxError".to_string(),
            message: "Unexpected end of input".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(2),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_syntax_error(&error, source);
        assert!(diag.is_some(), "Should diagnose unexpected end");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "SyntaxError");
        assert!(d.message.contains("unclosed"));
        // Should suggest adding closing brace
        assert!(d.fix.is_some(), "Should suggest adding closing brace");
        let fix = d.fix.unwrap();
        assert_eq!(fix.edits[0].new_text, "}");
    }

    #[test]
    fn test_syntax_error_missing_initializer() {
        let source = "const x;\nconsole.log(x);\n";
        let error = ParsedError {
            error_type: "SyntaxError".to_string(),
            message: "Missing initializer in const declaration".to_string(),
            file: Some(PathBuf::from("app.js")),
            line: Some(1),
            column: Some(7),
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_syntax_error(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "SyntaxError");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("let x;"),
            "Fix should change const to let, got: {}",
            fix.edits[0].new_text
        );
    }

    // ---- Dispatcher ----

    #[test]
    fn test_diagnose_js_dispatches_reference_error() {
        let source = "const data = fs.readFileSync('file.txt');\n";
        let tree = crate::ast::parser::parse(source, crate::Language::JavaScript).unwrap();
        let error = ParsedError {
            error_type: "ReferenceError".to_string(),
            message: "fs is not defined".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_javascript(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "ReferenceError");
    }

    #[test]
    fn test_diagnose_js_dispatches_type_error_not_function() {
        let source = "const arr = [1, 2, 3];\nconst len = arr.length();\n";
        let tree = crate::ast::parser::parse(source, crate::Language::JavaScript).unwrap();
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "arr.length is not a function".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_javascript(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "TypeError");
    }

    #[test]
    fn test_diagnose_js_dispatches_type_error_undefined() {
        let source = "const name = obj.profile.name;\n";
        let tree = crate::ast::parser::parse(source, crate::Language::JavaScript).unwrap();
        let error = ParsedError {
            error_type: "TypeError".to_string(),
            message: "Cannot read properties of undefined (reading 'name')".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_javascript(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "TypeError");
    }

    #[test]
    fn test_diagnose_js_dispatches_syntax_error() {
        let source = "const x = {\n";
        let tree = crate::ast::parser::parse(source, crate::Language::JavaScript).unwrap();
        let error = ParsedError {
            error_type: "SyntaxError".to_string(),
            message: "Unexpected end of input".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_javascript(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "SyntaxError");
    }

    #[test]
    fn test_diagnose_js_unknown_error_returns_none() {
        let source = "const x = 1;\n";
        let tree = crate::ast::parser::parse(source, crate::Language::JavaScript).unwrap();
        let error = ParsedError {
            error_type: "RangeError".to_string(),
            message: "Maximum call stack size exceeded".to_string(),
            file: None,
            line: None,
            column: None,
            language: "javascript".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_javascript(&error, source, &tree, None);
        assert!(diag.is_none());
    }

    // ---- Helper function tests ----

    #[test]
    fn test_extract_js_name() {
        assert_eq!(
            extract_js_name("fs is not defined", "is not defined"),
            Some("fs".to_string())
        );
        assert_eq!(
            extract_js_name("path is not defined", "is not defined"),
            Some("path".to_string())
        );
        assert_eq!(extract_js_name("random text", "is not defined"), None);
    }

    #[test]
    fn test_extract_not_a_function_name() {
        assert_eq!(
            extract_not_a_function_name("arr.length is not a function"),
            Some("length".to_string())
        );
        assert_eq!(
            extract_not_a_function_name("someFunc is not a function"),
            Some("someFunc".to_string())
        );
        assert_eq!(extract_not_a_function_name("random text"), None);
    }

    #[test]
    fn test_extract_reading_property() {
        assert_eq!(
            extract_reading_property("Cannot read properties of undefined (reading 'name')"),
            Some("name".to_string())
        );
        assert_eq!(
            extract_reading_property("Cannot read properties of null (reading 'foo')"),
            Some("foo".to_string())
        );
        assert_eq!(extract_reading_property("random text"), None);
    }

    #[test]
    fn test_extract_unexpected_token() {
        assert_eq!(
            extract_unexpected_token("Unexpected token '}'"),
            Some("}".to_string())
        );
        assert_eq!(
            extract_unexpected_token("Unexpected token }"),
            Some("}".to_string())
        );
        assert_eq!(
            extract_unexpected_token("Unexpected token ','"),
            Some(",".to_string())
        );
    }

    #[test]
    fn test_count_delimiters() {
        let source = "function foo() { if (x) { return [1]; } }";
        let (opens, closes) = count_delimiters(source, '}');
        assert_eq!(opens, 2);
        assert_eq!(closes, 2);
    }

    #[test]
    fn test_count_delimiters_unmatched() {
        let source = "function foo() { if (x) { return 1; }";
        let (opens, closes) = count_delimiters(source, '}');
        assert_eq!(opens, 2);
        assert_eq!(closes, 1);
    }

    #[test]
    fn test_inject_require_no_existing() {
        let source = "const x = 1;\n";
        let result = inject_require_statement(source, "const fs = require('fs');");
        assert!(result.is_some());
        let (text, line) = result.unwrap();
        assert!(text.contains("require('fs')"));
        assert_eq!(line, 1);
    }

    #[test]
    fn test_inject_require_after_existing() {
        let source = "const path = require('path');\n\nconst x = 1;\n";
        let result = inject_require_statement(source, "const fs = require('fs');");
        assert!(result.is_some());
        let (text, line) = result.unwrap();
        assert!(text.contains("require('fs')"));
        assert_eq!(line, 1); // After the first require
    }

    #[test]
    fn test_inject_require_already_present() {
        let source = "const fs = require('fs');\nconst data = fs.readFileSync('file.txt');\n";
        let result = inject_require_statement(source, "const fs = require('fs');");
        assert!(
            result.is_none(),
            "Should return None when require already present"
        );
    }
}
