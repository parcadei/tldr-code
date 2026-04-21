//! Patch applicator -- applies `Fix` edits to source text.
//!
//! Takes source code as a string and a `Vec<TextEdit>`, and returns the
//! patched source string. Edits are applied in reverse line order to
//! preserve line numbers for subsequent edits.

use super::types::{EditKind, Fix, TextEdit};

/// Apply a set of text edits to source code.
///
/// The edits are sorted by line number in descending order before application,
/// so that earlier edits don't shift line numbers for later ones.
///
/// Returns the patched source string.
pub fn apply_fix(source: &str, fix: &Fix) -> String {
    apply_edits(source, &fix.edits)
}

/// Apply a vector of text edits to source code.
///
/// Edits are sorted by line number (descending) before application.
pub fn apply_edits(source: &str, edits: &[TextEdit]) -> String {
    if edits.is_empty() {
        return source.to_string();
    }

    let mut lines: Vec<String> = source.lines().map(|l| l.to_string()).collect();

    // If source ends with newline, preserve it
    let ends_with_newline = source.ends_with('\n');

    // Sort edits by line number descending so we apply from bottom to top
    let mut sorted_edits: Vec<&TextEdit> = edits.iter().collect();
    sorted_edits.sort_by(|a, b| b.line.cmp(&a.line));

    for edit in sorted_edits {
        // Line numbers are 1-indexed
        let idx = edit.line.saturating_sub(1);

        match &edit.kind {
            EditKind::InsertBefore => {
                if idx <= lines.len() {
                    lines.insert(idx, edit.new_text.clone());
                }
            }
            EditKind::InsertAfter => {
                let insert_at = (idx + 1).min(lines.len());
                lines.insert(insert_at, edit.new_text.clone());
            }
            EditKind::ReplaceLine => {
                if idx < lines.len() {
                    lines[idx] = edit.new_text.clone();
                }
            }
            EditKind::DeleteLine => {
                if idx < lines.len() {
                    lines.remove(idx);
                }
            }
            EditKind::ReplaceRange { start_col, end_col } => {
                if idx < lines.len() {
                    let line = &lines[idx];
                    let start = (*start_col).min(line.len());
                    let end = (*end_col).min(line.len());
                    let mut new_line = String::new();
                    new_line.push_str(&line[..start]);
                    new_line.push_str(&edit.new_text);
                    new_line.push_str(&line[end..]);
                    lines[idx] = new_line;
                }
            }
        }
    }

    let mut result = lines.join("\n");
    if ends_with_newline && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fix::types::{EditKind, Fix, TextEdit};

    #[test]
    fn test_insert_before() {
        let source = "line1\nline2\nline3\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 2,
                column: None,
                kind: EditKind::InsertBefore,
                new_text: "new_line".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "line1\nnew_line\nline2\nline3\n");
    }

    #[test]
    fn test_insert_after() {
        let source = "line1\nline2\nline3\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 1,
                column: None,
                kind: EditKind::InsertAfter,
                new_text: "new_line".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "line1\nnew_line\nline2\nline3\n");
    }

    #[test]
    fn test_replace_line() {
        let source = "line1\nline2\nline3\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 2,
                column: None,
                kind: EditKind::ReplaceLine,
                new_text: "replaced".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "line1\nreplaced\nline3\n");
    }

    #[test]
    fn test_replace_range() {
        let source = "hello world\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 1,
                column: None,
                kind: EditKind::ReplaceRange {
                    start_col: 6,
                    end_col: 11,
                },
                new_text: "rust".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "hello rust\n");
    }

    #[test]
    fn test_multiple_edits_reverse_order() {
        let source = "def inc():\n    counter += 1\n    return counter\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![
                TextEdit {
                    line: 1,
                    column: None,
                    kind: EditKind::InsertAfter,
                    new_text: "    global counter".to_string(),
                },
                TextEdit {
                    line: 3,
                    column: None,
                    kind: EditKind::InsertAfter,
                    new_text: "    # fixed".to_string(),
                },
            ],
        };
        let result = apply_fix(source, &fix);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "def inc():");
        assert_eq!(lines[1], "    global counter");
        assert_eq!(lines[2], "    counter += 1");
        assert_eq!(lines[3], "    return counter");
        assert_eq!(lines[4], "    # fixed");
    }

    #[test]
    fn test_empty_edits() {
        let source = "hello\n";
        let fix = Fix {
            description: "no-op".to_string(),
            edits: vec![],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "hello\n");
    }

    #[test]
    fn test_preserves_trailing_newline() {
        let source = "a\nb\n";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 1,
                column: None,
                kind: EditKind::ReplaceLine,
                new_text: "x".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "x\nb\n");
    }

    #[test]
    fn test_no_trailing_newline_preserved() {
        let source = "a\nb";
        let fix = Fix {
            description: "test".to_string(),
            edits: vec![TextEdit {
                line: 1,
                column: None,
                kind: EditKind::ReplaceLine,
                new_text: "x".to_string(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "x\nb");
    }

    #[test]
    fn test_delete_line() {
        let source = "line1\nline2\nline3\n";
        let fix = Fix {
            description: "test delete".to_string(),
            edits: vec![TextEdit {
                line: 2,
                column: None,
                kind: EditKind::DeleteLine,
                new_text: String::new(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "line1\nline3\n", "DeleteLine should remove the line entirely, not leave a blank");
    }

    #[test]
    fn test_delete_multiple_lines() {
        let source = "import (\n\t\"fmt\"\n)\nfunc main() {}\n";
        let fix = Fix {
            description: "delete import block".to_string(),
            edits: vec![
                TextEdit {
                    line: 1,
                    column: None,
                    kind: EditKind::DeleteLine,
                    new_text: String::new(),
                },
                TextEdit {
                    line: 2,
                    column: None,
                    kind: EditKind::DeleteLine,
                    new_text: String::new(),
                },
                TextEdit {
                    line: 3,
                    column: None,
                    kind: EditKind::DeleteLine,
                    new_text: String::new(),
                },
            ],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "func main() {}\n", "Deleting 3 lines should leave only the remaining line");
    }

    #[test]
    fn test_delete_first_line() {
        let source = "first\nsecond\nthird\n";
        let fix = Fix {
            description: "delete first".to_string(),
            edits: vec![TextEdit {
                line: 1,
                column: None,
                kind: EditKind::DeleteLine,
                new_text: String::new(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "second\nthird\n");
    }

    #[test]
    fn test_delete_last_line() {
        let source = "first\nsecond\nthird\n";
        let fix = Fix {
            description: "delete last".to_string(),
            edits: vec![TextEdit {
                line: 3,
                column: None,
                kind: EditKind::DeleteLine,
                new_text: String::new(),
            }],
        };
        let result = apply_fix(source, &fix);
        assert_eq!(result, "first\nsecond\n");
    }
}
