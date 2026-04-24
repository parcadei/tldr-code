//! Rust error analyzers -- 5 analyzers ported from FastEdit rust_analyzers.py.
//!
//! Each analyzer is a pure function that takes a `ParsedError`, source code,
//! and a tree-sitter `Tree`, and returns an `Option<Diagnosis>`.
//!
//! # Analyzer Inventory (5 total)
//!
//! | # | Error Code | Analyzer           | Fix                                          |
//! |---|------------|--------------------|----------------------------------------------|
//! | 1 | E0599      | MethodNotFound     | Inject `use <trait>` from TRAIT_IMPORTS table |
//! | 2 | E0277      | TypeMismatch       | Insert `.copied()`, `as usize`, etc.         |
//! | 3 | E0425      | NotInScope         | Inject `use` from KNOWN_ITEMS table          |
//! | 4 | E0433      | FailedToResolve    | Inject `use` from KNOWN_ITEMS table (type-position) |
//! | 5 | E0308      | MismatchedTypes    | Type coercion: &str->String, Option, &       |

use regex::Regex;
use tree_sitter::Tree;

use super::types::{Diagnosis, EditKind, Fix, FixConfidence, FixLocation, ParsedError, TextEdit};

// ============================================================================
// Known-fix lookup tables (data, not code)
// ============================================================================

/// E0599: Method not found -> trait import to add.
///
/// Maps a method name to the `use` statement that brings the required trait
/// into scope. Ported from FastEdit TRAIT_IMPORTS dict.
static TRAIT_IMPORTS: &[(&str, &str)] = &[
    // std::io
    ("read", "use std::io::Read;"),
    ("read_to_string", "use std::io::Read;"),
    ("read_exact", "use std::io::Read;"),
    ("write", "use std::io::Write;"),
    ("write_all", "use std::io::Write;"),
    ("write_fmt", "use std::io::Write;"),
    ("flush", "use std::io::Write;"),
    ("read_line", "use std::io::BufRead;"),
    ("lines", "use std::io::BufRead;"),
    ("seek", "use std::io::Seek;"),
    // std::fmt
    ("write!", "use std::fmt::Write;"),
    // std::str
    ("parse", "use std::str::FromStr;"),
    ("from_str", "use std::str::FromStr;"),
    // std::convert
    ("into", "use std::convert::Into;"),
    ("try_into", "use std::convert::TryInto;"),
    ("try_from", "use std::convert::TryFrom;"),
    ("as_ref", "use std::convert::AsRef;"),
    // std::ops
    ("deref", "use std::ops::Deref;"),
    // std::fmt::Display (for .to_string() on custom types)
    ("display", "use std::fmt::Display;"),
    // std::iter (usually in prelude, but just in case)
    ("collect", "use std::iter::Iterator;"),
];

/// E0425: Not in scope -> use statement to add.
///
/// Maps a type/module name to the `use` statement that brings it into scope.
/// Ported from FastEdit KNOWN_IMPORTS dict.
static KNOWN_ITEMS: &[(&str, &str)] = &[
    ("HashMap", "use std::collections::HashMap;"),
    ("BTreeMap", "use std::collections::BTreeMap;"),
    ("HashSet", "use std::collections::HashSet;"),
    ("BTreeSet", "use std::collections::BTreeSet;"),
    ("VecDeque", "use std::collections::VecDeque;"),
    ("BinaryHeap", "use std::collections::BinaryHeap;"),
    ("Arc", "use std::sync::Arc;"),
    ("Mutex", "use std::sync::Mutex;"),
    ("RwLock", "use std::sync::RwLock;"),
    ("Sender", "use std::sync::mpsc::Sender;"),
    ("Receiver", "use std::sync::mpsc::Receiver;"),
    ("Path", "use std::path::Path;"),
    ("PathBuf", "use std::path::PathBuf;"),
    ("File", "use std::fs::File;"),
    ("OpenOptions", "use std::fs::OpenOptions;"),
    ("Duration", "use std::time::Duration;"),
    ("Instant", "use std::time::Instant;"),
    ("Ordering", "use std::cmp::Ordering;"),
    ("Reverse", "use std::cmp::Reverse;"),
    ("thread", "use std::thread;"),
];

// ============================================================================
// Top-level dispatcher
// ============================================================================

/// Dispatch to the correct Rust analyzer based on error code.
///
/// Returns `Some(Diagnosis)` if an analyzer handled the error, `None` otherwise.
pub fn diagnose_rust(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    let error_code = error.error_type.as_str();

    match error_code {
        "E0599" => analyze_e0599(error, source),
        "E0277" => analyze_e0277(error, source),
        "E0425" => analyze_e0425(error, source),
        "E0433" => analyze_e0433(error, source),
        "E0308" => analyze_e0308(error, source),
        _ => None,
    }
}

/// Check whether a given error code has a registered Rust analyzer.
pub fn has_analyzer(error_code: &str) -> bool {
    matches!(error_code, "E0599" | "E0277" | "E0425" | "E0433" | "E0308")
}

// ============================================================================
// Shared helper: inject a `use` statement into Rust source
// ============================================================================

/// Inject a `use` statement into Rust source code.
///
/// Places the new import after the last existing `use` line, or at the top
/// of the file if there are no existing imports. Returns `None` if the import
/// is already present.
fn inject_use_statement(source: &str, use_stmt: &str) -> Option<(String, usize)> {
    // Already present -- no edit needed
    if source.contains(use_stmt) {
        return None;
    }

    let lines: Vec<&str> = source.lines().collect();

    // Find the last `use` line
    let mut last_use_line: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
            last_use_line = Some(i);
        }
    }

    // Insert after the last use, or at the top
    let insert_after_line = last_use_line.unwrap_or(0);
    let line_1indexed = insert_after_line + 1;

    let edit_kind = if last_use_line.is_some() {
        EditKind::InsertAfter
    } else {
        // No existing use statements: insert before line 1 (top of file)
        EditKind::InsertBefore
    };

    let new_text = if last_use_line.is_some() {
        use_stmt.to_string()
    } else {
        // At the top, add a blank line after the import for readability
        format!("{}\n", use_stmt)
    };

    // Compute result text for verification
    let mut result_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    match edit_kind {
        EditKind::InsertAfter => {
            result_lines.insert(insert_after_line + 1, use_stmt.to_string());
        }
        EditKind::InsertBefore => {
            result_lines.insert(0, use_stmt.to_string());
            result_lines.insert(1, String::new());
        }
        _ => {}
    }

    let mut result = result_lines.join("\n");
    if source.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }

    Some((new_text, line_1indexed))
}

// ============================================================================
// Analyzer 1: E0599 -- Method not found (missing trait import)
// ============================================================================

/// Analyze E0599: no method named `X` found.
///
/// This usually means a trait method is being called without the trait in scope.
/// The fix is to inject the appropriate `use` statement.
///
/// Handles:
/// - Direct method name lookup in TRAIT_IMPORTS
/// - Compiler hint extraction from children messages
/// - "cannot write"/"cannot read" fallback patterns
fn analyze_e0599(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Extract method name: "no method named `read_line` found"
    let method_re = Regex::new(r"no method named `(\w+)` found").ok()?;

    let use_stmt = if let Some(caps) = method_re.captures(msg) {
        let method = caps.get(1).unwrap().as_str();

        // Look up in TRAIT_IMPORTS table
        let from_table = TRAIT_IMPORTS
            .iter()
            .find(|(m, _)| *m == method)
            .map(|(_, stmt)| *stmt);

        if let Some(stmt) = from_table {
            if stmt.is_empty() {
                // Method is usually in prelude, no import needed
                return Some(Diagnosis {
                    language: "rust".to_string(),
                    error_code: "E0599".to_string(),
                    message: format!("Method `{}` not found -- may need turbofish syntax", method),
                    location: error.line.map(|l| FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: l,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Low,
                    fix: None,
                });
            }
            stmt.to_string()
        } else {
            // Check compiler hint in children: "the following trait is implemented
            // but not in scope; perhaps add a `use` for it"
            let hint = extract_compiler_hint(&error.raw_text);
            if let Some(h) = hint {
                h
            } else {
                // No known fix
                return Some(Diagnosis {
                    language: "rust".to_string(),
                    error_code: "E0599".to_string(),
                    message: format!("Unknown method `{}` -- needs manual fix", method),
                    location: error.line.map(|l| FixLocation {
                        file: error.file.clone().unwrap_or_default(),
                        line: l,
                        column: error.column,
                    }),
                    confidence: FixConfidence::Low,
                    fix: None,
                });
            }
        }
    } else {
        // Handle "cannot write"/"cannot read" style messages
        if msg.to_lowercase().contains("cannot write") || msg.contains("Write") {
            "use std::io::Write;".to_string()
        } else if msg.to_lowercase().contains("cannot read") || msg.contains("Read") {
            "use std::io::Read;".to_string()
        } else {
            return None;
        }
    };

    // Build the fix
    let (new_text, insert_line) = inject_use_statement(source, &use_stmt)?;

    let edit_kind = if source.lines().any(|l| {
        let t = l.trim();
        t.starts_with("use ") || t.starts_with("pub use ")
    }) {
        EditKind::InsertAfter
    } else {
        EditKind::InsertBefore
    };

    Some(Diagnosis {
        language: "rust".to_string(),
        error_code: "E0599".to_string(),
        message: format!("Method not found -- missing trait import: {}", use_stmt),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Add `{}`", use_stmt),
            edits: vec![TextEdit {
                line: insert_line,
                column: None,
                kind: edit_kind,
                new_text,
            }],
        }),
    })
}

/// Extract a compiler-suggested `use` statement from raw error text.
///
/// Looks for patterns like: `use std::io::BufRead;` in compiler hint messages.
fn extract_compiler_hint(raw_text: &str) -> Option<String> {
    let hint_re = Regex::new(r"`use ([\w:]+);`").ok()?;
    if let Some(caps) = hint_re.captures(raw_text) {
        let path = caps.get(1).unwrap().as_str();
        return Some(format!("use {};", path));
    }
    None
}

// ============================================================================
// Analyzer 2: E0277 -- Type mismatch (.copied(), as usize, etc.)
// ============================================================================

/// Analyze E0277: the trait bound `X: Y` is not satisfied.
///
/// Common patterns:
/// - "cannot be indexed by `u32`" -> cast index to `usize`
/// - "cannot be built from an iterator over elements of type `&T`" -> `.copied()`
fn analyze_e0277(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Pattern 1: "cannot be indexed by `u32`" -> cast to usize
    if msg.contains("cannot be indexed by") && msg.contains("u32") {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            if line_no > 0 && line_no <= lines.len() {
                let old_line = lines[line_no - 1];
                // Find pattern like `items.get(idx)` and add `as usize`
                let cast_re = Regex::new(r"\b(\w+)\s*\)").ok()?;
                let new_line = cast_re.replace(old_line, "$1 as usize)").to_string();

                if new_line != old_line {
                    return Some(Diagnosis {
                        language: "rust".to_string(),
                        error_code: "E0277".to_string(),
                        message: format!(
                            "Index type mismatch -- cast to usize at line {}",
                            line_no
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Medium,
                        fix: Some(Fix {
                            description: format!("Cast index to `usize` at line {}", line_no),
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

    // Pattern 2: "cannot be built from an iterator over elements of type `&T`"
    // Fix: add .copied() before .collect()
    if msg.contains("cannot be built from an iterator") && msg.contains('&') {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            // Search nearby lines for .collect() without .copied()/.cloned()
            let search_start = line_no.saturating_sub(3).max(0);
            let search_end = (line_no + 3).min(lines.len());

            for (i, line) in lines[search_start..search_end].iter().enumerate() {
                if line.contains(".collect()")
                    && !line.contains(".copied()")
                    && !line.contains(".cloned()")
                {
                    let actual_line = search_start + i;
                    let new_line = line.replace(".collect()", ".copied().collect()");
                    return Some(Diagnosis {
                        language: "rust".to_string(),
                        error_code: "E0277".to_string(),
                        message: format!(
                            "Iterator yields references -- insert `.copied()` before `.collect()` at line {}",
                            actual_line + 1
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: actual_line + 1,
                            column: None,
                        }),
                        confidence: FixConfidence::Medium,
                        fix: Some(Fix {
                            description: format!(
                                "Insert `.copied()` before `.collect()` at line {}",
                                actual_line + 1
                            ),
                            edits: vec![TextEdit {
                                line: actual_line + 1,
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

    // Fallback: unrecognized E0277 pattern
    Some(Diagnosis {
        language: "rust".to_string(),
        error_code: "E0277".to_string(),
        message: format!("Type mismatch: {}", msg),
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
// Analyzer 3: E0425 -- Not found in scope (inject known use)
// ============================================================================

/// Analyze E0425: cannot find value/type `X` in this scope.
///
/// Looks up the missing name in the KNOWN_ITEMS table and injects the
/// appropriate `use` statement.
fn analyze_e0425(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Extract the unresolved name
    let name = extract_unresolved_name(msg)?;

    // Look up in KNOWN_ITEMS table
    let use_stmt = KNOWN_ITEMS
        .iter()
        .find(|(item, _)| *item == name)
        .map(|(_, stmt)| *stmt)?;

    // Build the fix
    let (new_text, insert_line) = inject_use_statement(source, use_stmt)?;

    let edit_kind = if source.lines().any(|l| {
        let t = l.trim();
        t.starts_with("use ") || t.starts_with("pub use ")
    }) {
        EditKind::InsertAfter
    } else {
        EditKind::InsertBefore
    };

    Some(Diagnosis {
        language: "rust".to_string(),
        error_code: "E0425".to_string(),
        message: format!("`{}` not in scope -- add `{}`", name, use_stmt),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Add `{}`", use_stmt),
            edits: vec![TextEdit {
                line: insert_line,
                column: None,
                kind: edit_kind,
                new_text,
            }],
        }),
    })
}

/// Extract the unresolved name from an E0425 error message.
///
/// Handles patterns:
/// - "cannot find type `HashMap` in this scope"
/// - "cannot find value `thread` in this scope"
/// - "`HashMap` not found"
/// - "not found in this scope...`HashMap`"
fn extract_unresolved_name(msg: &str) -> Option<String> {
    // Try "cannot find type/value `X` in this scope"
    if let Some(caps) = Regex::new(r"cannot find (?:type|value) `(\w+)`")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // Try "not found in this scope...`X`" or "`X` not found"
    if let Some(caps) = Regex::new(r"`(\w+)` not found")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // Try "not found in this scope.*`X`"
    if let Some(caps) = Regex::new(r"not found in this scope.*`(\w+)`")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    None
}

// ============================================================================
// Analyzer 4: E0433 -- Failed to resolve (type-position missing use)
// ============================================================================

/// Analyze E0433: failed to resolve: use of undeclared type `X`.
///
/// This is the type-position counterpart to E0425 (value-position). `rustc`
/// emits E0433 when a type name like `HashMap` appears in type position
/// (e.g., `let m: HashMap<...> = HashMap::new()`) without the corresponding
/// `use` statement. E0433 is the MOST COMMON error for missing `use`
/// statements in practice.
///
/// The fix is identical to E0425: look up the type in KNOWN_ITEMS and inject
/// the appropriate `use` statement.
fn analyze_e0433(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Extract the type name from the error message.
    // Patterns:
    //   "failed to resolve: use of undeclared type `HashMap`"
    //   "failed to resolve: use of undeclared crate or module `HashMap`"
    //   "could not find `HashMap` in `std`"
    let type_name = extract_failed_resolve_name(msg)?;

    // Look up in KNOWN_ITEMS table
    let use_stmt_opt = KNOWN_ITEMS
        .iter()
        .find(|(item, _)| *item == type_name)
        .map(|(_, stmt)| *stmt);

    let use_stmt = match use_stmt_opt {
        Some(stmt) => stmt,
        None => {
            // Unknown type: return diagnosis without fix
            return Some(Diagnosis {
                language: "rust".to_string(),
                error_code: "E0433".to_string(),
                message: format!(
                    "Failed to resolve `{}` -- not in known items table",
                    type_name
                ),
                location: error.line.map(|l| FixLocation {
                    file: error.file.clone().unwrap_or_default(),
                    line: l,
                    column: error.column,
                }),
                confidence: FixConfidence::Low,
                fix: None,
            });
        }
    };

    // Build the fix
    let (new_text, insert_line) = inject_use_statement(source, use_stmt)?;

    let edit_kind = if source.lines().any(|l| {
        let t = l.trim();
        t.starts_with("use ") || t.starts_with("pub use ")
    }) {
        EditKind::InsertAfter
    } else {
        EditKind::InsertBefore
    };

    Some(Diagnosis {
        language: "rust".to_string(),
        error_code: "E0433".to_string(),
        message: format!("`{}` not resolved -- add `{}`", type_name, use_stmt),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Add `{}`", use_stmt),
            edits: vec![TextEdit {
                line: insert_line,
                column: None,
                kind: edit_kind,
                new_text,
            }],
        }),
    })
}

/// Extract the unresolved type name from an E0433 error message.
///
/// Handles patterns:
/// - "failed to resolve: use of undeclared type `HashMap`"
/// - "failed to resolve: use of undeclared crate or module `HashMap`"
/// - "could not find `HashMap` in `std`"
/// - "use of undeclared type `HashMap`"
fn extract_failed_resolve_name(msg: &str) -> Option<String> {
    // Try "use of undeclared type `X`"
    if let Some(caps) = Regex::new(r"use of undeclared (?:type|crate or module) `(\w+)`")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // Try "could not find `X` in"
    if let Some(caps) = Regex::new(r"could not find `(\w+)` in")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // Try "failed to resolve.*`X`"
    if let Some(caps) = Regex::new(r"failed to resolve.*`(\w+)`")
        .ok()
        .and_then(|re| re.captures(msg))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    None
}

// ============================================================================
// Analyzer 5: E0308 -- Mismatched types (&str vs String, Option, etc.)
// ============================================================================

/// Analyze E0308: mismatched types.
///
/// Common patterns:
/// - Pattern 1/2: `.cloned()` or `.ok_or()` with &str/String mismatch
/// - Pattern 3: expected `String`, found `&str` -> add `.to_string()`
/// - Pattern 4: expected `&str`, found `String` -> add `&` borrowing
/// - Pattern 5: expected `T`, found `&T` (or vice versa) -> add `*` or `&`
fn analyze_e0308(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;
    // Combine message and raw_text for broader pattern matching
    let full_text = format!("{} {}", msg, error.raw_text);

    // Extract expected/found types from error message for Pattern 4/5
    let type_mismatch_re = Regex::new(r"expected `([^`]+)`, found `([^`]+)`").ok();
    let (expected_type, found_type) = type_mismatch_re
        .as_ref()
        .and_then(|re| re.captures(&full_text))
        .map(|caps| {
            (
                caps.get(1).unwrap().as_str().to_string(),
                caps.get(2).unwrap().as_str().to_string(),
            )
        })
        .unwrap_or_default();

    // Pattern 1/2: &str vs String conversion (specific subpatterns first)
    if full_text.contains("String") && full_text.contains("&str") {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            if line_no > 0 && line_no <= lines.len() {
                let old_line = lines[line_no - 1];
                let new_line;

                // Subpattern 1: .cloned() present -> replace with .map(|s| s.to_string())
                if old_line.contains(".cloned()") {
                    new_line = old_line.replace(".cloned()", ".map(|s| s.to_string())");
                } else if old_line.contains(".ok_or(") && !old_line.contains(".map(") {
                    // Subpattern 2: .ok_or() without prior .map() -> insert .map(|s| s.to_string())
                    new_line = old_line.replace(".ok_or(", ".map(|s| s.to_string()).ok_or(");
                } else if expected_type == "String" && found_type == "&str" {
                    // Pattern 3: generic "expected String, found &str"
                    // Append .to_string() to the rightmost expression before ; or )
                    new_line = apply_to_string_coercion(old_line);
                } else if expected_type == "&str" && found_type == "String" {
                    // Pattern 4: "expected &str, found String"
                    // Prepend & to the rhs expression
                    new_line = apply_borrow_coercion(old_line);
                } else {
                    // Cannot determine specific fix
                    new_line = old_line.to_string();
                }

                if new_line != old_line {
                    let description = if expected_type == "&str" {
                        format!("Borrow `String` as `&str` at line {}", line_no)
                    } else {
                        format!("Convert `&str` to `String` at line {}", line_no)
                    };
                    return Some(Diagnosis {
                        language: "rust".to_string(),
                        error_code: "E0308".to_string(),
                        message: format!(
                            "Type mismatch: expected `{}`, found `{}` at line {}",
                            expected_type, found_type, line_no
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Medium,
                        fix: Some(Fix {
                            description,
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

    // Pattern 5: Reference mismatch (expected T, found &T or expected &T, found T)
    // This handles non-String/&str cases like i32/&i32, u64/&u64, etc.
    if !expected_type.is_empty() && !found_type.is_empty() {
        // Check: expected `T`, found `&T` -> dereference with *
        let needs_deref =
            found_type.starts_with('&') && expected_type == found_type.trim_start_matches('&');
        // Check: expected `&T`, found `T` -> add &
        let needs_ref =
            expected_type.starts_with('&') && found_type == expected_type.trim_start_matches('&');

        if needs_deref || needs_ref {
            if let Some(line_no) = error.line {
                let lines: Vec<&str> = source.lines().collect();
                if line_no > 0 && line_no <= lines.len() {
                    let old_line = lines[line_no - 1];
                    let new_line = if needs_deref {
                        apply_deref_coercion(old_line)
                    } else {
                        apply_borrow_coercion(old_line)
                    };

                    if new_line != old_line {
                        let description = if needs_deref {
                            format!(
                                "Dereference `{}` to `{}` at line {}",
                                found_type, expected_type, line_no
                            )
                        } else {
                            format!(
                                "Borrow `{}` as `{}` at line {}",
                                found_type, expected_type, line_no
                            )
                        };
                        return Some(Diagnosis {
                            language: "rust".to_string(),
                            error_code: "E0308".to_string(),
                            message: format!(
                                "Type mismatch: expected `{}`, found `{}` at line {}",
                                expected_type, found_type, line_no
                            ),
                            location: Some(FixLocation {
                                file: error.file.clone().unwrap_or_default(),
                                line: line_no,
                                column: error.column,
                            }),
                            confidence: FixConfidence::Medium,
                            fix: Some(Fix {
                                description,
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
    }

    // Fallback: unrecognized E0308 pattern
    Some(Diagnosis {
        language: "rust".to_string(),
        error_code: "E0308".to_string(),
        message: format!("Mismatched types: {}", msg),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::Low,
        fix: None,
    })
}

/// Apply `.to_string()` coercion to a source line.
///
/// Heuristic: find the rightmost expression before `;` or `)` and append
/// `.to_string()`. Handles assignment RHS (`= EXPR;`), function arguments
/// (`fn(EXPR)`), and string literals (`"..."`).
fn apply_to_string_coercion(line: &str) -> String {
    let trimmed = line.trim();

    // Case 1: Assignment `= EXPR;` -- append .to_string() to EXPR
    if let Some(caps) = Regex::new(r"^(.*=\s*)(.+?)\s*;\s*$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let expr = caps.get(2).unwrap().as_str();
        // Don't double-apply if already has .to_string()
        if expr.ends_with(".to_string()") || expr.ends_with(".to_owned()") {
            return line.to_string();
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}{}.to_string();", indent, prefix, expr);
    }

    // Case 2: Function call `fn(EXPR)` at end of line (possibly with ;)
    // Find the last identifier or string literal before ) or );
    if let Some(caps) = Regex::new(r"^(.*\(\s*)(\w+)(\s*\)\s*;?\s*)$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let arg = caps.get(2).unwrap().as_str();
        let suffix = caps.get(3).unwrap().as_str();
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}{}.to_string(){}", indent, prefix, arg, suffix);
    }

    // No recognized pattern -- return unchanged
    line.to_string()
}

/// Apply `&` borrow coercion to a source line.
///
/// Heuristic: find the RHS expression in `= EXPR;` or the argument in
/// `fn(EXPR)` and prepend `&`.
fn apply_borrow_coercion(line: &str) -> String {
    let trimmed = line.trim();

    // Case 1: Assignment `= EXPR;` -- prepend & to EXPR
    if let Some(caps) = Regex::new(r"^(.*=\s*)(.+?)\s*;\s*$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let expr = caps.get(2).unwrap().as_str();
        // Don't double-borrow
        if expr.starts_with('&') {
            return line.to_string();
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}&{};", indent, prefix, expr);
    }

    // Case 2: Function call `fn(EXPR)` -- prepend & to the last arg
    if let Some(caps) = Regex::new(r"^(.*\(\s*)(\w+)(\s*\)\s*;?\s*)$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let arg = caps.get(2).unwrap().as_str();
        let suffix = caps.get(3).unwrap().as_str();
        // Don't double-borrow
        if arg.starts_with('&') {
            return line.to_string();
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}&{}{}", indent, prefix, arg, suffix);
    }

    // No recognized pattern -- return unchanged
    line.to_string()
}

/// Apply `*` dereference coercion to a source line.
///
/// Heuristic: find the RHS expression in `= EXPR;` or the argument in
/// `fn(EXPR)` and prepend `*`.
fn apply_deref_coercion(line: &str) -> String {
    let trimmed = line.trim();

    // Case 1: Assignment `= EXPR;` -- prepend * to EXPR
    if let Some(caps) = Regex::new(r"^(.*=\s*)(.+?)\s*;\s*$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let expr = caps.get(2).unwrap().as_str();
        // Don't double-deref
        if expr.starts_with('*') {
            return line.to_string();
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}*{};", indent, prefix, expr);
    }

    // Case 2: Function call `fn(EXPR)` -- prepend * to the last arg
    if let Some(caps) = Regex::new(r"^(.*\(\s*)(\w+)(\s*\)\s*;?\s*)$")
        .ok()
        .and_then(|re| re.captures(trimmed))
    {
        let prefix = caps.get(1).unwrap().as_str();
        let arg = caps.get(2).unwrap().as_str();
        let suffix = caps.get(3).unwrap().as_str();
        let indent = &line[..line.len() - line.trim_start().len()];
        return format!("{}{}*{}{}", indent, prefix, arg, suffix);
    }

    // No recognized pattern -- return unchanged
    line.to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---- Validation gate: all 4 analyzers registered ----

    #[test]
    fn test_all_5_rust_analyzers_registered_original_4() {
        let error_codes = ["E0599", "E0277", "E0425", "E0308"];
        for code in &error_codes {
            assert!(
                has_analyzer(code),
                "Analyzer for {} should be registered",
                code
            );
        }
    }

    #[test]
    fn test_unknown_error_code_not_handled() {
        assert!(!has_analyzer("E9999"));
        assert!(!has_analyzer(""));
        assert!(!has_analyzer("E0001"));
    }

    // ---- E0599: Method not found ----

    #[test]
    fn test_rust_e0599_missing_trait_read_line() {
        let source = "fn main() {\n    let mut buf = String::new();\n    std::io::stdin().read_line(&mut buf);\n}\n";
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "no method named `read_line` found for struct `Stdin` in the current scope".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: Some(21),
            language: "rust".to_string(),
            raw_text: r#"{"code":"E0599","message":"no method named `read_line` found for struct `Stdin` in the current scope","children":[{"message":"the following trait is implemented but not in scope; perhaps add a `use` for it: `use std::io::BufRead;`"}]}"#.to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0599(&error, source);
        assert!(diag.is_some(), "Should diagnose E0599 for read_line");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "E0599");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("use std::io::BufRead;"),
            "Fix should inject BufRead import, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0599_write_trait() {
        let source = "use std::fs::File;\n\nfn main() {\n    let f = File::create(\"out.txt\").unwrap();\n    f.write_all(b\"hello\");\n}\n";
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "no method named `write_all` found for struct `File`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(5),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0599(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(fix.edits[0].new_text.contains("use std::io::Write;"));
    }

    #[test]
    fn test_rust_e0599_already_imported() {
        let source = "use std::io::BufRead;\n\nfn main() {\n    let mut buf = String::new();\n    std::io::stdin().read_line(&mut buf);\n}\n";
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "no method named `read_line` found for struct `Stdin`".to_string(),
            file: None,
            line: Some(5),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        // Import already present -- should return None (no fix needed)
        let diag = analyze_e0599(&error, source);
        assert!(
            diag.is_none() || diag.as_ref().unwrap().fix.is_none(),
            "Should not produce a fix when import already present"
        );
    }

    #[test]
    fn test_rust_e0599_compiler_hint_extraction() {
        let raw = r#"error[E0599]: no method named `read_line` found
--> src/main.rs:3:21
help: the following trait is implemented but not in scope; perhaps add a `use` for it
  |  `use std::io::BufRead;`"#;
        let hint = extract_compiler_hint(raw);
        assert_eq!(hint, Some("use std::io::BufRead;".to_string()));
    }

    #[test]
    fn test_rust_e0599_unknown_method() {
        let source = "fn main() {\n    let x = 42;\n    x.frobnicate();\n}\n";
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "no method named `frobnicate` found for type `i32`".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0599(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none(), "Unknown method should not produce a fix");
    }

    #[test]
    fn test_rust_e0599_cannot_write_fallback() {
        let source = "fn main() {}\n";
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "cannot write to output -- Write trait missing".to_string(),
            file: None,
            line: Some(1),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0599(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains("use std::io::Write;"));
    }

    // ---- E0277: Type mismatch ----

    #[test]
    fn test_rust_e0277_iterator_copied() {
        let source = "fn main() {\n    let v = vec![1, 2, 3];\n    let w: Vec<i32> = v.iter().collect();\n}\n";
        let error = ParsedError {
            error_type: "E0277".to_string(),
            message: "a value of type `Vec<i32>` cannot be built from an iterator over elements of type `&i32`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0277(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "E0277");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains(".copied().collect()"),
            "Fix should insert .copied(), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0277_index_cast_usize() {
        let source = "fn main() {\n    let items = vec![\"a\", \"b\", \"c\"];\n    let idx: u32 = 1;\n    let val = items.get(idx);\n}\n";
        let error = ParsedError {
            error_type: "E0277".to_string(),
            message: "the type `[&str]` cannot be indexed by `u32`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0277(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("as usize"),
            "Fix should cast to usize, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0277_unrecognized_pattern() {
        let error = ParsedError {
            error_type: "E0277".to_string(),
            message: "the trait `Foo` is not implemented for `Bar`".to_string(),
            file: None,
            line: None,
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0277(&error, "fn main() {}\n");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none());
    }

    // ---- E0425: Not in scope ----

    #[test]
    fn test_rust_e0425_hashmap() {
        let source = "fn main() {\n    let m: HashMap<String, i32> = HashMap::new();\n}\n";
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "cannot find type `HashMap` in this scope".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(2),
            column: Some(12),
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(diag.is_some(), "Should diagnose E0425 for HashMap");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "E0425");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0]
                .new_text
                .contains("use std::collections::HashMap;"),
            "Fix should inject HashMap import, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0425_pathbuf() {
        let source = "fn main() {\n    let p = PathBuf::from(\"/tmp\");\n}\n";
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "cannot find type `PathBuf` in this scope".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains("use std::path::PathBuf;"));
    }

    #[test]
    fn test_rust_e0425_arc() {
        let source = "fn main() {\n    let x = Arc::new(42);\n}\n";
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "`Arc` not found in this scope".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(diag.is_some());
        let fix = diag.unwrap().fix.unwrap();
        assert!(fix.edits[0].new_text.contains("use std::sync::Arc;"));
    }

    #[test]
    fn test_rust_e0425_unknown_item() {
        let source = "fn main() {\n    let x = SomeRandomThing::new();\n}\n";
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "cannot find value `SomeRandomThing` in this scope".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(
            diag.is_none(),
            "Unknown items should return None (not in KNOWN_ITEMS)"
        );
    }

    #[test]
    fn test_rust_e0425_already_imported() {
        let source = "use std::collections::HashMap;\n\nfn main() {\n    let m: HashMap<String, i32> = HashMap::new();\n}\n";
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "cannot find type `HashMap` in this scope".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(
            diag.is_none(),
            "Should return None when import already present"
        );
    }

    // ---- E0308: Mismatched types ----

    #[test]
    fn test_rust_e0308_str_to_string_cloned() {
        let source = "fn get_name(data: &[(&str, i32)]) -> Option<String> {\n    data.iter().find(|(k,_)| *k == \"name\").map(|(k,_)| k).cloned()\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `Option<String>`, found `Option<&&str>`"
                .to_string(),
            file: Some(PathBuf::from("lib.rs")),
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.error_code, "E0308");
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains(".map(|s| s.to_string())"),
            "Fix should replace .cloned() with .map(|s| s.to_string()), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_ok_or_pattern() {
        let source = "fn lookup(map: &std::collections::HashMap<String, &str>, key: &str) -> Result<String, String> {\n    map.get(key).ok_or(\"not found\".to_string())\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `String`, found `&str`".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0]
                .new_text
                .contains(".map(|s| s.to_string()).ok_or("),
            "Fix should insert .map(|s| s.to_string()) before .ok_or(), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_unrecognized_pattern() {
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `bool`, found `()`".to_string(),
            file: None,
            line: None,
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, "fn main() {}\n");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none());
    }

    // ---- Dispatcher ----

    #[test]
    fn test_diagnose_rust_dispatches_e0599() {
        let source = "fn main() {\n    let mut buf = String::new();\n    std::io::stdin().read_line(&mut buf);\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Rust).unwrap();
        let error = ParsedError {
            error_type: "E0599".to_string(),
            message: "no method named `read_line` found for struct `Stdin`".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_rust(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "E0599");
    }

    #[test]
    fn test_diagnose_rust_dispatches_e0425() {
        let source = "fn main() {\n    let m: HashMap<String, i32> = HashMap::new();\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Rust).unwrap();
        let error = ParsedError {
            error_type: "E0425".to_string(),
            message: "cannot find type `HashMap` in this scope".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_rust(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "E0425");
    }

    #[test]
    fn test_diagnose_rust_unknown_code_returns_none() {
        let source = "fn main() {}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Rust).unwrap();
        let error = ParsedError {
            error_type: "E9999".to_string(),
            message: "something weird happened".to_string(),
            file: None,
            line: None,
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_rust(&error, source, &tree, None);
        assert!(diag.is_none());
    }

    // ---- inject_use_statement helper ----

    #[test]
    fn test_inject_use_no_existing_imports() {
        let source = "fn main() {\n    println!(\"hello\");\n}\n";
        let result = inject_use_statement(source, "use std::collections::HashMap;");
        assert!(result.is_some());
        let (new_text, line) = result.unwrap();
        assert!(new_text.contains("use std::collections::HashMap;"));
        assert_eq!(line, 1);
    }

    #[test]
    fn test_inject_use_after_existing_imports() {
        let source = "use std::fs::File;\n\nfn main() {}\n";
        let result = inject_use_statement(source, "use std::io::Write;");
        assert!(result.is_some());
        let (new_text, line) = result.unwrap();
        assert!(new_text.contains("use std::io::Write;"));
        assert_eq!(line, 1);
    }

    #[test]
    fn test_inject_use_already_present() {
        let source = "use std::io::Write;\n\nfn main() {}\n";
        let result = inject_use_statement(source, "use std::io::Write;");
        assert!(
            result.is_none(),
            "Should return None when import already present"
        );
    }

    // ---- Integration with fixture files ----

    #[test]
    fn test_fixture_missing_trait() {
        let source = include_str!("../../tests/fixtures/fix/rust/missing_trait.rs");
        let error_json = include_str!("../../tests/fixtures/fix/rust/missing_trait.error.json");
        let expected = include_str!("../../tests/fixtures/fix/rust/missing_trait.fixed.rs");

        let error_data: serde_json::Value = serde_json::from_str(error_json).unwrap();
        let error = ParsedError {
            error_type: error_data["code"].as_str().unwrap().to_string(),
            message: error_data["message"].as_str().unwrap().to_string(),
            file: Some(PathBuf::from("missing_trait.rs")),
            line: error_data["line"].as_u64().map(|l| l as usize),
            column: error_data["col"].as_u64().map(|c| c as usize),
            language: "rust".to_string(),
            raw_text: error_json.to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0599(&error, source);
        assert!(diag.is_some(), "Should diagnose the missing_trait fixture");
        let d = diag.unwrap();
        assert!(d.fix.is_some(), "Should produce a fix");

        // Apply the fix
        let patched = crate::fix::patch::apply_fix(source, d.fix.as_ref().unwrap());
        assert_eq!(
            patched.trim(),
            expected.trim(),
            "Patched output should match expected fixture.\nGot:\n{}\nExpected:\n{}",
            patched,
            expected
        );
    }

    #[test]
    fn test_fixture_not_in_scope() {
        let source = include_str!("../../tests/fixtures/fix/rust/not_in_scope.rs");
        let error_json = include_str!("../../tests/fixtures/fix/rust/not_in_scope.error.json");
        let expected = include_str!("../../tests/fixtures/fix/rust/not_in_scope.fixed.rs");

        let error_data: serde_json::Value = serde_json::from_str(error_json).unwrap();
        let error = ParsedError {
            error_type: error_data["code"].as_str().unwrap().to_string(),
            message: error_data["message"].as_str().unwrap().to_string(),
            file: Some(PathBuf::from("not_in_scope.rs")),
            line: error_data["line"].as_u64().map(|l| l as usize),
            column: error_data["col"].as_u64().map(|c| c as usize),
            language: "rust".to_string(),
            raw_text: error_json.to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0425(&error, source);
        assert!(diag.is_some(), "Should diagnose the not_in_scope fixture");
        let d = diag.unwrap();
        assert!(d.fix.is_some(), "Should produce a fix");

        // Apply the fix
        let patched = crate::fix::patch::apply_fix(source, d.fix.as_ref().unwrap());
        assert_eq!(
            patched.trim(),
            expected.trim(),
            "Patched output should match expected fixture.\nGot:\n{}\nExpected:\n{}",
            patched,
            expected
        );
    }

    // ---- E0433: Failed to resolve (type-position missing use) ----

    #[test]
    fn test_all_5_rust_analyzers_registered() {
        let error_codes = ["E0599", "E0277", "E0425", "E0308", "E0433"];
        for code in &error_codes {
            assert!(
                has_analyzer(code),
                "Analyzer for {} should be registered",
                code
            );
        }
    }

    #[test]
    fn test_rust_e0433_hashmap() {
        let source = "fn main() {\n    let m: HashMap<String, i32> = HashMap::new();\n}\n";
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `HashMap`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(2),
            column: Some(12),
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(diag.is_some(), "Should diagnose E0433 for HashMap");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "E0433");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0]
                .new_text
                .contains("use std::collections::HashMap;"),
            "Fix should inject HashMap import, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0433_arc() {
        let source = "fn main() {\n    let x = Arc::new(42);\n}\n";
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `Arc`".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(diag.is_some(), "Should diagnose E0433 for Arc");
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains("use std::sync::Arc;"));
    }

    #[test]
    fn test_rust_e0433_unknown_type() {
        let source = "fn main() {\n    let x = SomeRandomType::new();\n}\n";
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `SomeRandomType`".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(diag.is_some(), "Should return diagnosis for unknown type");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none(), "Unknown type should not produce a fix");
    }

    #[test]
    fn test_rust_e0433_already_imported() {
        let source =
            "use std::collections::HashMap;\n\nfn main() {\n    let m = HashMap::new();\n}\n";
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `HashMap`".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(
            diag.is_none() || diag.as_ref().unwrap().fix.is_none(),
            "Should not produce a fix when import already present"
        );
    }

    #[test]
    fn test_rust_e0433_with_existing_uses() {
        let source = "use std::fs::File;\n\nfn main() {\n    let m = HashMap::new();\n}\n";
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `HashMap`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert_eq!(
            fix.edits[0].kind,
            EditKind::InsertAfter,
            "Should insert after existing use statements"
        );
        assert!(fix.edits[0]
            .new_text
            .contains("use std::collections::HashMap;"));
    }

    #[test]
    fn test_diagnose_rust_dispatches_e0433() {
        let source = "fn main() {\n    let m = HashMap::new();\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Rust).unwrap();
        let error = ParsedError {
            error_type: "E0433".to_string(),
            message: "failed to resolve: use of undeclared type `HashMap`".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_rust(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "E0433");
    }

    // ---- E0308: Pattern 3 -- expected String, found &str (generic) ----

    #[test]
    fn test_rust_e0308_string_literal_to_string() {
        // Pattern 3: `let s: String = "hello";` -> `let s: String = "hello".to_string();`
        let source = "fn main() {\n    let s: String = \"hello\";\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `String`, found `&str`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some(), "Should diagnose string literal -> String");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some(), "Should produce a fix for string literal");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("\"hello\".to_string()"),
            "Fix should append .to_string() to string literal, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_variable_to_string() {
        // Pattern 3: `foo(name)` where expected String -> `foo(name.to_string())`
        let source = "fn main() {\n    let name = \"world\";\n    foo(name);\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `String`, found `&str`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some(), "Should diagnose variable -> String");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some(), "Should produce a fix for variable arg");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("name.to_string()"),
            "Fix should append .to_string() to variable, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_assignment_to_string() {
        // Pattern 3: `let s: String = greeting;` -> `let s: String = greeting.to_string();`
        let source = "fn main() {\n    let greeting = \"hi\";\n    let s: String = greeting;\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `String`, found `&str`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("greeting.to_string()"),
            "Fix should append .to_string() to assignment rhs, got: {}",
            fix.edits[0].new_text
        );
    }

    // ---- E0308: Pattern 4 -- expected &str, found String ----

    #[test]
    fn test_rust_e0308_string_to_str_ref() {
        // Pattern 4: `let s: &str = my_string;` -> `let s: &str = &my_string;`
        let source = "fn main() {\n    let my_string = String::from(\"hello\");\n    let s: &str = my_string;\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `&str`, found `String`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some(), "Should diagnose String -> &str");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some(), "Should produce a fix for String -> &str");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("&my_string"),
            "Fix should prepend & to rhs, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_string_to_str_fn_arg() {
        // Pattern 4: `takes_str(my_string)` -> `takes_str(&my_string)`
        let source = "fn main() {\n    let my_string = String::from(\"hello\");\n    takes_str(my_string);\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `&str`, found `String`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        // Should contain &my_string -- either via & prefix or .as_str()
        let text = &fix.edits[0].new_text;
        assert!(
            text.contains("&my_string") || text.contains("my_string.as_str()"),
            "Fix should borrow or .as_str() the String arg, got: {}",
            text
        );
    }

    // ---- E0308: Pattern 5 -- reference mismatch (expected T, found &T / expected &T, found T) ----

    #[test]
    fn test_rust_e0308_expected_i32_found_ref_i32() {
        // Pattern 5: `let x: i32 = &val;` -> `let x: i32 = *&val;` (or just `val`)
        // More realistically: `let x: i32 = some_ref;` -> `let x: i32 = *some_ref;`
        let source = "fn main() {\n    let val = 42;\n    let r = &val;\n    let x: i32 = r;\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `i32`, found `&i32`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some(), "Should diagnose &i32 -> i32");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some(), "Should produce a fix for ref mismatch");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("*r"),
            "Fix should dereference the reference, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_expected_ref_found_owned() {
        // Pattern 5: `let x: &i32 = val;` -> `let x: &i32 = &val;`
        let source = "fn main() {\n    let val: i32 = 42;\n    let x: &i32 = val;\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `&i32`, found `i32`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(3),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some(), "Should diagnose i32 -> &i32");
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some(), "Should produce a fix for missing ref");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("&val"),
            "Fix should add & to the value, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_rust_e0308_deref_in_fn_call() {
        // Pattern 5: `foo(some_ref)` where expected i32, found &i32 -> `foo(*some_ref)`
        let source = "fn main() {\n    let val = 42;\n    let r = &val;\n    foo(r);\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `i32`, found `&i32`".to_string(),
            file: Some(PathBuf::from("main.rs")),
            line: Some(4),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Medium);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("*r"),
            "Fix should dereference the reference in fn call, got: {}",
            fix.edits[0].new_text
        );
    }

    // ---- E0308: Existing patterns still work after broadening ----

    #[test]
    fn test_rust_e0308_cloned_still_works_after_broadening() {
        // Regression: existing .cloned() pattern must still be handled
        let source = "fn get_name(data: &[(&str, i32)]) -> Option<String> {\n    data.iter().find(|(k,_)| *k == \"name\").map(|(k,_)| k).cloned()\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `Option<String>`, found `Option<&&str>`"
                .to_string(),
            file: Some(PathBuf::from("lib.rs")),
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains(".map(|s| s.to_string())"));
    }

    #[test]
    fn test_rust_e0308_ok_or_still_works_after_broadening() {
        // Regression: existing .ok_or() pattern must still be handled
        let source = "fn lookup(map: &std::collections::HashMap<String, &str>, key: &str) -> Result<String, String> {\n    map.get(key).ok_or(\"not found\".to_string())\n}\n";
        let error = ParsedError {
            error_type: "E0308".to_string(),
            message: "mismatched types: expected `String`, found `&str`".to_string(),
            file: None,
            line: Some(2),
            column: None,
            language: "rust".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0308(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        assert!(d.fix.unwrap().edits[0]
            .new_text
            .contains(".map(|s| s.to_string()).ok_or("));
    }

    #[test]
    fn test_fixture_failed_resolve() {
        let source = include_str!("../../tests/fixtures/fix/rust/failed_resolve.rs");
        let error_json = include_str!("../../tests/fixtures/fix/rust/failed_resolve.error.json");
        let expected = include_str!("../../tests/fixtures/fix/rust/failed_resolve.fixed.rs");

        let error_data: serde_json::Value = serde_json::from_str(error_json).unwrap();
        let error = ParsedError {
            error_type: error_data["code"].as_str().unwrap().to_string(),
            message: error_data["message"].as_str().unwrap().to_string(),
            file: Some(PathBuf::from("failed_resolve.rs")),
            line: error_data["line"].as_u64().map(|l| l as usize),
            column: error_data["col"].as_u64().map(|c| c as usize),
            language: "rust".to_string(),
            raw_text: error_json.to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_e0433(&error, source);
        assert!(diag.is_some(), "Should diagnose the failed_resolve fixture");
        let d = diag.unwrap();
        assert!(d.fix.is_some(), "Should produce a fix");

        // Apply the fix
        let patched = crate::fix::patch::apply_fix(source, d.fix.as_ref().unwrap());
        assert_eq!(
            patched.trim(),
            expected.trim(),
            "Patched output should match expected fixture.\nGot:\n{}\nExpected:\n{}",
            patched,
            expected
        );
    }
}
