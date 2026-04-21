//! Go error analyzers -- 6 analyzers for `go build` / `go vet` output.
//!
//! Each analyzer is a pure function that takes a `ParsedError`, source code,
//! and a tree-sitter `Tree`, and returns an `Option<Diagnosis>`.
//!
//! # Analyzer Inventory (6 total)
//!
//! | # | Pattern                            | Analyzer              | Fix                                     | Confidence |
//! |---|------------------------------------|-----------------------|-----------------------------------------|------------|
//! | 1 | `undefined: X`                     | UndefinedAnalyzer     | Inject `import` for known packages      | HIGH       |
//! | 2 | `cannot use X as type Y`           | TypeMismatchAnalyzer  | Type conversion (string<->[]byte, etc.) | MEDIUM     |
//! | 3 | `X.Y undefined (type ... no ...)`  | FieldNotFoundAnalyzer | Check for typo / correct method name    | MEDIUM     |
//! | 4 | `imported and not used`            | UnusedImportAnalyzer  | Remove the unused import line           | HIGH       |
//! | 5 | `declared but not used`            | UnusedVarAnalyzer     | Prefix with `_`                         | HIGH       |
//! | 6 | `missing return`                   | MissingReturnAnalyzer | Add `return <zero_value>` at end of fn  | HIGH       |

use regex::Regex;
use tree_sitter::Tree;

use super::types::{Diagnosis, EditKind, Fix, FixConfidence, FixLocation, ParsedError, TextEdit};

// ============================================================================
// Known-fix lookup tables (data, not code)
// ============================================================================

/// Known Go standard library packages: maps a short name to its import path.
///
/// Used by the UndefinedAnalyzer to inject the correct import when a package
/// identifier is used without importing it.
static GO_KNOWN_IMPORTS: &[(&str, &str)] = &[
    ("fmt", "\"fmt\""),
    ("strings", "\"strings\""),
    ("strconv", "\"strconv\""),
    ("os", "\"os\""),
    ("io", "\"io\""),
    ("http", "\"net/http\""),
    ("json", "\"encoding/json\""),
    ("context", "\"context\""),
    ("sync", "\"sync\""),
    ("time", "\"time\""),
    ("filepath", "\"path/filepath\""),
    ("errors", "\"errors\""),
    ("log", "\"log\""),
    ("sort", "\"sort\""),
    ("math", "\"math\""),
    ("regexp", "\"regexp\""),
    ("bytes", "\"bytes\""),
    ("bufio", "\"bufio\""),
    ("ioutil", "\"io/ioutil\""),
    ("reflect", "\"reflect\""),
    ("testing", "\"testing\""),
    ("path", "\"path\""),
    ("runtime", "\"runtime\""),
    ("unicode", "\"unicode\""),
    ("net", "\"net\""),
];

/// Go zero values by type name, used by MissingReturnAnalyzer.
///
/// For types not found here, the default is `nil` (for pointers, slices,
/// maps, interfaces, channels, func types) or the zero-value literal.
static GO_ZERO_VALUES: &[(&str, &str)] = &[
    ("int", "0"),
    ("int8", "0"),
    ("int16", "0"),
    ("int32", "0"),
    ("int64", "0"),
    ("uint", "0"),
    ("uint8", "0"),
    ("uint16", "0"),
    ("uint32", "0"),
    ("uint64", "0"),
    ("uintptr", "0"),
    ("float32", "0.0"),
    ("float64", "0.0"),
    ("complex64", "0"),
    ("complex128", "0"),
    ("string", "\"\""),
    ("bool", "false"),
    ("error", "nil"),
    ("byte", "0"),
    ("rune", "0"),
];

/// Known Go type conversions: maps (source_type, target_type) to the
/// conversion expression template. The placeholder `{expr}` is replaced
/// with the actual expression text.
static GO_TYPE_CONVERSIONS: &[(&str, &str, &str)] = &[
    ("string", "[]byte", "[]byte({expr})"),
    ("[]byte", "string", "string({expr})"),
    ("int", "int64", "int64({expr})"),
    ("int64", "int", "int({expr})"),
    ("int", "int32", "int32({expr})"),
    ("int32", "int", "int({expr})"),
    ("float64", "int", "int({expr})"),
    ("int", "float64", "float64({expr})"),
    ("float32", "float64", "float64({expr})"),
    ("float64", "float32", "float32({expr})"),
    ("string", "[]rune", "[]rune({expr})"),
    ("[]rune", "string", "string({expr})"),
    ("int", "string", "strconv.Itoa({expr})"),
    ("string", "int", "strconv.Atoi({expr})"),
];

// ============================================================================
// Top-level dispatcher
// ============================================================================

/// Dispatch to the correct Go analyzer based on error pattern.
///
/// Returns `Some(Diagnosis)` if an analyzer handled the error, `None` otherwise.
pub fn diagnose_go(
    error: &ParsedError,
    source: &str,
    _tree: &Tree,
    _api_surface: Option<&()>,
) -> Option<Diagnosis> {
    let msg = &error.message;
    let full = format!("{} {}", msg, error.raw_text);

    // Pattern 1: "undefined: X"
    if msg.contains("undefined:") && !msg.contains("has no field or method") {
        return analyze_undefined(error, source);
    }

    // Pattern 2: "cannot use X as type Y"
    if full.contains("cannot use") && full.contains("as type") {
        return analyze_type_mismatch(error, source);
    }

    // Pattern 3: "X.Y undefined (type X has no field or method Y)"
    if full.contains("has no field or method") {
        return analyze_field_not_found(error, source);
    }

    // Pattern 4: "imported and not used" / "imported but not used"
    if full.contains("imported and not used") || full.contains("imported but not used") {
        return analyze_unused_import(error, source);
    }

    // Pattern 5: "declared but not used" / "declared and not used"
    if full.contains("declared but not used") || full.contains("declared and not used") {
        return analyze_unused_var(error, source);
    }

    // Pattern 6: "missing return"
    if full.contains("missing return") {
        return analyze_missing_return(error, source);
    }

    None
}

/// Check whether a given error pattern string has a registered Go analyzer.
pub fn has_analyzer(error_pattern: &str) -> bool {
    matches!(
        error_pattern,
        "undefined"
            | "type_mismatch"
            | "field_not_found"
            | "unused_import"
            | "unused_var"
            | "missing_return"
    )
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Find the line number of the `package` declaration (usually line 1).
/// Returns 1 if not found.
fn find_package_line(source: &str) -> usize {
    for (i, line) in source.lines().enumerate() {
        if line.trim().starts_with("package ") {
            return i + 1;
        }
    }
    1
}

/// Find the last import line number. Returns None if no imports exist.
fn find_last_import_line(source: &str) -> Option<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut last_import: Option<usize> = None;
    let mut in_import_block = false;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("import (") {
            in_import_block = true;
            last_import = Some(i + 1);
        } else if in_import_block {
            if trimmed == ")" {
                in_import_block = false;
                last_import = Some(i + 1);
            } else {
                last_import = Some(i + 1);
            }
        } else if trimmed.starts_with("import ") && !trimmed.starts_with("import (") {
            last_import = Some(i + 1);
        }
    }

    last_import
}

/// Find the closing paren line of an `import (...)` block that contains a
/// specific import path. Returns the 1-indexed line number of `)`.
fn find_import_block_close(source: &str) -> Option<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut in_block = false;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("import (") {
            in_block = true;
        } else if in_block && trimmed == ")" {
            return Some(i + 1);
        }
    }

    None
}

/// Check whether a given import path is already present in source.
fn has_import(source: &str, import_path: &str) -> bool {
    // Handles both `import "fmt"` and `import (\n\t"fmt"\n)` styles
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.contains(import_path) {
            // Check it's actually an import context (not just a string in code)
            if trimmed.starts_with("import ")
                || trimmed.starts_with("\"")
                || trimmed.starts_with("//")
            {
                return true;
            }
        }
    }
    false
}

// ============================================================================
// Analyzer 1: undefined -- missing import for known packages
// ============================================================================

/// Analyze `undefined: X` errors.
///
/// If X matches a known Go standard library package, injects the import.
/// Handles both single-import and import-block styles.
fn analyze_undefined(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let msg = &error.message;

    // Extract the undefined identifier: "undefined: fmt" -> "fmt"
    let re = Regex::new(r"undefined:\s*(\w+)").ok()?;
    let caps = re.captures(msg).or_else(|| re.captures(&error.raw_text))?;
    let name = caps.get(1)?.as_str();

    // Look up in known imports table
    let import_path = GO_KNOWN_IMPORTS
        .iter()
        .find(|(pkg, _)| *pkg == name)
        .map(|(_, path)| *path)?;

    // Already imported?
    if has_import(source, import_path) {
        return Some(Diagnosis {
            language: "go".to_string(),
            error_code: "undefined".to_string(),
            message: format!(
                "`{}` is already imported but still undefined -- may need manual fix",
                name
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

    // Determine where to insert the import
    let (insert_line, edit_kind, new_text) = if let Some(close_line) = find_import_block_close(source) {
        // There is an `import (...)` block -- insert before the closing `)`
        (
            close_line,
            EditKind::InsertBefore,
            format!("\t{}", import_path),
        )
    } else if find_last_import_line(source).is_some() {
        // There are single-line imports -- add another after the last one
        let last = find_last_import_line(source).unwrap();
        (
            last,
            EditKind::InsertAfter,
            format!("import {}", import_path),
        )
    } else {
        // No imports at all -- insert after package declaration
        let pkg_line = find_package_line(source);
        (
            pkg_line,
            EditKind::InsertAfter,
            format!("\nimport {}", import_path),
        )
    };

    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "undefined".to_string(),
        message: format!("`{}` is undefined -- add `import {}`", name, import_path),
        location: error.line.map(|l| FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: l,
            column: error.column,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Add `import {}` for package `{}`", import_path, name),
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
// Analyzer 2: type mismatch -- Go type conversions
// ============================================================================

/// Analyze `cannot use X as type Y` errors.
///
/// Looks up the (source_type, target_type) pair in GO_TYPE_CONVERSIONS and
/// suggests wrapping the expression with the correct conversion.
fn analyze_type_mismatch(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let full = format!("{} {}", error.message, error.raw_text);

    // Extract expression name, source type, and target type from the error message.
    //
    // Go error formats:
    //   "cannot use s (variable of type string) as type []byte in variable declaration"
    //   "cannot use x (variable of type int) as type int64 in variable declaration"
    //
    // Strategy: use two separate regex passes for robustness.

    // Pass 1: Extract the expression name
    let expr_re = Regex::new(r"cannot use (\w+)").ok()?;
    let expr_caps = expr_re.captures(&full)?;
    let expr_name = expr_caps.get(1)?.as_str();

    // Pass 2: Extract source type from "(variable of type <src>)" or "(type <src>)"
    let src_re = Regex::new(r"\((?:variable of )?type ([^)]+)\)").ok()?;
    let src_caps = src_re.captures(&full)?;
    let src_type_raw = src_caps.get(1)?.as_str().trim();

    // Pass 3: Extract target type from "as type <dst>" or "as <dst>"
    let dst_re = Regex::new(r"as (?:type )?(\S+)").ok()?;
    let dst_caps = dst_re.captures(&full)?;
    let dst_type_raw = dst_caps.get(1)?.as_str()
        .trim_end_matches([',', '.', ';']);

    // Normalize types for lookup (strip leading *, &, etc.)
    let src_type = src_type_raw.trim_start_matches('*');
    let dst_type = dst_type_raw.trim_start_matches('*');

    // Look up in GO_TYPE_CONVERSIONS table
    let conversion = GO_TYPE_CONVERSIONS
        .iter()
        .find(|(src, dst, _)| *src == src_type && *dst == dst_type);

    if let Some((_, _, template)) = conversion {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            if line_no > 0 && line_no <= lines.len() {
                let old_line = lines[line_no - 1];
                let converted = template.replace("{expr}", expr_name);

                // Replace the expression on the offending line
                if old_line.contains(expr_name) {
                    // Find the assignment: replace just the RHS expression
                    let new_line = old_line.replacen(expr_name, &converted, 1);

                    if new_line != old_line {
                        return Some(Diagnosis {
                            language: "go".to_string(),
                            error_code: "type_mismatch".to_string(),
                            message: format!(
                                "Cannot use `{}` ({}) as {} -- apply type conversion",
                                expr_name, src_type, dst_type
                            ),
                            location: Some(FixLocation {
                                file: error.file.clone().unwrap_or_default(),
                                line: line_no,
                                column: error.column,
                            }),
                            confidence: FixConfidence::Medium,
                            fix: Some(Fix {
                                description: format!(
                                    "Convert `{}` from `{}` to `{}` at line {}",
                                    expr_name, src_type, dst_type, line_no
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
    }

    // Fallback: unrecognized type pair
    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "type_mismatch".to_string(),
        message: format!(
            "Cannot use expression as type {} -- needs manual conversion",
            dst_type
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
// Analyzer 3: field/method not found
// ============================================================================

/// Analyze `X.Y undefined (type X has no field or method Y)` errors.
///
/// Attempts to suggest the correct method/field name based on simple
/// edit-distance heuristics for common typos.
fn analyze_field_not_found(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let full = format!("{} {}", error.message, error.raw_text);

    // Extract: "X.Y undefined (type X has no field or method Y)"
    let re = Regex::new(r"(\w+)\.(\w+) undefined.*has no field or method (\w+)").ok()?;
    let caps = re.captures(&full)?;

    let receiver = caps.get(1)?.as_str();
    let method = caps.get(3)?.as_str();

    // For common typos in known stdlib, suggest the correct name
    // This is a simple approach -- a full implementation would use edit distance
    // or the API surface to look up correct names
    let suggestion = suggest_correction(method);

    if let Some(correct_name) = suggestion {
        if let Some(line_no) = error.line {
            let lines: Vec<&str> = source.lines().collect();
            if line_no > 0 && line_no <= lines.len() {
                let old_line = lines[line_no - 1];
                let typo_pattern = format!("{}.{}", receiver, method);
                let fixed_pattern = format!("{}.{}", receiver, correct_name);

                if old_line.contains(&typo_pattern) {
                    let new_line = old_line.replace(&typo_pattern, &fixed_pattern);
                    return Some(Diagnosis {
                        language: "go".to_string(),
                        error_code: "field_not_found".to_string(),
                        message: format!(
                            "`{}.{}` does not exist -- did you mean `{}.{}`?",
                            receiver, method, receiver, correct_name
                        ),
                        location: Some(FixLocation {
                            file: error.file.clone().unwrap_or_default(),
                            line: line_no,
                            column: error.column,
                        }),
                        confidence: FixConfidence::Medium,
                        fix: Some(Fix {
                            description: format!(
                                "Replace `{}.{}` with `{}.{}` at line {}",
                                receiver, method, receiver, correct_name, line_no
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

    // Fallback: no suggestion
    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "field_not_found".to_string(),
        message: format!(
            "`{}.{}` does not exist on type `{}` -- check API surface or docs",
            receiver, method, receiver
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

/// Suggest a corrected method/field name for common typos.
///
/// Uses a table of known common misspellings. For a production system,
/// this would use edit distance against the API surface.
fn suggest_correction(misspelled: &str) -> Option<&'static str> {
    // Common typos in Go standard library methods
    static CORRECTIONS: &[(&str, &str)] = &[
        ("Contians", "Contains"),
        ("Contins", "Contains"),
        ("Conatins", "Contains"),
        ("Containss", "Contains"),
        ("Repalce", "Replace"),
        ("Reaplce", "Replace"),
        ("Replacee", "Replace"),
        ("Sprintff", "Sprintf"),
        ("Spritf", "Sprintf"),
        ("Pirntln", "Println"),
        ("Pritnln", "Println"),
        ("Printlm", "Println"),
        ("Prnitf", "Printf"),
        ("HasPerfix", "HasPrefix"),
        ("HasPrifix", "HasPrefix"),
        ("HasSufix", "HasSuffix"),
        ("HasSuffx", "HasSuffix"),
        ("TrimSapce", "TrimSpace"),
        ("TrimSpce", "TrimSpace"),
        ("ToLoewr", "ToLower"),
        ("ToUpeer", "ToUpper"),
        ("Joim", "Join"),
        ("Spilt", "Split"),
        ("Spllit", "Split"),
        ("SplitN", "SplitN"),
        ("Atou", "Atoi"),
        ("Itao", "Itoa"),
        ("NewReeader", "NewReader"),
        ("NewWritter", "NewWriter"),
        ("Marshel", "Marshal"),
        ("Unmarshl", "Unmarshal"),
        ("Unmarshel", "Unmarshal"),
    ];

    CORRECTIONS
        .iter()
        .find(|(typo, _)| *typo == misspelled)
        .map(|(_, correct)| *correct)
}

// ============================================================================
// Analyzer 4: unused import -- remove the import line
// ============================================================================

/// Analyze `"X" imported and not used` errors.
///
/// Removes the unused import line. Handles both single-import and import-block
/// styles. When removing from an import block, also removes the blank line if
/// it would leave the block empty.
fn analyze_unused_import(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let full = format!("{} {}", error.message, error.raw_text);

    // Extract the unused package path: `"os" imported and not used` or
    // `"os" imported but not used` (Go compilers use both phrasings)
    let re = Regex::new(r#""([^"]+)"\s+imported (?:and|but) not used"#).ok()?;
    let caps = re.captures(&full)?;
    let import_path = caps.get(1)?.as_str();

    let lines: Vec<&str> = source.lines().collect();

    // Find the line that contains this import
    let mut import_line: Option<usize> = None;
    let mut in_import_block = false;
    let mut block_start: Option<usize> = None;
    let mut block_end: Option<usize> = None;
    let mut block_item_count = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("import (") {
            in_import_block = true;
            block_start = Some(i);
            block_item_count = 0;
        } else if in_import_block {
            if trimmed == ")" {
                block_end = Some(i);
                in_import_block = false;
            } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
                block_item_count += 1;
                if trimmed.contains(&format!("\"{}\"", import_path)) {
                    import_line = Some(i + 1); // 1-indexed
                }
            }
        } else if trimmed.starts_with("import ")
            && trimmed.contains(&format!("\"{}\"", import_path))
        {
            import_line = Some(i + 1); // 1-indexed
        }
    }

    let import_line = import_line?;

    // Build the edit -- use DeleteLine to remove lines entirely (not leave blanks)
    let edits = if let (1, Some(bs), Some(be)) = (block_item_count, block_start, block_end) {
        // Only one import in the block -- remove the entire import (...) block
        let start = bs + 1; // 1-indexed
        let end = be + 1;
        // Delete each line from start to end
        (start..=end)
            .map(|line_no| TextEdit {
                line: line_no,
                column: None,
                kind: EditKind::DeleteLine,
                new_text: String::new(),
            })
            .collect()
    } else {
        // Multiple imports in block, or single-line import -- just delete this line
        vec![TextEdit {
            line: import_line,
            column: None,
            kind: EditKind::DeleteLine,
            new_text: String::new(),
        }]
    };

    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "unused_import".to_string(),
        message: format!("`{}` imported but not used -- removing import", import_path),
        location: Some(FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: import_line,
            column: None,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Remove unused import `\"{}\"`", import_path),
            edits,
        }),
    })
}

// ============================================================================
// Analyzer 5: unused variable -- prefix with _
// ============================================================================

/// Analyze `X declared but not used` errors.
///
/// Replaces the variable name with `_` on the declaring line. Handles both
/// `:=` short declarations and `var` declarations.
fn analyze_unused_var(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let full = format!("{} {}", error.message, error.raw_text);

    // Extract variable name: "x declared but not used"
    let re = Regex::new(r"(\w+) declared (?:and|but) not used").ok()?;
    let caps = re.captures(&full)?;
    let var_name = caps.get(1)?.as_str();

    let line_no = error.line?;
    let lines: Vec<&str> = source.lines().collect();
    if line_no == 0 || line_no > lines.len() {
        return None;
    }
    let old_line = lines[line_no - 1];

    // Replace the variable with _ in the declaration
    let new_line = if old_line.contains(":=") {
        // Short declaration: `x := expr` -> `_ = expr`
        // Handle multi-assignment: `x, y := f()` -- only replace the unused var
        let decl_re = Regex::new(&format!(r"\b{}\b", regex::escape(var_name))).ok()?;
        let replaced = decl_re.replace(old_line, "_").to_string();

        // `_ := expr` is invalid Go ("no new variables on left side of :=").
        // When the only variable on the left side is `_`, change `:=` to `=`.
        // Multi-var (`_, y := f()`) keeps `:=` because `y` is new.
        // for-range (`for _ := range ...`) also keeps `:=` (valid syntax).
        if replaced.trim_start().starts_with("for ") {
            replaced
        } else if let Some(lhs) = replaced.split(":=").next() {
            let all_blank = lhs
                .split(',')
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .all(|v| v == "_");
            if all_blank {
                replaced.replacen(":=", "=", 1)
            } else {
                replaced
            }
        } else {
            replaced
        }
    } else if old_line.contains("var ") {
        // Var declaration: `var x int = expr` -> `var _ int = expr`
        let decl_re = Regex::new(&format!(r"\bvar\s+{}\b", regex::escape(var_name))).ok()?;
        decl_re.replace(old_line, "var _").to_string()
    } else {
        // Range/for var: `for x := range items` -> `for _ := range items`
        let decl_re = Regex::new(&format!(r"\b{}\b", regex::escape(var_name))).ok()?;
        decl_re.replace(old_line, "_").to_string()
    };

    if new_line == old_line {
        return None;
    }

    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "unused_var".to_string(),
        message: format!("`{}` declared but not used -- replacing with `_`", var_name),
        location: Some(FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: line_no,
            column: error.column,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!("Replace `{}` with `_` at line {}", var_name, line_no),
            edits: vec![TextEdit {
                line: line_no,
                column: None,
                kind: EditKind::ReplaceLine,
                new_text: new_line,
            }],
        }),
    })
}

// ============================================================================
// Analyzer 6: missing return -- inject zero-value return
// ============================================================================

/// Analyze `missing return at end of function` errors.
///
/// Parses the function signature from source at the error line to determine
/// the return type, then injects `return <zero_value>` before the closing
/// brace of the function body.
fn analyze_missing_return(error: &ParsedError, source: &str) -> Option<Diagnosis> {
    let line_no = error.line?;
    let lines: Vec<&str> = source.lines().collect();
    if line_no == 0 || line_no > lines.len() {
        return None;
    }

    // The error line points to the function declaration. Find the return type.
    let func_line = lines[line_no - 1];
    let return_type = extract_go_return_type(func_line)?;
    let zero_value = go_zero_value(&return_type);

    // Find the closing brace of this function
    let closing_brace = find_function_closing_brace(source, line_no);
    let closing_brace_line = closing_brace?;

    // Determine indentation from the function body
    let indent = if closing_brace_line >= 2 && closing_brace_line <= lines.len() {
        let prev_line = lines[closing_brace_line - 2];
        let leading: String = prev_line.chars().take_while(|c| c.is_whitespace()).collect();
        leading
    } else {
        "\t".to_string()
    };

    let return_stmt = format!("{}return {}", indent, zero_value);

    Some(Diagnosis {
        language: "go".to_string(),
        error_code: "missing_return".to_string(),
        message: format!(
            "Missing return at end of function -- inserting `return {}`",
            zero_value
        ),
        location: Some(FixLocation {
            file: error.file.clone().unwrap_or_default(),
            line: closing_brace_line,
            column: None,
        }),
        confidence: FixConfidence::High,
        fix: Some(Fix {
            description: format!(
                "Add `return {}` before closing brace at line {}",
                zero_value, closing_brace_line
            ),
            edits: vec![TextEdit {
                line: closing_brace_line,
                column: None,
                kind: EditKind::InsertBefore,
                new_text: return_stmt,
            }],
        }),
    })
}

/// Extract the return type from a Go function signature line.
///
/// Handles:
/// - `func add(a int, b int) int {` -> `int`
/// - `func f() (string, error) {` -> `string, error`
/// - `func f() error {` -> `error`
fn extract_go_return_type(func_line: &str) -> Option<String> {
    let trimmed = func_line.trim();
    if !trimmed.starts_with("func ") {
        return None;
    }

    // Find the closing paren of the parameter list
    let mut paren_depth = 0;
    let mut param_end = None;
    for (i, ch) in trimmed.char_indices() {
        if ch == '(' {
            paren_depth += 1;
        } else if ch == ')' {
            paren_depth -= 1;
            if paren_depth == 0 {
                param_end = Some(i);
                break;
            }
        }
    }

    let param_end = param_end?;
    let after_params = trimmed[param_end + 1..].trim();

    // Strip trailing `{`
    let after_params = after_params.trim_end_matches('{').trim();

    if after_params.is_empty() {
        return None; // No return type
    }

    // Handle multi-return: (string, error)
    if after_params.starts_with('(') && after_params.ends_with(')') {
        let inner = &after_params[1..after_params.len() - 1];
        return Some(inner.to_string());
    }

    Some(after_params.to_string())
}

/// Look up the zero value for a Go type.
fn go_zero_value(type_str: &str) -> String {
    // Handle multi-return types
    if type_str.contains(',') {
        let parts: Vec<&str> = type_str.split(',').map(|s| s.trim()).collect();
        let zeros: Vec<String> = parts
            .iter()
            .map(|t| go_zero_value_single(t.trim()))
            .collect();
        return zeros.join(", ");
    }

    go_zero_value_single(type_str)
}

/// Look up the zero value for a single Go type.
fn go_zero_value_single(type_str: &str) -> String {
    let clean = type_str.trim();

    // Check lookup table first
    for (t, zero) in GO_ZERO_VALUES {
        if *t == clean {
            return zero.to_string();
        }
    }

    // Pointer, slice, map, interface, func, channel -> nil
    if clean.starts_with('*')
        || clean.starts_with("[]")
        || clean.starts_with("map[")
        || clean.starts_with("func")
        || clean.starts_with("chan ")
        || clean.starts_with("<-chan")
        || clean == "interface{}"
        || clean == "any"
    {
        return "nil".to_string();
    }

    // Named types with known package prefixes
    // e.g., "http.Handler" -> nil (interface), "time.Duration" -> 0
    if clean.contains('.') {
        // Assume it's a struct/interface type -> nil
        return "nil".to_string();
    }

    // Struct types -> the struct literal
    if clean.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        return format!("{}{{}}",  clean);
    }

    // Default fallback
    "nil".to_string()
}

/// Find the closing brace of a function starting at the given line.
/// Returns the 1-indexed line number of `}`.
fn find_function_closing_brace(source: &str, func_line: usize) -> Option<usize> {
    let lines: Vec<&str> = source.lines().collect();
    if func_line == 0 || func_line > lines.len() {
        return None;
    }

    let mut brace_depth = 0;
    let mut found_open = false;

    for (i, line) in lines[func_line - 1..].iter().enumerate() {
        for ch in line.chars() {
            if ch == '{' {
                brace_depth += 1;
                found_open = true;
            } else if ch == '}' {
                brace_depth -= 1;
                if found_open && brace_depth == 0 {
                    return Some(func_line + i);
                }
            }
        }
    }

    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ---- Validation gate: all 6 analyzers registered ----

    #[test]
    fn test_all_6_go_analyzers_registered() {
        let patterns = [
            "undefined",
            "type_mismatch",
            "field_not_found",
            "unused_import",
            "unused_var",
            "missing_return",
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
    fn test_unknown_pattern_not_handled() {
        assert!(!has_analyzer("some_random_pattern"));
        assert!(!has_analyzer(""));
        assert!(!has_analyzer("syntax_error"));
    }

    // ---- Analyzer 1: undefined (missing import) ----

    #[test]
    fn test_go_undefined_fmt() {
        let source = "package main\n\nfunc main() {\n\ts := fmt.Sprintf(\"hello %s\", \"world\")\n\tprintln(s)\n}\n";
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: fmt".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(7),
            language: "go".to_string(),
            raw_text: "./main.go:4:7: undefined: fmt".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_undefined(&error, source);
        assert!(diag.is_some(), "Should diagnose undefined fmt");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "undefined");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("\"fmt\""),
            "Fix should inject fmt import, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_undefined_json() {
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tdata, _ := json.Marshal(map[string]int{\"a\": 1})\n\tfmt.Println(string(data))\n}\n";
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: json".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(6),
            column: Some(14),
            language: "go".to_string(),
            raw_text: "./main.go:6:14: undefined: json".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_undefined(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("\"encoding/json\""),
            "Fix should inject encoding/json import, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_undefined_already_imported() {
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: fmt".to_string(),
            file: None,
            line: Some(6),
            column: None,
            language: "go".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_undefined(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none(), "Should not produce a fix when already imported");
    }

    #[test]
    fn test_go_undefined_unknown_package() {
        let source = "package main\n\nfunc main() {\n\tresult := mypkg.DoSomething()\n}\n";
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: mypkg".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:4:12: undefined: mypkg".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_undefined(&error, source);
        assert!(
            diag.is_none(),
            "Unknown packages should return None (not in GO_KNOWN_IMPORTS)"
        );
    }

    #[test]
    fn test_go_undefined_into_import_block() {
        let source = "package main\n\nimport (\n\t\"os\"\n)\n\nfunc main() {\n\tfmt.Println(os.Getenv(\"HOME\"))\n}\n";
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: fmt".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(8),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:8:2: undefined: fmt".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_undefined(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        // Should insert before the closing `)` of the import block
        assert!(
            fix.edits[0].new_text.contains("\"fmt\""),
            "Fix should add fmt to import block, got: {}",
            fix.edits[0].new_text
        );
        assert_eq!(fix.edits[0].kind, EditKind::InsertBefore);
    }

    // ---- Analyzer 2: type mismatch ----

    #[test]
    fn test_go_type_mismatch_string_to_bytes() {
        let source = "package main\n\nfunc main() {\n\tvar s string = \"hello\"\n\tvar b []byte = s\n\tprintln(b)\n}\n";
        let error = ParsedError {
            error_type: "type_mismatch".to_string(),
            message: "cannot use s (variable of type string) as type []byte in variable declaration".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(5),
            column: Some(17),
            language: "go".to_string(),
            raw_text: "./main.go:5:17: cannot use s (variable of type string) as type []byte in variable declaration".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_mismatch(&error, source);
        assert!(diag.is_some(), "Should diagnose type mismatch string->[]byte");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "type_mismatch");
        assert!(d.fix.is_some(), "Should have a fix");
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("[]byte(s)"),
            "Fix should wrap s in []byte(), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_type_mismatch_int_to_int64() {
        let source = "package main\n\nfunc main() {\n\tvar x int = 42\n\tvar y int64 = x\n\tprintln(y)\n}\n";
        let error = ParsedError {
            error_type: "type_mismatch".to_string(),
            message: "cannot use x (variable of type int) as type int64 in variable declaration".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(5),
            column: Some(16),
            language: "go".to_string(),
            raw_text: "./main.go:5:16: cannot use x (variable of type int) as type int64 in variable declaration".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_mismatch(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("int64(x)"),
            "Fix should wrap x in int64(), got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_type_mismatch_unknown_types() {
        let source = "package main\n\nfunc main() {\n\tvar x MyType = other\n}\n";
        let error = ParsedError {
            error_type: "type_mismatch".to_string(),
            message: "cannot use other (variable of type OtherType) as type MyType".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_type_mismatch(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none(), "Unknown type pair should not produce a fix");
    }

    // ---- Analyzer 3: field not found ----

    #[test]
    fn test_go_field_not_found_typo() {
        let source = "package main\n\nimport \"strings\"\n\nfunc main() {\n\ts := \"hello world\"\n\tresult := strings.Contians(s, \"hello\")\n\tprintln(result)\n}\n";
        let error = ParsedError {
            error_type: "field_not_found".to_string(),
            message: "strings.Contians undefined (type strings has no field or method Contians)".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(7),
            column: Some(13),
            language: "go".to_string(),
            raw_text: "./main.go:7:13: strings.Contians undefined (type strings has no field or method Contians)".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_field_not_found(&error, source);
        assert!(diag.is_some(), "Should diagnose field not found for Contians");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "field_not_found");
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("strings.Contains"),
            "Fix should correct to Contains, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_field_not_found_no_suggestion() {
        let source = "package main\n\nfunc main() {\n\tx := myStruct.DoZzz()\n}\n";
        let error = ParsedError {
            error_type: "field_not_found".to_string(),
            message: "myStruct.DoZzz undefined (type myStruct has no field or method DoZzz)".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_field_not_found(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.confidence, FixConfidence::Low);
        assert!(d.fix.is_none(), "Unknown method should not produce a fix");
    }

    // ---- Analyzer 4: unused import ----

    #[test]
    fn test_go_unused_import_in_block() {
        let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"os\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(5),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:5:2: \"os\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some(), "Should diagnose unused import");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "unused_import");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        // Should delete the "os" line entirely
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].kind, EditKind::DeleteLine);
        assert!(fix.edits[0].new_text.is_empty(), "Should delete the line");
    }

    #[test]
    fn test_go_unused_import_single_line() {
        let source = "package main\n\nimport \"os\"\n\nfunc main() {\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"os\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"os\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert_eq!(fix.edits[0].line, 3);
        assert!(fix.edits[0].new_text.is_empty());
    }

    #[test]
    fn test_go_unused_import_sole_in_block() {
        let source = "package main\n\nimport (\n\t\"os\"\n)\n\nfunc main() {\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"os\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:4:2: \"os\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        // Should remove the entire import block (3 lines)
        assert_eq!(
            fix.edits.len(),
            3,
            "Should remove all 3 lines of import block"
        );
    }

    // ---- Analyzer 5: unused variable ----

    #[test]
    fn test_go_unused_var_short_decl() {
        let source = "package main\n\nfunc main() {\n\tx := 42\n\ty := \"hello\"\n\tprintln(y)\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(diag.is_some(), "Should diagnose unused var");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "unused_var");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("_ ="),
            "Fix should replace x with _ and use `=` (not `:=`), got: {}",
            fix.edits[0].new_text
        );
        assert!(
            !fix.edits[0].new_text.contains("_ :="),
            "Fix must NOT use `:=` with blank identifier, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_unused_var_declaration() {
        let source = "package main\n\nfunc main() {\n\tvar count int\n\tprintln(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "count declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(6),
            language: "go".to_string(),
            raw_text: "./main.go:4:6: count declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("var _ int") || fix.edits[0].new_text.contains("var _"),
            "Fix should replace var count with var _, got: {}",
            fix.edits[0].new_text
        );
    }

    // ---- Analyzer 6: missing return ----

    #[test]
    fn test_go_missing_return_int() {
        let source = "package main\n\nfunc add(a int, b int) int {\n\tsum := a + b\n\tfmt.Println(sum)\n}\n\nfunc main() {\n\tadd(1, 2)\n}\n";
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(1),
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_missing_return(&error, source);
        assert!(diag.is_some(), "Should diagnose missing return");
        let d = diag.unwrap();
        assert_eq!(d.error_code, "missing_return");
        assert_eq!(d.confidence, FixConfidence::High);
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("return 0"),
            "Fix should add return 0 for int return type, got: {}",
            fix.edits[0].new_text
        );
        assert_eq!(fix.edits[0].kind, EditKind::InsertBefore);
    }

    #[test]
    fn test_go_missing_return_string() {
        let source = "package main\n\nfunc greet(name string) string {\n\tresult := \"Hello, \" + name\n\tfmt.Println(result)\n}\n";
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(1),
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_missing_return(&error, source);
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert!(d.fix.is_some());
        let fix = d.fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("return \"\""),
            "Fix should add return \"\" for string return type, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_missing_return_error() {
        let source = "package main\n\nfunc doWork() error {\n\tfmt.Println(\"working\")\n}\n";
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(1),
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_missing_return(&error, source);
        assert!(diag.is_some());
        let fix = diag.unwrap().fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("return nil"),
            "Fix should add return nil for error return type, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_missing_return_multi_return() {
        let source = "package main\n\nfunc divide(a, b int) (int, error) {\n\tif b == 0 {\n\t\treturn 0, fmt.Errorf(\"division by zero\")\n\t}\n}\n";
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(1),
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_missing_return(&error, source);
        assert!(diag.is_some());
        let fix = diag.unwrap().fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("return 0, nil"),
            "Fix should add return 0, nil for (int, error) return type, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_missing_return_bool() {
        let source = "package main\n\nfunc isValid(s string) bool {\n\tfmt.Println(s)\n}\n";
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(1),
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_missing_return(&error, source);
        assert!(diag.is_some());
        let fix = diag.unwrap().fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("return false"),
            "Fix should add return false for bool return type, got: {}",
            fix.edits[0].new_text
        );
    }

    // ---- Helper tests ----

    #[test]
    fn test_extract_go_return_type_simple() {
        assert_eq!(
            extract_go_return_type("func add(a int, b int) int {"),
            Some("int".to_string())
        );
        assert_eq!(
            extract_go_return_type("func greet(name string) string {"),
            Some("string".to_string())
        );
        assert_eq!(
            extract_go_return_type("func doWork() error {"),
            Some("error".to_string())
        );
    }

    #[test]
    fn test_extract_go_return_type_multi() {
        assert_eq!(
            extract_go_return_type("func divide(a, b int) (int, error) {"),
            Some("int, error".to_string())
        );
        assert_eq!(
            extract_go_return_type("func f() (string, bool, error) {"),
            Some("string, bool, error".to_string())
        );
    }

    #[test]
    fn test_extract_go_return_type_no_return() {
        assert_eq!(extract_go_return_type("func main() {"), None);
        assert_eq!(extract_go_return_type("func doStuff(x int) {"), None);
    }

    #[test]
    fn test_extract_go_return_type_not_func() {
        assert_eq!(extract_go_return_type("var x int = 42"), None);
        assert_eq!(extract_go_return_type("type Foo struct {"), None);
    }

    #[test]
    fn test_go_zero_value_single() {
        assert_eq!(go_zero_value_single("int"), "0");
        assert_eq!(go_zero_value_single("string"), "\"\"");
        assert_eq!(go_zero_value_single("bool"), "false");
        assert_eq!(go_zero_value_single("error"), "nil");
        assert_eq!(go_zero_value_single("float64"), "0.0");
    }

    #[test]
    fn test_go_zero_value_pointer_types() {
        assert_eq!(go_zero_value_single("*Foo"), "nil");
        assert_eq!(go_zero_value_single("[]byte"), "nil");
        assert_eq!(go_zero_value_single("map[string]int"), "nil");
        assert_eq!(go_zero_value_single("interface{}"), "nil");
        assert_eq!(go_zero_value_single("func()"), "nil");
        assert_eq!(go_zero_value_single("chan int"), "nil");
    }

    #[test]
    fn test_go_zero_value_multi() {
        assert_eq!(go_zero_value("int, error"), "0, nil");
        assert_eq!(go_zero_value("string, bool, error"), "\"\", false, nil");
    }

    #[test]
    fn test_suggest_correction() {
        assert_eq!(suggest_correction("Contians"), Some("Contains"));
        assert_eq!(suggest_correction("Repalce"), Some("Replace"));
        assert_eq!(suggest_correction("Pirntln"), Some("Println"));
        assert_eq!(suggest_correction("Contains"), None); // Correct already
        assert_eq!(suggest_correction("FooBar"), None);
    }

    #[test]
    fn test_find_package_line() {
        assert_eq!(find_package_line("package main\n\nfunc main() {}\n"), 1);
        assert_eq!(
            find_package_line("// Comment\npackage utils\n\nfunc f() {}\n"),
            2
        );
    }

    #[test]
    fn test_find_import_block_close() {
        let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {}\n";
        assert_eq!(find_import_block_close(source), Some(6));
    }

    #[test]
    fn test_has_import() {
        let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
        assert!(has_import(source, "\"fmt\""));
        assert!(has_import(source, "\"os\""));
        assert!(!has_import(source, "\"strings\""));
    }

    #[test]
    fn test_find_function_closing_brace() {
        let source = "package main\n\nfunc add(a, b int) int {\n\treturn a + b\n}\n\nfunc main() {\n}\n";
        assert_eq!(find_function_closing_brace(source, 3), Some(5));
        assert_eq!(find_function_closing_brace(source, 7), Some(8));
    }

    // ---- Dispatcher tests ----

    #[test]
    fn test_diagnose_go_dispatches_undefined() {
        let source = "package main\n\nfunc main() {\n\tfmt.Println(\"hello\")\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "undefined".to_string(),
            message: "undefined: fmt".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:4:2: undefined: fmt".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "undefined");
    }

    #[test]
    fn test_diagnose_go_dispatches_unused_var() {
        let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: None,
            line: Some(4),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:4:2: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "unused_var");
    }

    #[test]
    fn test_diagnose_go_dispatches_missing_return() {
        let source = "package main\n\nfunc f() int {\n\tfmt.Println(42)\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "missing_return".to_string(),
            message: "missing return at end of function".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "go".to_string(),
            raw_text: "./main.go:3:1: missing return at end of function".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(diag.is_some());
        assert_eq!(diag.unwrap().error_code, "missing_return");
    }

    #[test]
    fn test_diagnose_go_returns_none_for_unknown() {
        let source = "package main\n\nfunc main() {}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "unknown".to_string(),
            message: "some unhandled error pattern".to_string(),
            file: None,
            line: Some(3),
            column: None,
            language: "go".to_string(),
            raw_text: String::new(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(diag.is_none(), "Should return None for unknown error patterns");
    }

    // ---- Bug fix: unused_var short decl must produce `_ = expr` not `_ := expr` ----

    #[test]
    fn test_go_unused_var_short_decl_produces_valid_go() {
        // `_ := 42` is invalid Go ("no new variables on left side of :=").
        // The fix must produce `_ = 42` (plain assignment, no colon).
        let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(diag.is_some(), "Should diagnose unused var");
        let fix = diag.unwrap().fix.unwrap();
        let new_text = &fix.edits[0].new_text;
        assert!(
            !new_text.contains("_ :="),
            "Fix must NOT produce `_ :=` (invalid Go), got: {}",
            new_text
        );
        assert!(
            new_text.contains("_ = "),
            "Fix must produce `_ = expr` (valid Go), got: {}",
            new_text
        );
    }

    #[test]
    fn test_go_unused_var_short_decl_multi_var_keeps_walrus() {
        // `x, y := f()` with x unused -> `_, y := f()` (walrus is valid here
        // because y is still a new variable)
        let source = "package main\n\nfunc f() (int, int) { return 1, 2 }\n\nfunc main() {\n\tx, y := f()\n\tprintln(y)\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(6),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:6:2: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(diag.is_some(), "Should diagnose unused var in multi-assign");
        let fix = diag.unwrap().fix.unwrap();
        let new_text = &fix.edits[0].new_text;
        assert!(
            new_text.contains("_, y :="),
            "Multi-var decl should keep := (y is new), got: {}",
            new_text
        );
    }

    #[test]
    fn test_go_unused_var_for_range_keeps_walrus() {
        // `for x := range items` with x unused -> `for _ := range items`
        // This is valid because := in for-range is syntactic, not a variable declaration.
        let source = "package main\n\nfunc main() {\n\titems := []int{1, 2, 3}\n\tfor x := range items {\n\t\tprintln(\"loop\")\n\t}\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(5),
            column: Some(6),
            language: "go".to_string(),
            raw_text: "./main.go:5:6: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(diag.is_some(), "Should diagnose unused for-range var");
        let fix = diag.unwrap().fix.unwrap();
        let new_text = &fix.edits[0].new_text;
        // for-range with := is fine because it is a range clause, not a short var decl
        assert!(
            new_text.contains("_ :=") || new_text.contains("_ ="),
            "for-range fix should replace var name, got: {}",
            new_text
        );
    }

    // ---- Bug fix: unused_import should use DeleteLine not ReplaceLine ----

    #[test]
    fn test_go_unused_import_single_line_deletes_not_blanks() {
        // Removing `import "fmt"` should delete the line entirely,
        // not leave a blank line.
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tprintln(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(8),
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"fmt\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some(), "Should diagnose unused import");
        let fix = diag.unwrap().fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(
            fix.edits[0].kind,
            EditKind::DeleteLine,
            "Single-line import removal should use DeleteLine, not ReplaceLine with empty string"
        );
    }

    #[test]
    fn test_go_unused_import_in_block_deletes_line() {
        // Removing one import from `import (\n\t"fmt"\n\t"os"\n)` should
        // delete just the "fmt" line, not replace it with blank.
        let source = "package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nfunc main() {\n\tos.Exit(0)\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: \"fmt\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some(), "Should diagnose unused import in block");
        let fix = diag.unwrap().fix.unwrap();
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(
            fix.edits[0].kind,
            EditKind::DeleteLine,
            "Import line removal from block should use DeleteLine"
        );
    }

    #[test]
    fn test_go_unused_import_sole_in_block_deletes_whole_block() {
        // Removing the only import from `import (\n\t"fmt"\n)` should
        // delete all 3 lines of the block.
        let source = "package main\n\nimport (\n\t\"fmt\"\n)\n\nfunc main() {\n\tprintln(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: \"fmt\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(diag.is_some(), "Should diagnose sole import in block");
        let fix = diag.unwrap().fix.unwrap();
        // All 3 lines (import (, "fmt", )) should be deleted
        assert_eq!(fix.edits.len(), 3, "Should delete all 3 lines of import block");
        for edit in &fix.edits {
            assert_eq!(
                edit.kind,
                EditKind::DeleteLine,
                "All block deletion edits should use DeleteLine, not ReplaceLine"
            );
        }
    }

    // ================================================================
    // Bug fix: alternate phrasings "declared AND not used" and
    //          "imported BUT not used" must be handled by both the
    //          dispatcher (diagnose_go) and the individual analyzers.
    // ================================================================

    // ---- "declared and not used" (alternate Go compiler phrasing) ----

    #[test]
    fn test_go_unused_var_declared_and_not_used_via_analyzer() {
        // Go compilers can emit "x declared and not used" instead of
        // "x declared but not used". The analyzer's regex already handles
        // (?:and|but), so this tests the analyzer directly.
        let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: x declared and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_var(&error, source);
        assert!(
            diag.is_some(),
            "Analyzer must handle 'declared and not used' phrasing"
        );
        let fix = diag.unwrap().fix.unwrap();
        assert!(
            fix.edits[0].new_text.contains("_ ="),
            "Fix should replace x with _, got: {}",
            fix.edits[0].new_text
        );
    }

    #[test]
    fn test_go_unused_var_declared_and_not_used_via_dispatcher() {
        // The dispatcher must route "declared and not used" to the
        // unused var analyzer, not fall through and return None.
        let source = "package main\n\nfunc main() {\n\tx := 42\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(4),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:4:2: x declared and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(
            diag.is_some(),
            "Dispatcher must handle 'declared and not used' -- not fall through to None"
        );
        let d = diag.unwrap();
        assert_eq!(
            d.error_code, "unused_var",
            "Error code should be unused_var, got: {}",
            d.error_code
        );
        assert!(d.fix.is_some(), "Should produce a fix");
    }

    // ---- "imported but not used" (alternate Go compiler phrasing) ----

    #[test]
    fn test_go_unused_import_imported_but_not_used_via_analyzer() {
        // Go compilers can emit '"fmt" imported but not used' instead of
        // '"fmt" imported and not used'. The analyzer must handle both.
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tprintln(\"hello\")\n}\n";
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(8),
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"fmt\" imported but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = analyze_unused_import(&error, source);
        assert!(
            diag.is_some(),
            "Analyzer must handle 'imported but not used' phrasing"
        );
        let fix = diag.unwrap().fix.unwrap();
        assert_eq!(fix.edits[0].kind, EditKind::DeleteLine);
    }

    #[test]
    fn test_go_unused_import_imported_but_not_used_via_dispatcher() {
        // The dispatcher must route "imported but not used" to the
        // unused import analyzer, not fall through and return None.
        let source = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tprintln(\"hello\")\n}\n";
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();
        let error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(8),
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"fmt\" imported but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let diag = diagnose_go(&error, source, &tree, None);
        assert!(
            diag.is_some(),
            "Dispatcher must handle 'imported but not used' -- not fall through to None"
        );
        let d = diag.unwrap();
        assert_eq!(
            d.error_code, "unused_import",
            "Error code should be unused_import, got: {}",
            d.error_code
        );
        assert!(d.fix.is_some(), "Should produce a fix");
    }

    // ---- E2E test: Go source with both unused var and unused import ----

    #[test]
    fn test_go_e2e_both_unused_var_and_unused_import() {
        // Simulates a Go source file that would produce both errors:
        //   ./main.go:3:8: "fmt" imported and not used
        //   ./main.go:6:2: x declared and not used
        let source = concat!(
            "package main\n",
            "\n",
            "import \"fmt\"\n",
            "\n",
            "func main() {\n",
            "\tx := 42\n",
            "}\n",
        );
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();

        // Error 1: unused import (using "and" phrasing)
        let import_error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(8),
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"fmt\" imported and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        // Error 2: unused var (using "and" phrasing)
        let var_error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared and not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(6),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:6:2: x declared and not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        // Both must produce fixes via the dispatcher
        let import_diag = diagnose_go(&import_error, source, &tree, None);
        assert!(import_diag.is_some(), "Import error must produce a diagnosis");
        let id = import_diag.unwrap();
        assert_eq!(id.error_code, "unused_import");
        assert!(id.fix.is_some(), "Import fix must exist");
        let import_fix = id.fix.unwrap();
        assert_eq!(import_fix.edits[0].kind, EditKind::DeleteLine);

        let var_diag = diagnose_go(&var_error, source, &tree, None);
        assert!(var_diag.is_some(), "Var error must produce a diagnosis");
        let vd = var_diag.unwrap();
        assert_eq!(vd.error_code, "unused_var");
        assert!(vd.fix.is_some(), "Var fix must exist");
        let var_fix = vd.fix.unwrap();
        assert_eq!(var_fix.edits[0].kind, EditKind::ReplaceLine);
        assert!(
            var_fix.edits[0].new_text.contains("_ ="),
            "Should replace x with _, got: {}",
            var_fix.edits[0].new_text
        );
    }

    // ---- E2E test: same as above but with "but" phrasings ----

    #[test]
    fn test_go_e2e_both_unused_with_but_phrasing() {
        let source = concat!(
            "package main\n",
            "\n",
            "import \"fmt\"\n",
            "\n",
            "func main() {\n",
            "\tx := 42\n",
            "}\n",
        );
        let tree = crate::ast::parser::parse(source, crate::Language::Go).unwrap();

        // "imported but not used" phrasing
        let import_error = ParsedError {
            error_type: "unused_import".to_string(),
            message: "\"fmt\" imported but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(3),
            column: Some(8),
            language: "go".to_string(),
            raw_text: "./main.go:3:8: \"fmt\" imported but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        // "declared but not used" phrasing
        let var_error = ParsedError {
            error_type: "unused_var".to_string(),
            message: "x declared but not used".to_string(),
            file: Some(PathBuf::from("./main.go")),
            line: Some(6),
            column: Some(2),
            language: "go".to_string(),
            raw_text: "./main.go:6:2: x declared but not used".to_string(),
            function_name: None,
            offending_line: None,
        };

        let import_diag = diagnose_go(&import_error, source, &tree, None);
        assert!(import_diag.is_some(), "Import error with 'but' phrasing must produce a diagnosis");
        assert_eq!(import_diag.as_ref().unwrap().error_code, "unused_import");
        assert!(import_diag.unwrap().fix.is_some());

        let var_diag = diagnose_go(&var_error, source, &tree, None);
        assert!(var_diag.is_some(), "Var error with 'but' phrasing must produce a diagnosis");
        assert_eq!(var_diag.as_ref().unwrap().error_code, "unused_var");
        assert!(var_diag.unwrap().fix.is_some());
    }
}
