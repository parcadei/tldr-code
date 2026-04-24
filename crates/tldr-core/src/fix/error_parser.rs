//! Error text parser -- converts raw error output into `ParsedError`.
//!
//! Supports:
//! - Python tracebacks (`Traceback (most recent call last):`)
//! - Language auto-detection from error format (spec section 9.7)
//!
//! The parser extracts the error type, message, file, line, function name,
//! and offending source line from the raw text.

use std::path::PathBuf;

use regex::Regex;

use super::types::ParsedError;

// ---------------------------------------------------------------------------
// pytest output pre-processing
// ---------------------------------------------------------------------------

/// Strip pytest decoration lines from raw output.
///
/// Removes:
/// - `====...====` separator lines (pytest section headers/footers)
/// - `____...____` test name separator lines
/// - `FAILED file::test - ErrorType: message` summary lines
/// - `N failed, M passed in X.XXs` timing lines
/// - `-- Captured stdout --` / `-- Captured stderr --` section headers
/// - Progress bar lines (`test_foo.py::test_bar PASSED  [XX%]`)
/// - Session info lines (`platform ...`, `rootdir: ...`, `collected N items`)
///
/// Preserves the actual traceback and error lines.
fn strip_pytest_decoration(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    let mut cleaned: Vec<&str> = Vec::new();
    let mut in_captured_section = false;

    // Pre-compile regexes outside the loop to avoid re-creation per line
    let timing_re = Regex::new(r"^\d+\s+(failed|passed|error)").unwrap();
    let progress_re = Regex::new(r"::\w+\s+(PASSED|FAILED|ERROR|SKIPPED)\s+\[").unwrap();

    for line in &lines {
        let trimmed = line.trim();

        // Skip empty lines (they'll be preserved around traceback content)
        if trimmed.is_empty() {
            // End captured section on blank line
            if in_captured_section {
                in_captured_section = false;
            }
            cleaned.push(line);
            continue;
        }

        // Skip if we're inside a "-- Captured stdout/stderr --" section
        if in_captured_section {
            continue;
        }

        // Skip === separator lines (e.g., "===== FAILURES =====")
        if trimmed.starts_with("===") && trimmed.ends_with("===") {
            continue;
        }
        // Skip === lines that are just separators without text
        if trimmed.chars().all(|c| c == '=') && trimmed.len() > 3 {
            continue;
        }

        // Skip ___ test name separator lines (e.g., "_______ test_foo _______")
        if trimmed.starts_with("___") && trimmed.ends_with("___") {
            continue;
        }
        if trimmed.chars().all(|c| c == '_') && trimmed.len() > 3 {
            continue;
        }

        // Skip FAILED summary lines
        if trimmed.starts_with("FAILED ") && trimmed.contains(" - ") {
            continue;
        }

        // Skip timing summary lines (e.g., "1 failed in 0.12s", "1 failed, 2 passed in 0.45s")
        if timing_re.is_match(trimmed) {
            continue;
        }

        // Skip "-- Captured stdout --" / "-- Captured stderr --" section headers
        // and mark that we're in a captured section
        if trimmed.starts_with("-- Captured ") && trimmed.ends_with(" --") {
            in_captured_section = true;
            continue;
        }

        // Skip pytest session header lines
        if trimmed.starts_with("platform ") && trimmed.contains("pytest") {
            continue;
        }
        if trimmed.starts_with("rootdir:") {
            continue;
        }
        if trimmed.starts_with("collected ") && trimmed.contains("item") {
            continue;
        }

        // Skip progress bar lines (e.g., "test_app.py::test_alpha PASSED   [ 33%]")
        if progress_re.is_match(trimmed) {
            continue;
        }

        // Skip pytest header line (e.g., "===== test session starts =====")
        // Already handled by === detection above

        cleaned.push(line);
    }

    cleaned.join("\n")
}

/// Parse a pytest summary line into a `ParsedError`.
///
/// Handles the format:
/// `FAILED test_app.py::test_foo - NameError: name 'x' is not defined`
fn parse_pytest_summary_line(raw: &str) -> Option<ParsedError> {
    let summary_re =
        Regex::new(r"FAILED\s+([^:]+)::(\w+)\s+-\s+([A-Z]\w*(?:Error|Exception|Warning)):\s*(.*)")
            .ok()?;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(caps) = summary_re.captures(trimmed) {
            let file = caps.get(1).unwrap().as_str();
            let _test_name = caps.get(2).unwrap().as_str();
            let error_type = caps.get(3).unwrap().as_str();
            let message = caps.get(4).unwrap().as_str();

            return Some(ParsedError {
                error_type: error_type.to_string(),
                message: message.to_string(),
                file: Some(PathBuf::from(file)),
                line: None,
                column: None,
                language: "python".to_string(),
                raw_text: raw.to_string(),
                function_name: None,
                offending_line: None,
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Python traceback parser
// ---------------------------------------------------------------------------

/// Parse a Python traceback or error message into a `ParsedError`.
///
/// Handles three formats:
/// 1. Full traceback with `Traceback (most recent call last):` header
///    (including when wrapped in pytest verbose output)
/// 2. Pytest summary line: `FAILED file::test - ErrorType: message`
/// 3. Single-line error: `ErrorType: message`
pub fn parse_python_error(raw: &str) -> Option<ParsedError> {
    // Pre-process: strip pytest decoration if present, then try full traceback
    let cleaned = strip_pytest_decoration(raw);
    if let Some(parsed) = parse_python_traceback(&cleaned) {
        return Some(parsed);
    }

    // Try pytest summary line fallback
    if let Some(parsed) = parse_pytest_summary_line(raw) {
        return Some(parsed);
    }

    // Fall back to single-line error
    parse_python_single_line(raw)
}

/// Parse a full Python traceback.
///
/// Format:
/// ```text
/// Traceback (most recent call last):
///   File "app.py", line 10, in some_function
///     counter += 1
/// UnboundLocalError: cannot access local variable 'counter'
/// ```
fn parse_python_traceback(raw: &str) -> Option<ParsedError> {
    if !raw.contains("Traceback (most recent call last)") {
        return None;
    }

    let lines: Vec<&str> = raw.lines().collect();

    // Extract file, line, function from the LAST `File "...", line N, in func` entry
    let file_line_re = Regex::new(r#"^\s*File "([^"]+)", line (\d+)(?:, in (\w+))?"#).ok()?;

    let mut file: Option<PathBuf> = None;
    let mut line_num: Option<usize> = None;
    let mut func_name: Option<String> = None;
    let mut offending_line: Option<String> = None;
    let mut last_file_idx: Option<usize> = None;

    for (idx, text) in lines.iter().enumerate() {
        if let Some(caps) = file_line_re.captures(text) {
            file = Some(PathBuf::from(caps.get(1).unwrap().as_str()));
            line_num = caps.get(2).and_then(|m| m.as_str().parse().ok());
            func_name = caps.get(3).map(|m| m.as_str().to_string());
            last_file_idx = Some(idx);
        }
    }

    // The offending line is the line immediately after the last File reference
    if let Some(idx) = last_file_idx {
        if idx + 1 < lines.len() {
            let candidate = lines[idx + 1].trim();
            // Skip if it looks like another File line or the error line
            if !candidate.starts_with("File ")
                && !candidate.is_empty()
                && !candidate.contains("Traceback")
            {
                offending_line = Some(candidate.to_string());
            }
        }
    }

    // The error line is the last non-empty line that matches `ErrorType: message`
    let error_line_re =
        Regex::new(r"^([A-Z]\w*(?:Error|Exception|Iteration|Warning))\s*:\s*(.*)$").ok()?;
    // Also handle bare error types like "KeyError: 'name'" (compiled outside the loop)
    let key_error_re = Regex::new(r"^(KeyError)\s*:\s*(.*)$").ok()?;

    let mut error_type = String::new();
    let mut message = String::new();

    for text in lines.iter().rev() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(caps) = error_line_re.captures(trimmed) {
            error_type = caps.get(1).unwrap().as_str().to_string();
            message = caps.get(2).unwrap().as_str().to_string();
            break;
        }
        if let Some(caps) = key_error_re.captures(trimmed) {
            error_type = caps.get(1).unwrap().as_str().to_string();
            message = caps.get(2).unwrap().as_str().to_string();
            break;
        }
        // StopIteration has no suffix "Error"
        if trimmed == "StopIteration" {
            error_type = "StopIteration".to_string();
            break;
        }
        break;
    }

    if error_type.is_empty() {
        // Try inference from message patterns
        error_type = extract_error_type(raw);
        if error_type == "Unknown" {
            return None;
        }
        message = raw.to_string();
    }

    Some(ParsedError {
        error_type,
        message,
        file,
        line: line_num,
        column: None,
        language: "python".to_string(),
        raw_text: raw.to_string(),
        function_name: func_name,
        offending_line,
    })
}

/// Parse a single-line Python error: `ErrorType: message`
fn parse_python_single_line(raw: &str) -> Option<ParsedError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let error_type = extract_error_type(trimmed);
    if error_type == "Unknown" {
        return None;
    }

    // Extract message: everything after "ErrorType: "
    let message = if let Some(idx) = trimmed.find(": ") {
        trimmed[idx + 2..].to_string()
    } else {
        trimmed.to_string()
    };

    // Try to extract file/line if present in the message
    let file_re = Regex::new(r#"File "([^"]+)", line (\d+)"#).ok()?;
    let (file, line) = if let Some(caps) = file_re.captures(raw) {
        (
            Some(PathBuf::from(caps.get(1).unwrap().as_str())),
            caps.get(2).and_then(|m| m.as_str().parse().ok()),
        )
    } else {
        (None, None)
    };

    Some(ParsedError {
        error_type,
        message,
        file,
        line,
        column: None,
        language: "python".to_string(),
        raw_text: raw.to_string(),
        function_name: None,
        offending_line: None,
    })
}

// ---------------------------------------------------------------------------
// Language auto-detection
// ---------------------------------------------------------------------------

/// Auto-detect language from error text format.
///
/// Returns "python", "rust", "typescript", "go", "javascript", or "unknown".
///
/// Detection order matters -- JavaScript is checked before the generic Python
/// `XError:` pattern because JS runtime errors (ReferenceError, TypeError) would
/// otherwise match Python. Rust is detected via both JSON format (`"code":"E0..."`)
/// and rendered text format (`error[E0425]: ...`).
pub fn detect_language(error_text: &str) -> &'static str {
    // Python: Traceback header is unambiguous
    if error_text.contains("Traceback (most recent call last)") {
        return "python";
    }

    // JavaScript (Node.js): must be checked BEFORE the generic Python `XError:` pattern
    // since JS runtime errors (ReferenceError, TypeError, SyntaxError) also match that regex.
    // JS detection is based on Node.js stack trace patterns or .js file references.
    if error_text.contains("at Object.<anonymous>")
        || error_text.contains("at Module._compile")
        || error_text.contains("at Module._extensions")
        || error_text.contains("at node:internal")
        || Regex::new(r"[\w./]+\.js:\d+")
            .map(|re| re.is_match(error_text))
            .unwrap_or(false)
    {
        return "javascript";
    }

    // JavaScript (Node.js): TypeError patterns that are unique to the V8/Node.js runtime
    // and NEVER appear in Python. Must be checked before the generic `XError:` Python pattern.
    //
    // Patterns:
    // - "Cannot read properties of undefined (reading 'X')"
    // - "Cannot read properties of null (reading 'X')"
    // - "Cannot set properties of undefined (setting 'X')"
    // - "Cannot set properties of null (setting 'X')"
    // - "X is not a function" (when preceded by TypeError:)
    if error_text.contains("Cannot read properties of")
        || error_text.contains("Cannot set properties of")
        || Regex::new(r"TypeError:\s+.+\s+is not a function")
            .map(|re| re.is_match(error_text))
            .unwrap_or(false)
    {
        return "javascript";
    }

    // Python: known error types (after JS check to avoid false positives)
    if Regex::new(r"^[A-Z]\w*Error:\s")
        .map(|re| re.is_match(error_text.trim()))
        .unwrap_or(false)
    {
        return "python";
    }

    // Rust: JSON with "code" field starting with E
    if error_text.contains(r#""code""#)
        && (error_text.contains(r#""E0"#) || error_text.contains(r#""rendered""#))
    {
        return "rust";
    }

    // Rust: rendered text format — `error[E0425]: message`
    if Regex::new(r"error\[[A-Z]\d+\]:")
        .map(|re| re.is_match(error_text))
        .unwrap_or(false)
    {
        return "rust";
    }

    // TypeScript: file.ts(line,col): error TS
    if Regex::new(r"\w+\.tsx?\(\d+,\d+\):\s*error\s+TS\d+")
        .map(|re| re.is_match(error_text))
        .unwrap_or(false)
    {
        return "typescript";
    }

    // TypeScript simplified: bare `error TS2304:` without file prefix
    if Regex::new(r"error\s+TS\d+:")
        .map(|re| re.is_match(error_text))
        .unwrap_or(false)
    {
        return "typescript";
    }

    // Go: file.go:line:col: pattern (matches ./main.go:10:5: or ./pkg/handler.go:10:5:)
    if Regex::new(r"[\w./]+\.go:\d+:\d+:")
        .map(|re| re.is_match(error_text))
        .unwrap_or(false)
    {
        return "go";
    }

    // Go simplified: bare Go diagnostic keywords without file:line:col prefix.
    // These patterns are unique to Go compiler output and do not collide with
    // Python/JS/Rust/TS error formats.
    if error_text.contains("undefined:")
        && !error_text.contains("is not defined")
        && !error_text.contains("has no field or method")
    {
        // "undefined: X" is Go; "is not defined" is JS/Python; "has no field or method" uses
        // the field_not_found path below
        return "go";
    }
    if error_text.contains("declared but not used") || error_text.contains("declared and not used")
    {
        return "go";
    }
    if error_text.contains("imported and not used") || error_text.contains("imported but not used")
    {
        return "go";
    }
    if error_text.contains("missing return at end of function") {
        return "go";
    }
    if error_text.contains("has no field or method") {
        return "go";
    }
    if error_text.contains("cannot use") && error_text.contains("as type") {
        return "go";
    }

    "unknown"
}

/// Parse error text with language auto-detection.
///
/// If `lang` is provided, uses the specified language parser.
/// Otherwise, auto-detects from the error format.
pub fn parse_error(raw: &str, lang: Option<&str>) -> Option<ParsedError> {
    let detected = lang.unwrap_or_else(|| detect_language(raw));
    match detected {
        "python" => parse_python_error(raw),
        "rust" => parse_rustc_error(raw),
        "typescript" => parse_tsc_error(raw),
        "go" => parse_go_error(raw),
        "javascript" => parse_js_error(raw),
        _ => {
            // Try Python parser as fallback -- it handles single-line errors
            parse_python_error(raw)
        }
    }
}

// ---------------------------------------------------------------------------
// JavaScript (Node.js) error parser
// ---------------------------------------------------------------------------

/// Parse a Node.js runtime error into a `ParsedError`.
///
/// Handles the Node.js error format:
/// ```text
/// /path/to/file.js:10
///     undefinedVar.foo()
///     ^
/// ReferenceError: undefinedVar is not defined
///     at Object.<anonymous> (/path/to/file.js:10:1)
///     at Module._compile (node:internal/modules/cjs/loader:1234:14)
/// ```
///
/// Also handles single-line runtime errors like:
/// `TypeError: Cannot read properties of undefined (reading 'foo')`
pub fn parse_js_error(raw: &str) -> Option<ParsedError> {
    let lines: Vec<&str> = raw.lines().collect();

    // Strategy 1: Look for the error type line (ReferenceError, TypeError, SyntaxError)
    let error_line_re = Regex::new(
        r"^(ReferenceError|TypeError|SyntaxError|RangeError|URIError|EvalError):\s*(.+)$",
    )
    .ok()?;

    let mut error_type = String::new();
    let mut message = String::new();
    let mut file: Option<PathBuf> = None;
    let mut line_num: Option<usize> = None;
    let mut column: Option<usize> = None;
    let mut offending_line: Option<String> = None;
    let mut function_name: Option<String> = None;

    // Find the error type line
    for text in &lines {
        let trimmed = text.trim();
        if let Some(caps) = error_line_re.captures(trimmed) {
            error_type = caps.get(1).unwrap().as_str().to_string();
            message = caps.get(2).unwrap().as_str().to_string();
            break;
        }
    }

    if error_type.is_empty() {
        return None;
    }

    // Strategy 2: Extract file/line from the header line: `/path/to/file.js:10`
    let header_re = Regex::new(r"^([^\s:]+\.(?:js|mjs|cjs)):(\d+)$").ok()?;
    for text in &lines {
        let trimmed = text.trim();
        if let Some(caps) = header_re.captures(trimmed) {
            file = Some(PathBuf::from(caps.get(1).unwrap().as_str()));
            line_num = caps.get(2).and_then(|m| m.as_str().parse().ok());
            break;
        }
    }

    // Strategy 3: If no header line found, extract from the stack trace
    // `at Object.<anonymous> (/path/to/file.js:10:1)`
    if file.is_none() {
        let stack_re =
            Regex::new(r"at\s+(?:([^\s(]+)\s+)?\(?([^)(\s]+\.(?:js|mjs|cjs)):(\d+):(\d+)\)?")
                .ok()?;
        for text in &lines {
            let trimmed = text.trim();
            if let Some(caps) = stack_re.captures(trimmed) {
                function_name = caps.get(1).map(|m| m.as_str().to_string());
                file = Some(PathBuf::from(caps.get(2).unwrap().as_str()));
                line_num = caps.get(3).and_then(|m| m.as_str().parse().ok());
                column = caps.get(4).and_then(|m| m.as_str().parse().ok());
                break;
            }
        }
    }

    // Strategy 4: Extract the offending source line (line after the header, before `^`)
    if let Some(header_idx) = lines.iter().position(|l| header_re.is_match(l.trim())) {
        if header_idx + 1 < lines.len() {
            let candidate = lines[header_idx + 1].trim();
            if !candidate.is_empty() && !candidate.starts_with('^') && !candidate.starts_with("at ")
            {
                offending_line = Some(candidate.to_string());
            }
        }
    }

    Some(ParsedError {
        error_type,
        message,
        file,
        line: line_num,
        column,
        language: "javascript".to_string(),
        raw_text: raw.to_string(),
        function_name,
        offending_line,
    })
}

// ---------------------------------------------------------------------------
// Rust error parser (rustc --error-format=json)
// ---------------------------------------------------------------------------

/// Parse a Rust compiler error from JSON output.
///
/// Handles two formats:
/// 1. Direct error JSON: `{"code":"E0599","message":"...",...}`
/// 2. Cargo JSON output: `{"reason":"compiler-message","message":{...}}`
///
/// Also handles the rendered text format as a fallback.
pub fn parse_rustc_error(raw: &str) -> Option<ParsedError> {
    // Try parsing as JSON first
    if let Some(parsed) = parse_rustc_json(raw) {
        return Some(parsed);
    }

    // Try line-by-line for cargo's multi-JSON output
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(parsed) = parse_rustc_json(trimmed) {
            return Some(parsed);
        }
    }

    // Fallback: try to parse rendered text format
    parse_rustc_rendered(raw)
}

/// Parse a single JSON object from rustc output.
fn parse_rustc_json(json_str: &str) -> Option<ParsedError> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;

    // Handle cargo JSON wrapper: {"reason":"compiler-message","message":{...}}
    let msg = if value.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
        value.get("message")?.clone()
    } else if value.get("code").is_some() || value.get("message").is_some() {
        // Direct error object
        value
    } else {
        return None;
    };

    // Only handle errors (not warnings)
    if let Some(level) = msg.get("level").and_then(|l| l.as_str()) {
        if level != "error" {
            return None;
        }
    }

    // Extract error code
    let error_code = msg
        .get("code")
        .and_then(|c| {
            // Code can be a string or an object with a "code" field
            if c.is_string() {
                c.as_str().map(|s| s.to_string())
            } else {
                c.get("code")
                    .and_then(|cc| cc.as_str())
                    .map(|s| s.to_string())
            }
        })
        .unwrap_or_default();

    let message = msg
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    // Extract primary span
    let spans = msg.get("spans").and_then(|s| s.as_array());
    let primary_span = spans.and_then(|spans| {
        spans
            .iter()
            .find(|s| s.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
            .or_else(|| spans.first())
    });

    let file = primary_span
        .and_then(|s| s.get("file_name"))
        .and_then(|f| f.as_str())
        .map(PathBuf::from);

    let line = primary_span
        .and_then(|s| s.get("line_start"))
        .and_then(|l| l.as_u64())
        .map(|l| l as usize);

    let column = primary_span
        .and_then(|s| s.get("column_start"))
        .and_then(|c| c.as_u64())
        .map(|c| c as usize);

    let offending_line = primary_span
        .and_then(|s| s.get("text"))
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|t| t.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());

    // Build the raw_text including children for hint extraction
    let raw_text = json_str.to_string();

    Some(ParsedError {
        error_type: error_code,
        message,
        file,
        line,
        column,
        language: "rust".to_string(),
        raw_text,
        function_name: None,
        offending_line,
    })
}

/// Parse rendered rustc error text (non-JSON format).
///
/// Format:
/// ```text
/// error[E0599]: no method named `read_line` found for struct `Stdin`
///   --> src/main.rs:3:21
/// ```
fn parse_rustc_rendered(raw: &str) -> Option<ParsedError> {
    let error_re = Regex::new(r"error\[([A-Z]\d+)\]:\s*(.+)").ok()?;

    let caps = error_re.captures(raw)?;
    let error_code = caps.get(1).unwrap().as_str().to_string();
    let message = caps.get(2).unwrap().as_str().to_string();

    // Extract location: --> file.rs:line:col
    let loc_re = Regex::new(r"-->\s+([^:]+):(\d+):(\d+)").ok()?;
    let (file, line, column) = if let Some(loc_caps) = loc_re.captures(raw) {
        (
            Some(PathBuf::from(loc_caps.get(1).unwrap().as_str())),
            loc_caps.get(2).and_then(|m| m.as_str().parse().ok()),
            loc_caps.get(3).and_then(|m| m.as_str().parse().ok()),
        )
    } else {
        (None, None, None)
    };

    Some(ParsedError {
        error_type: error_code,
        message,
        file,
        line,
        column,
        language: "rust".to_string(),
        raw_text: raw.to_string(),
        function_name: None,
        offending_line: None,
    })
}

// ---------------------------------------------------------------------------
// TypeScript (tsc) error parser
// ---------------------------------------------------------------------------

/// Parse a TypeScript compiler error from tsc output.
///
/// Handles the standard tsc output format:
/// `file.ts(line,col): error TS2304: Cannot find name 'foo'.`
///
/// Also handles multi-line tsc output by extracting the first error line.
pub fn parse_tsc_error(raw: &str) -> Option<ParsedError> {
    // Try each line for a tsc error pattern
    let tsc_re = Regex::new(r"([^\s(]+\.tsx?)\((\d+),(\d+)\):\s*error\s+(TS\d+):\s*(.*)").ok()?;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(caps) = tsc_re.captures(trimmed) {
            let file = caps.get(1).unwrap().as_str();
            let line_no: usize = caps.get(2).unwrap().as_str().parse().ok()?;
            let col: usize = caps.get(3).unwrap().as_str().parse().ok()?;
            let error_code = caps.get(4).unwrap().as_str();
            let message = caps.get(5).unwrap().as_str().trim_end_matches('.');

            return Some(ParsedError {
                error_type: error_code.to_string(),
                message: message.to_string(),
                file: Some(PathBuf::from(file)),
                line: Some(line_no),
                column: Some(col),
                language: "typescript".to_string(),
                raw_text: raw.to_string(),
                function_name: None,
                offending_line: None,
            });
        }
    }

    // Fallback: bare `error TS2304: message` without file(line,col) prefix.
    // Matches simplified / hand-typed / truncated tsc output.
    let tsc_fallback_re = Regex::new(r"error\s+(TS\d+):\s*(.+)").ok()?;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(caps) = tsc_fallback_re.captures(trimmed) {
            let error_code = caps.get(1).unwrap().as_str();
            let message = caps.get(2).unwrap().as_str().trim_end_matches('.');

            return Some(ParsedError {
                error_type: error_code.to_string(),
                message: message.to_string(),
                file: None,
                line: None,
                column: None,
                language: "typescript".to_string(),
                raw_text: raw.to_string(),
                function_name: None,
                offending_line: None,
            });
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Go error parser (go build / go vet output)
// ---------------------------------------------------------------------------

/// Parse a Go compiler/vet error from `go build` or `go vet` output.
///
/// Handles the standard Go error format:
/// `./main.go:10:5: undefined: foo`
///
/// Also detects Go-specific error patterns to classify the error type.
pub fn parse_go_error(raw: &str) -> Option<ParsedError> {
    // Go error line pattern: file.go:line:col: message
    let go_re = Regex::new(r"([^\s:]+\.go):(\d+):(\d+):\s*(.+)").ok()?;

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(caps) = go_re.captures(trimmed) {
            let file = caps.get(1).unwrap().as_str();
            let line_no: usize = caps.get(2).unwrap().as_str().parse().ok()?;
            let col: usize = caps.get(3).unwrap().as_str().parse().ok()?;
            let message = caps.get(4).unwrap().as_str();

            // Classify the error type based on message content
            let error_type = classify_go_error(message);

            return Some(ParsedError {
                error_type,
                message: message.to_string(),
                file: Some(PathBuf::from(file)),
                line: Some(line_no),
                column: Some(col),
                language: "go".to_string(),
                raw_text: raw.to_string(),
                function_name: None,
                offending_line: None,
            });
        }
    }

    // Fallback: bare Go error messages without file:line:col prefix.
    // Matches simplified / hand-typed / truncated go build output.
    // Try classify_go_error on each line; if it returns something other
    // than "go_error" (the catch-all), the line is a recognizable Go diagnostic.
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let error_type = classify_go_error(trimmed);
        if error_type != "go_error" {
            return Some(ParsedError {
                error_type,
                message: trimmed.to_string(),
                file: None,
                line: None,
                column: None,
                language: "go".to_string(),
                raw_text: raw.to_string(),
                function_name: None,
                offending_line: None,
            });
        }
    }

    None
}

/// Classify a Go error message into a canonical error type.
///
/// Maps Go error message patterns to the analyzer pattern names used
/// by the Go fix module.
///
/// Handles two alternate Go compiler phrasings for unused variables:
/// - `x declared but not used`  (older gc format)
/// - `declared and not used: x` (alternate gc format)
///
/// Handles multiple alternate phrasings for unused imports:
/// - `"os" imported and not used`
/// - `"os" imported but not used`
/// - `imported and not used: "os"` (alternate gc format)
/// - `imported but not used: "os"` (alternate gc format)
fn classify_go_error(message: &str) -> String {
    if message.contains("undefined:") && !message.contains("has no field or method") {
        "undefined".to_string()
    } else if message.contains("cannot use") && message.contains("as type") {
        "type_mismatch".to_string()
    } else if message.contains("has no field or method") {
        "field_not_found".to_string()
    } else if message.contains("imported and not used") || message.contains("imported but not used")
    {
        // Both "imported and not used" and "imported but not used"
        // are valid Go compiler phrasings for unused imports.
        "unused_import".to_string()
    } else if message.contains("declared but not used") || message.contains("declared and not used")
    {
        // Both "x declared but not used" and "declared and not used: x"
        // are valid Go compiler phrasings for the same diagnostic.
        "unused_var".to_string()
    } else if message.contains("missing return") {
        "missing_return".to_string()
    } else {
        // Use the raw message prefix as error type
        "go_error".to_string()
    }
}

// ---------------------------------------------------------------------------
// Error type extraction (ported from FastEdit base.py)
// ---------------------------------------------------------------------------

/// Inference table: message pattern -> inferred error type.
/// More specific patterns must come before broader ones.
static INFERENCE_TABLE: &[(&str, &str)] = &[
    // Tier 1 (scope / name)
    ("referenced before assignment", "UnboundLocalError"),
    ("cannot access local variable", "UnboundLocalError"),
    ("is not defined", "NameError"),
    // Tier 2 (types)
    ("object is not callable", "TypeError"),
    ("not JSON serializable", "TypeError"),
    ("unexpected keyword argument", "TypeError"),
    ("required positional argument", "TypeError"),
    ("has no attribute", "AttributeError"),
    ("object is not subscriptable", "TypeError"),
    ("object is not iterable", "TypeError"),
    ("unhashable type", "TypeError"),
    ("argument of type", "TypeError"),
    // Tier 0 (compile-time) -- must come after "has no attribute"
    ("inconsistent use of tabs and spaces", "IndentationError"),
    ("expected an indented block", "IndentationError"),
    ("unexpected indent", "IndentationError"),
    ("unindent does not match", "IndentationError"),
    ("expected ':'", "SyntaxError"),
    ("invalid syntax", "SyntaxError"),
    ("prior to global declaration", "SyntaxError"),
    // Tier 3 (imports)
    ("partially initialized module", "ImportError"),
    ("cannot import name", "ImportError"),
    ("No module named", "ImportError"),
    // Tier 4 (lookups)
    ("list index out of range", "IndexError"),
    ("string index out of range", "IndexError"),
    ("tuple index out of range", "IndexError"),
    // Tier 5 (value / unicode / arithmetic)
    ("invalid literal for int", "ValueError"),
    ("not enough values to unpack", "ValueError"),
    ("too many values to unpack", "ValueError"),
    ("substring not found", "ValueError"),
    ("codec can't decode", "UnicodeError"),
    ("codec can't encode", "UnicodeError"),
    ("float division by zero", "ZeroDivisionError"),
    ("integer division or modulo by zero", "ZeroDivisionError"),
    ("division by zero", "ZeroDivisionError"),
    // Tier 6 (runtime / control flow)
    ("maximum recursion depth exceeded", "RecursionError"),
    // Tier 7 (OS / resources)
    ("No such file or directory", "OSError"),
    ("Permission denied", "OSError"),
    ("Is a directory", "OSError"),
    ("File exists", "OSError"),
];

/// Extract the error type from an error string.
///
/// Handles two formats:
/// 1. Explicit prefix: `UnboundLocalError: message` -> `UnboundLocalError`
/// 2. Message inference: `"referenced before assignment"` -> `UnboundLocalError`
///
/// Returns `"Unknown"` if the error type cannot be determined.
pub fn extract_error_type(error_string: &str) -> String {
    if error_string.is_empty() {
        return "Unknown".to_string();
    }

    // Try explicit prefix: "ErrorType: message" or bare "ErrorType:" at end
    if let Some(caps) = Regex::new(r"^([A-Z]\w+):\s?")
        .ok()
        .and_then(|re| re.captures(error_string.trim()))
    {
        let name = caps.get(1).unwrap().as_str();
        if name.ends_with("Error")
            || name.ends_with("Exception")
            || name.ends_with("Iteration")
            || name == "KeyError"
            || name == "StopIteration"
            || name == "StopAsyncIteration"
        {
            return name.to_string();
        }
    }

    // Infer from message patterns
    for (pattern, error_type) in INFERENCE_TABLE {
        if error_string.contains(pattern) {
            return (*error_type).to_string();
        }
    }

    "Unknown".to_string()
}

/// Extract a variable name from common error message patterns.
///
/// Handles:
/// - `local variable 'x' referenced before assignment`
/// - `cannot access local variable 'x'`
/// - `name 'x' is not defined`
/// - `has no attribute 'x'`
pub fn extract_variable_name(error_message: &str) -> Option<String> {
    // Python <3.12: "local variable 'x' referenced before assignment"
    if let Some(caps) = Regex::new(r"local variable '(\w+)' referenced before assignment")
        .ok()
        .and_then(|re| re.captures(error_message))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // Python 3.12+: "cannot access local variable 'x'"
    if let Some(caps) = Regex::new(r"cannot access local variable '(\w+)'")
        .ok()
        .and_then(|re| re.captures(error_message))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // "name 'x' is not defined"
    if let Some(caps) = Regex::new(r"name '(\w+)' is not defined")
        .ok()
        .and_then(|re| re.captures(error_message))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    // "has no attribute 'x'"
    if let Some(caps) = Regex::new(r"has no attribute '(\w+)'")
        .ok()
        .and_then(|re| re.captures(error_message))
    {
        return Some(caps.get(1).unwrap().as_str().to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_error_type_explicit_prefix() {
        assert_eq!(
            extract_error_type("UnboundLocalError: cannot access local variable 'x'"),
            "UnboundLocalError"
        );
        assert_eq!(
            extract_error_type("TypeError: 'dict' object is not callable"),
            "TypeError"
        );
        assert_eq!(extract_error_type("KeyError: 'name'"), "KeyError");
        assert_eq!(extract_error_type("StopIteration: "), "StopIteration");
    }

    #[test]
    fn test_extract_error_type_inference() {
        assert_eq!(
            extract_error_type("referenced before assignment"),
            "UnboundLocalError"
        );
        assert_eq!(extract_error_type("name 'os' is not defined"), "NameError");
        assert_eq!(extract_error_type("division by zero"), "ZeroDivisionError");
        assert_eq!(
            extract_error_type("maximum recursion depth exceeded"),
            "RecursionError"
        );
        assert_eq!(extract_error_type("list index out of range"), "IndexError");
    }

    #[test]
    fn test_extract_error_type_unknown() {
        assert_eq!(extract_error_type(""), "Unknown");
        assert_eq!(extract_error_type("some random text"), "Unknown");
    }

    #[test]
    fn test_extract_variable_name() {
        assert_eq!(
            extract_variable_name("local variable 'counter' referenced before assignment"),
            Some("counter".to_string())
        );
        assert_eq!(
            extract_variable_name("cannot access local variable 'x'"),
            Some("x".to_string())
        );
        assert_eq!(
            extract_variable_name("name 'os' is not defined"),
            Some("os".to_string())
        );
        assert_eq!(
            extract_variable_name("has no attribute 'frobnicate'"),
            Some("frobnicate".to_string())
        );
        assert_eq!(extract_variable_name("some random text"), None);
    }

    #[test]
    fn test_parse_python_traceback() {
        let tb = "\
Traceback (most recent call last):
  File \"app.py\", line 10, in inc
    counter += 1
UnboundLocalError: cannot access local variable 'counter'";

        let parsed = parse_python_error(tb).unwrap();
        assert_eq!(parsed.error_type, "UnboundLocalError");
        assert_eq!(parsed.message, "cannot access local variable 'counter'");
        assert_eq!(parsed.file, Some(PathBuf::from("app.py")));
        assert_eq!(parsed.line, Some(10));
        assert_eq!(parsed.function_name, Some("inc".to_string()));
        assert_eq!(parsed.offending_line, Some("counter += 1".to_string()));
        assert_eq!(parsed.language, "python");
    }

    #[test]
    fn test_parse_python_single_line() {
        let err = "UnboundLocalError: cannot access local variable 'counter'";
        let parsed = parse_python_error(err).unwrap();
        assert_eq!(parsed.error_type, "UnboundLocalError");
        assert_eq!(parsed.message, "cannot access local variable 'counter'");
        assert_eq!(parsed.language, "python");
    }

    #[test]
    fn test_detect_language_python() {
        assert_eq!(
            detect_language("Traceback (most recent call last):"),
            "python"
        );
        assert_eq!(detect_language("UnboundLocalError: something"), "python");
    }

    #[test]
    fn test_detect_language_unknown() {
        assert_eq!(detect_language("just some text"), "unknown");
    }

    #[test]
    fn test_parse_error_with_lang() {
        let err = "NameError: name 'os' is not defined";
        let parsed = parse_error(err, Some("python")).unwrap();
        assert_eq!(parsed.error_type, "NameError");
    }

    #[test]
    fn test_parse_error_auto_detect() {
        let err = "TypeError: 'dict' object is not callable";
        let parsed = parse_error(err, None).unwrap();
        assert_eq!(parsed.error_type, "TypeError");
        assert_eq!(parsed.language, "python");
    }

    #[test]
    fn test_parse_traceback_multiple_frames() {
        let tb = "\
Traceback (most recent call last):
  File \"main.py\", line 5, in main
    result = process(data)
  File \"utils.py\", line 12, in process
    return data[key]
KeyError: 'name'";

        let parsed = parse_python_error(tb).unwrap();
        assert_eq!(parsed.error_type, "KeyError");
        // Should extract from the LAST frame
        assert_eq!(parsed.file, Some(PathBuf::from("utils.py")));
        assert_eq!(parsed.line, Some(12));
        assert_eq!(parsed.function_name, Some("process".to_string()));
        assert_eq!(parsed.offending_line, Some("return data[key]".to_string()));
    }

    // ---- Rust error parser tests ----

    #[test]
    fn test_parse_rustc_json_direct() {
        let json = r#"{"code":"E0599","message":"no method named `read_line` found for struct `Stdin`","spans":[{"file_name":"src/main.rs","line_start":3,"column_start":21,"is_primary":true,"text":[{"text":"    std::io::stdin().read_line(&mut buf);"}]}],"level":"error"}"#;
        let parsed = parse_rustc_error(json);
        assert!(parsed.is_some(), "Should parse direct rustc JSON");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "E0599");
        assert!(p.message.contains("read_line"));
        assert_eq!(p.file, Some(PathBuf::from("src/main.rs")));
        assert_eq!(p.line, Some(3));
        assert_eq!(p.column, Some(21));
        assert_eq!(p.language, "rust");
    }

    #[test]
    fn test_parse_rustc_json_cargo_wrapper() {
        let json = r#"{"reason":"compiler-message","message":{"code":{"code":"E0425","explanation":null},"message":"cannot find type `HashMap` in this scope","level":"error","spans":[{"file_name":"src/main.rs","line_start":2,"column_start":12,"is_primary":true}]}}"#;
        let parsed = parse_rustc_error(json);
        assert!(parsed.is_some(), "Should parse cargo JSON wrapper");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "E0425");
        assert!(p.message.contains("HashMap"));
        assert_eq!(p.line, Some(2));
    }

    #[test]
    fn test_parse_rustc_rendered_text() {
        let rendered = "error[E0599]: no method named `write_all` found for struct `File`\n  --> src/main.rs:5:7\n";
        let parsed = parse_rustc_error(rendered);
        assert!(parsed.is_some(), "Should parse rendered rustc text");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "E0599");
        assert!(p.message.contains("write_all"));
        assert_eq!(p.file, Some(PathBuf::from("src/main.rs")));
        assert_eq!(p.line, Some(5));
        assert_eq!(p.column, Some(7));
    }

    #[test]
    fn test_detect_language_rust() {
        let json = r#"{"code":"E0599","message":"no method found","level":"error"}"#;
        assert_eq!(detect_language(json), "rust");
    }

    #[test]
    fn test_parse_error_auto_detect_rust() {
        let json =
            r#"{"code":"E0425","message":"cannot find type `HashMap`","level":"error","spans":[]}"#;
        let parsed = parse_error(json, None);
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.language, "rust");
        assert_eq!(p.error_type, "E0425");
    }

    #[test]
    fn test_parse_error_explicit_lang_rust() {
        let json = r#"{"code":"E0308","message":"mismatched types","level":"error","spans":[]}"#;
        let parsed = parse_error(json, Some("rust"));
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "E0308");
        assert_eq!(p.language, "rust");
    }

    #[test]
    fn test_parse_rustc_warning_ignored() {
        let json = r#"{"code":"","message":"unused variable `x`","level":"warning","spans":[]}"#;
        let parsed = parse_rustc_json(json);
        assert!(parsed.is_none(), "Warnings should be ignored");
    }

    // ---- TypeScript (tsc) error parser tests ----

    #[test]
    fn test_parse_tsc_error_basic() {
        let err = "app.ts(1,13): error TS2304: Cannot find name 'express'.";
        let parsed = parse_tsc_error(err);
        assert!(parsed.is_some(), "Should parse tsc error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2304");
        assert_eq!(p.message, "Cannot find name 'express'");
        assert_eq!(p.file, Some(PathBuf::from("app.ts")));
        assert_eq!(p.line, Some(1));
        assert_eq!(p.column, Some(13));
        assert_eq!(p.language, "typescript");
    }

    #[test]
    fn test_parse_tsc_error_tsx_file() {
        let err =
            "components/App.tsx(42,10): error TS2339: Property 'foo' does not exist on type 'Bar'.";
        let parsed = parse_tsc_error(err);
        assert!(parsed.is_some(), "Should parse tsc error for .tsx files");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2339");
        assert!(p.message.contains("foo"));
        assert_eq!(p.file, Some(PathBuf::from("components/App.tsx")));
        assert_eq!(p.line, Some(42));
        assert_eq!(p.column, Some(10));
    }

    #[test]
    fn test_parse_tsc_error_multiline() {
        let err = "some warning output\napp.ts(10,5): error TS2322: Type 'string' is not assignable to type 'number'.\nmore output";
        let parsed = parse_tsc_error(err);
        assert!(
            parsed.is_some(),
            "Should parse tsc error from multiline output"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2322");
        assert_eq!(p.line, Some(10));
    }

    #[test]
    fn test_parse_tsc_error_not_tsc() {
        let err = "this is not a tsc error";
        let parsed = parse_tsc_error(err);
        assert!(parsed.is_none(), "Should not parse non-tsc text");
    }

    #[test]
    fn test_detect_language_typescript() {
        assert_eq!(
            detect_language("app.ts(1,13): error TS2304: Cannot find name 'express'."),
            "typescript"
        );
        assert_eq!(
            detect_language("file.tsx(42,10): error TS2339: Property does not exist."),
            "typescript"
        );
    }

    #[test]
    fn test_parse_error_auto_detect_typescript() {
        let err = "app.ts(1,13): error TS2304: Cannot find name 'express'.";
        let parsed = parse_error(err, None);
        assert!(parsed.is_some(), "Should auto-detect TypeScript");
        let p = parsed.unwrap();
        assert_eq!(p.language, "typescript");
        assert_eq!(p.error_type, "TS2304");
    }

    #[test]
    fn test_parse_error_explicit_lang_typescript() {
        let err = "app.ts(5,10): error TS7006: Parameter 'x' implicitly has an 'any' type.";
        let parsed = parse_error(err, Some("typescript"));
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS7006");
        assert_eq!(p.language, "typescript");
    }

    // ---- Go error parser tests ----

    #[test]
    fn test_parse_go_error_undefined() {
        let err = "./main.go:4:7: undefined: fmt";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go undefined error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "undefined");
        assert_eq!(p.message, "undefined: fmt");
        assert_eq!(p.file, Some(PathBuf::from("./main.go")));
        assert_eq!(p.line, Some(4));
        assert_eq!(p.column, Some(7));
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_type_mismatch() {
        let err = "./main.go:5:17: cannot use s (variable of type string) as type []byte in variable declaration";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go type mismatch error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "type_mismatch");
        assert!(p.message.contains("cannot use"));
        assert_eq!(p.line, Some(5));
    }

    #[test]
    fn test_parse_go_error_unused_import() {
        let err = "./main.go:5:2: \"os\" imported and not used";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go unused import error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
        assert!(p.message.contains("imported and not used"));
    }

    #[test]
    fn test_parse_go_error_unused_var() {
        let err = "./main.go:4:2: x declared but not used";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go unused var error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_var");
    }

    #[test]
    fn test_parse_go_error_missing_return() {
        let err = "./main.go:3:1: missing return at end of function";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go missing return error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "missing_return");
    }

    #[test]
    fn test_parse_go_error_field_not_found() {
        let err = "./main.go:7:13: strings.Contians undefined (type strings has no field or method Contians)";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse Go field not found error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "field_not_found");
    }

    #[test]
    fn test_detect_language_go() {
        assert_eq!(detect_language("./main.go:4:7: undefined: fmt"), "go");
        assert_eq!(
            detect_language("./pkg/handler.go:10:5: x declared but not used"),
            "go"
        );
    }

    #[test]
    fn test_parse_error_auto_detect_go() {
        let err = "./main.go:4:7: undefined: fmt";
        let parsed = parse_error(err, None);
        assert!(parsed.is_some(), "Should auto-detect Go");
        let p = parsed.unwrap();
        assert_eq!(p.language, "go");
        assert_eq!(p.error_type, "undefined");
    }

    #[test]
    fn test_parse_error_explicit_lang_go() {
        let err = "./main.go:3:1: missing return at end of function";
        let parsed = parse_error(err, Some("go"));
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "missing_return");
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_not_go() {
        let err = "this is not a go error";
        let parsed = parse_go_error(err);
        assert!(parsed.is_none(), "Should not parse non-go text");
    }

    #[test]
    fn test_classify_go_error() {
        assert_eq!(classify_go_error("undefined: fmt"), "undefined");
        assert_eq!(
            classify_go_error("cannot use s (variable of type string) as type []byte"),
            "type_mismatch"
        );
        assert_eq!(
            classify_go_error("strings.X undefined (type strings has no field or method X)"),
            "field_not_found"
        );
        assert_eq!(
            classify_go_error("\"os\" imported and not used"),
            "unused_import"
        );
        assert_eq!(classify_go_error("x declared but not used"), "unused_var");
        // Alternate phrasing: "declared and not used: x"
        assert_eq!(classify_go_error("declared and not used: x"), "unused_var");
        assert_eq!(
            classify_go_error("missing return at end of function"),
            "missing_return"
        );
        assert_eq!(classify_go_error("some other error"), "go_error");
    }

    #[test]
    fn test_parse_go_error_unused_var_alt_format() {
        // Alternate Go compiler phrasing: "declared and not used: x"
        let err = "./main.go:4:2: declared and not used: x";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse Go unused var (alternate format)"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_var");
        assert_eq!(p.file, Some(PathBuf::from("./main.go")));
        assert_eq!(p.line, Some(4));
        assert_eq!(p.column, Some(2));
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_detect_language_go_alt_unused_var_format() {
        // detect_language must recognise the alternate "declared and not used" phrasing
        assert_eq!(
            detect_language("./main.go:4:2: declared and not used: x"),
            "go"
        );
    }

    #[test]
    fn test_parse_error_go_alt_unused_var_autodetect() {
        // End-to-end: auto-detect + parse for alternate unused-var phrasing
        let err = "./cmd/main.go:8:5: declared and not used: cfg";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect and parse alternate Go unused var"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "go");
        assert_eq!(p.error_type, "unused_var");
        assert!(p.message.contains("declared and not used"));
    }

    // ---- JavaScript (Node.js) error parser tests ----

    #[test]
    fn test_parse_js_error_reference_error() {
        let err = "/app/app.js:1\nconst data = fs.readFileSync('config.json');\n             ^\nReferenceError: fs is not defined\n    at Object.<anonymous> (/app/app.js:1:14)";
        let parsed = parse_js_error(err);
        assert!(parsed.is_some(), "Should parse JS ReferenceError");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "ReferenceError");
        assert_eq!(p.message, "fs is not defined");
        assert_eq!(p.file, Some(PathBuf::from("/app/app.js")));
        assert_eq!(p.line, Some(1));
        assert_eq!(p.language, "javascript");
        assert_eq!(
            p.offending_line,
            Some("const data = fs.readFileSync('config.json');".to_string())
        );
    }

    #[test]
    fn test_parse_js_error_type_error() {
        let err = "/app/app.js:2\nconst len = arr.length();\n                      ^\nTypeError: arr.length is not a function\n    at Object.<anonymous> (/app/app.js:2:27)";
        let parsed = parse_js_error(err);
        assert!(parsed.is_some(), "Should parse JS TypeError");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TypeError");
        assert!(p.message.contains("is not a function"));
        assert_eq!(p.file, Some(PathBuf::from("/app/app.js")));
        assert_eq!(p.line, Some(2));
    }

    #[test]
    fn test_parse_js_error_syntax_error() {
        let err = "/app/app.js:3\n  age: 30\n  ^^^\nSyntaxError: Unexpected identifier\n    at Object.<anonymous> (/app/app.js:3:2)";
        let parsed = parse_js_error(err);
        assert!(parsed.is_some(), "Should parse JS SyntaxError");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "SyntaxError");
        assert!(p.message.contains("Unexpected identifier"));
    }

    #[test]
    fn test_parse_js_error_single_line() {
        let err = "TypeError: Cannot read properties of undefined (reading 'name')\n    at Object.<anonymous> (/app/app.js:5:20)";
        let parsed = parse_js_error(err);
        assert!(parsed.is_some(), "Should parse single-line JS error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TypeError");
        assert!(p.message.contains("Cannot read properties"));
        // Should extract file from stack trace
        assert_eq!(p.file, Some(PathBuf::from("/app/app.js")));
        assert_eq!(p.line, Some(5));
        assert_eq!(p.column, Some(20));
    }

    #[test]
    fn test_parse_js_error_not_js() {
        let err = "this is not a javascript error";
        let parsed = parse_js_error(err);
        assert!(parsed.is_none(), "Should not parse non-JS text");
    }

    #[test]
    fn test_detect_language_javascript() {
        assert_eq!(
            detect_language("/app/app.js:1\nfs.readFileSync\nReferenceError: fs is not defined\n    at Object.<anonymous> (/app/app.js:1:14)"),
            "javascript"
        );
        assert_eq!(
            detect_language("TypeError: Cannot read properties of undefined\n    at Module._compile (node:internal/modules/cjs/loader:1234:14)"),
            "javascript"
        );
    }

    #[test]
    fn test_parse_error_auto_detect_javascript() {
        let err = "/app/app.js:1\nconst data = fs.readFileSync('file.txt');\n             ^\nReferenceError: fs is not defined\n    at Object.<anonymous> (/app/app.js:1:14)";
        let parsed = parse_error(err, None);
        assert!(parsed.is_some(), "Should auto-detect JavaScript");
        let p = parsed.unwrap();
        assert_eq!(p.language, "javascript");
        assert_eq!(p.error_type, "ReferenceError");
    }

    #[test]
    fn test_parse_error_explicit_lang_javascript() {
        let err =
            "ReferenceError: path is not defined\n    at Object.<anonymous> (/app/app.js:1:14)";
        let parsed = parse_error(err, Some("javascript"));
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "ReferenceError");
        assert_eq!(p.language, "javascript");
    }

    #[test]
    fn test_parse_js_error_mjs_file() {
        let err = "/app/utils.mjs:10\nimport { foo } from './bar.mjs';\n       ^\nSyntaxError: Unexpected token '}'\n    at Object.<anonymous> (/app/utils.mjs:10:8)";
        let parsed = parse_js_error(err);
        assert!(parsed.is_some(), "Should parse .mjs file errors");
        let p = parsed.unwrap();
        assert_eq!(p.file, Some(PathBuf::from("/app/utils.mjs")));
        assert_eq!(p.line, Some(10));
    }

    // ---- Bug regression tests ----

    /// Bug 1: Node.js ReferenceError with .js file path must NOT be detected as Python.
    /// The `ReferenceError:` prefix matches `^[A-Z]\w*Error:\s` but the .js file references
    /// and `at Object.<anonymous>` stack trace should cause JS detection to win.
    #[test]
    fn test_bug1_js_autodetect_not_misclassified_as_python() {
        let err = "/path/to/file.js:10\n    undefinedVar.foo()\n    ^\nReferenceError: undefinedVar is not defined\n    at Object.<anonymous> (/path/to/file.js:10:1)";
        // Auto-detection must return "javascript", NOT "python"
        assert_eq!(
            detect_language(err),
            "javascript",
            "Node.js error with .js file refs and at Object.<anonymous> must detect as javascript"
        );
        // parse_error with auto-detection must produce a JS ParsedError
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should parse Node.js error with auto-detection"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "javascript");
        assert_eq!(p.error_type, "ReferenceError");
        assert_eq!(p.message, "undefinedVar is not defined");
        assert_eq!(p.file, Some(PathBuf::from("/path/to/file.js")));
        assert_eq!(p.line, Some(10));
    }

    /// Bug 1 variant: JS error with ONLY file.js reference (no stack trace keywords).
    /// detect_language must still pick up the .js file path.
    #[test]
    fn test_bug1_js_autodetect_file_path_only() {
        let err = "/path/to/file.js:10\n    undefinedVar.foo()\n    ^\nReferenceError: undefinedVar is not defined";
        assert_eq!(
            detect_language(err),
            "javascript",
            "Node.js error with .js file path but no stack trace should still detect as javascript"
        );
    }

    /// Bug 1 variant: Pure `ReferenceError:` without JS context must still be Python.
    #[test]
    fn test_bug1_python_reference_error_without_js_context() {
        let err = "ReferenceError: something is not defined";
        // No .js file references, no `at Object.<anonymous>` => should be Python
        // (Python doesn't have ReferenceError but the generic pattern matches)
        assert_eq!(
            detect_language(err),
            "python",
            "ReferenceError without any JS context should fall through to python"
        );
    }

    /// Bug 2: Rendered Rust error format must be auto-detected as "rust".
    /// `error[E0425]: cannot find value...` is NOT JSON, so the JSON detection
    /// (`"code"` + `"E0"`) does not fire. detect_language must also handle rendered format.
    #[test]
    fn test_bug2_rust_rendered_autodetect() {
        let err = "error[E0425]: cannot find value `HashMap` in this scope\n  --> src/main.rs:2:12";
        assert_eq!(
            detect_language(err),
            "rust",
            "Rendered rustc error[EXXXX] format must be auto-detected as rust"
        );
    }

    /// Bug 2: parse_error with auto-detection on rendered Rust error.
    #[test]
    fn test_bug2_rust_rendered_parse_error_auto() {
        let err = "error[E0425]: cannot find value `HashMap` in this scope\n  --> src/main.rs:2:12";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect and parse rendered Rust error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "rust");
        assert_eq!(p.error_type, "E0425");
        assert!(p.message.contains("HashMap"));
        assert_eq!(p.file, Some(PathBuf::from("src/main.rs")));
        assert_eq!(p.line, Some(2));
        assert_eq!(p.column, Some(12));
    }

    /// Bug 2: parse_error with explicit --lang rust on rendered error.
    #[test]
    fn test_bug2_rust_rendered_parse_error_explicit_lang() {
        let err = "error[E0425]: cannot find value `HashMap` in this scope\n  --> src/main.rs:2:12";
        let parsed = parse_error(err, Some("rust"));
        assert!(
            parsed.is_some(),
            "Should parse rendered Rust error with explicit lang"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "rust");
        assert_eq!(p.error_type, "E0425");
        assert_eq!(p.file, Some(PathBuf::from("src/main.rs")));
        assert_eq!(p.line, Some(2));
        assert_eq!(p.column, Some(12));
    }

    /// Bug 2 variant: Multiple rendered Rust errors.
    #[test]
    fn test_bug2_rust_rendered_multiple_errors() {
        let err = "error[E0425]: cannot find value `HashMap` in this scope\n  --> src/main.rs:2:12\n   |\n2  |     let m = HashMap::new();\n   |             ^^^^^^^ not found in this scope\n\nerror[E0308]: mismatched types\n  --> src/main.rs:5:10";
        // Should detect as rust
        assert_eq!(detect_language(err), "rust");
        // Should parse the first error
        let parsed = parse_error(err, None);
        assert!(parsed.is_some());
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "E0425");
    }

    /// Bug 3: Go error with quoted package name in unused import message.
    /// The format `"fmt" imported and not used` must parse correctly.
    #[test]
    fn test_bug3_go_unused_import_quoted_package() {
        let err = "./main.go:5:2: \"fmt\" imported and not used";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse Go unused import with quoted package name"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
        assert_eq!(p.file, Some(PathBuf::from("./main.go")));
        assert_eq!(p.line, Some(5));
        assert_eq!(p.column, Some(2));
        assert!(p.message.contains("imported and not used"));
        assert_eq!(p.language, "go");
    }

    /// Bug 3: Go error auto-detection with quoted package unused import.
    #[test]
    fn test_bug3_go_unused_import_quoted_autodetect() {
        let err = "./main.go:5:2: \"fmt\" imported and not used";
        assert_eq!(detect_language(err), "go");
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect Go unused import with quotes"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "go");
        assert_eq!(p.error_type, "unused_import");
    }

    /// Bug 3 variant: classify_go_error with quoted package name.
    #[test]
    fn test_bug3_classify_go_error_quoted_import() {
        assert_eq!(
            classify_go_error("\"fmt\" imported and not used"),
            "unused_import"
        );
        assert_eq!(
            classify_go_error("\"os/exec\" imported and not used"),
            "unused_import"
        );
    }

    // ---- Simplified (bare) error format tests ----

    // TypeScript simplified: bare `error TS2304: Cannot find name 'express'.`
    // without the file(line,col) prefix.

    #[test]
    fn test_parse_tsc_error_simplified_ts2304() {
        let err = "error TS2304: Cannot find name 'express'.";
        let parsed = parse_tsc_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified tsc error without file prefix"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2304");
        assert_eq!(p.message, "Cannot find name 'express'");
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
        assert_eq!(p.column, None);
        assert_eq!(p.language, "typescript");
    }

    #[test]
    fn test_parse_tsc_error_simplified_ts2339() {
        let err = "error TS2339: Property 'foo' does not exist on type 'Bar'.";
        let parsed = parse_tsc_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified tsc property error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2339");
        assert!(p.message.contains("foo"));
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
    }

    #[test]
    fn test_parse_tsc_error_simplified_ts2322() {
        let err = "error TS2322: Type 'string' is not assignable to type 'number'.";
        let parsed = parse_tsc_error(err);
        assert!(parsed.is_some(), "Should parse simplified tsc type error");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2322");
        assert!(p.message.contains("assignable"));
        assert_eq!(p.file, None);
    }

    #[test]
    fn test_parse_tsc_error_simplified_no_trailing_dot() {
        let err = "error TS7006: Parameter 'x' implicitly has an 'any' type";
        let parsed = parse_tsc_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified tsc error without trailing dot"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS7006");
        assert!(p.message.contains("implicitly"));
    }

    #[test]
    fn test_parse_tsc_error_simplified_multiline_input() {
        // A multi-line input where one line is simplified TS error
        let err =
            "Some preamble output\nerror TS2304: Cannot find name 'React'.\nSome trailing output";
        let parsed = parse_tsc_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified tsc error from multiline input"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2304");
        assert!(p.message.contains("React"));
        assert_eq!(p.file, None);
    }

    #[test]
    fn test_parse_tsc_error_simplified_via_parse_error() {
        // End-to-end: auto-detect + parse for simplified TS
        let err = "error TS2304: Cannot find name 'express'.";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect and parse simplified TS error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "typescript");
        assert_eq!(p.error_type, "TS2304");
    }

    #[test]
    fn test_parse_tsc_error_simplified_explicit_lang() {
        let err = "error TS2339: Property 'x' does not exist on type 'Y'.";
        let parsed = parse_error(err, Some("typescript"));
        assert!(
            parsed.is_some(),
            "Should parse simplified TS error with explicit lang"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TS2339");
        assert_eq!(p.language, "typescript");
        assert_eq!(p.file, None);
    }

    // Go simplified: bare Go error messages without file:line:col prefix.

    #[test]
    fn test_parse_go_error_simplified_undefined() {
        let err = "undefined: fmt";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go undefined error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "undefined");
        assert_eq!(p.message, "undefined: fmt");
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
        assert_eq!(p.column, None);
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_simplified_unused_var() {
        let err = "x declared but not used";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go unused var error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_var");
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_simplified_unused_var_alt() {
        let err = "declared and not used: cfg";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go unused var (alt format)"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_var");
        assert_eq!(p.file, None);
    }

    #[test]
    fn test_parse_go_error_simplified_unused_import() {
        let err = "imported and not used: \"os\"";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse simplified Go unused import");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_simplified_unused_import_quoted() {
        let err = "\"fmt\" imported and not used";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go unused import (quoted)"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
        assert_eq!(p.file, None);
    }

    #[test]
    fn test_parse_go_error_simplified_missing_return() {
        let err = "missing return at end of function";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go missing return"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "missing_return");
        assert_eq!(p.file, None);
        assert_eq!(p.line, None);
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_simplified_type_mismatch() {
        let err = "cannot use s (variable of type string) as type []byte";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse simplified Go type mismatch");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "type_mismatch");
        assert_eq!(p.file, None);
        assert_eq!(p.language, "go");
    }

    #[test]
    fn test_parse_go_error_simplified_field_not_found() {
        let err = "strings.Contians undefined (type strings has no field or method Contians)";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse simplified Go field not found"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "field_not_found");
        assert_eq!(p.file, None);
    }

    #[test]
    fn test_parse_go_error_simplified_via_parse_error() {
        // End-to-end: auto-detect + parse for simplified Go
        let err = "undefined: fmt";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect and parse simplified Go error"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "go");
        assert_eq!(p.error_type, "undefined");
    }

    #[test]
    fn test_parse_go_error_simplified_explicit_lang() {
        let err = "x declared but not used";
        let parsed = parse_error(err, Some("go"));
        assert!(
            parsed.is_some(),
            "Should parse simplified Go error with explicit lang"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_var");
        assert_eq!(p.language, "go");
        assert_eq!(p.file, None);
    }

    // detect_language tests for simplified formats

    #[test]
    fn test_detect_language_ts_simplified() {
        assert_eq!(
            detect_language("error TS2339: Property does not exist"),
            "typescript"
        );
    }

    #[test]
    fn test_detect_language_ts_simplified_ts2304() {
        assert_eq!(
            detect_language("error TS2304: Cannot find name 'express'."),
            "typescript"
        );
    }

    #[test]
    fn test_detect_language_go_simplified_undefined() {
        assert_eq!(detect_language("undefined: fmt"), "go");
    }

    #[test]
    fn test_detect_language_go_simplified_unused_var() {
        assert_eq!(detect_language("x declared but not used"), "go");
    }

    #[test]
    fn test_detect_language_go_simplified_unused_var_alt() {
        assert_eq!(detect_language("declared and not used: x"), "go");
    }

    #[test]
    fn test_detect_language_go_simplified_unused_import() {
        assert_eq!(detect_language("imported and not used: \"os\""), "go");
    }

    #[test]
    fn test_detect_language_go_simplified_missing_return() {
        assert_eq!(detect_language("missing return at end of function"), "go");
    }

    #[test]
    fn test_detect_language_go_simplified_type_mismatch() {
        assert_eq!(
            detect_language("cannot use s (variable of type string) as type []byte"),
            "go"
        );
    }

    // ---- Bug 4: JS TypeError auto-detection ----

    /// Bug 4: `TypeError: Cannot read properties of undefined (reading 'foo')`
    /// without any Node.js stack trace or .js file reference must auto-detect
    /// as JavaScript, NOT Python. This is a Node.js-specific error format
    /// that never appears in Python.
    #[test]
    fn test_bug4_js_typeerror_cannot_read_properties_autodetect() {
        let err = "TypeError: Cannot read properties of undefined (reading 'foo')";
        assert_eq!(
            detect_language(err),
            "javascript",
            "JS TypeError 'Cannot read properties of undefined' must detect as javascript"
        );
    }

    /// Bug 4 variant: Cannot read properties of null.
    #[test]
    fn test_bug4_js_typeerror_cannot_read_properties_null() {
        let err = "TypeError: Cannot read properties of null (reading 'bar')";
        assert_eq!(
            detect_language(err),
            "javascript",
            "JS TypeError 'Cannot read properties of null' must detect as javascript"
        );
    }

    /// Bug 4 variant: Cannot set properties of undefined.
    #[test]
    fn test_bug4_js_typeerror_cannot_set_properties() {
        let err = "TypeError: Cannot set properties of undefined (setting 'x')";
        assert_eq!(
            detect_language(err),
            "javascript",
            "JS TypeError 'Cannot set properties of undefined' must detect as javascript"
        );
    }

    /// Bug 4 variant: X is not a function (without any stack trace).
    #[test]
    fn test_bug4_js_typeerror_not_a_function() {
        let err = "TypeError: foo.bar is not a function";
        assert_eq!(
            detect_language(err),
            "javascript",
            "JS TypeError 'X is not a function' must detect as javascript"
        );
    }

    /// Bug 4: End-to-end parse_error auto-detection for JS TypeError.
    #[test]
    fn test_bug4_js_typeerror_parse_error_autodetect() {
        let err = "TypeError: Cannot read properties of undefined (reading 'name')";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should parse JS TypeError with auto-detection"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "javascript");
        assert_eq!(p.error_type, "TypeError");
        assert!(p.message.contains("Cannot read properties"));
    }

    /// Bug 4: "X is not a function" must also parse as JS via parse_error.
    #[test]
    fn test_bug4_js_typeerror_not_a_function_parse() {
        let err = "TypeError: myCallback is not a function";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should parse JS 'not a function' TypeError"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "javascript");
        assert_eq!(p.error_type, "TypeError");
        assert!(p.message.contains("is not a function"));
    }

    // ---- Bug 2: Verbose pytest output ----

    /// Bug 2: Verbose pytest output with decorations wrapping a Python traceback.
    /// The parser must strip pytest decoration and extract the NameError correctly.
    #[test]
    fn test_bug2_verbose_pytest_traceback_extraction() {
        let pytest_output = "\
============================= FAILURES =============================
_____________ test_foo _____________

    def test_foo():
>       result = compute(x)

Traceback (most recent call last):
  File \"test_app.py\", line 5, in test_foo
    result = compute(x)
NameError: name 'x' is not defined

========================= short test summary info =========================
FAILED test_app.py::test_foo - NameError: name 'x' is not defined
1 failed in 0.12s";

        let parsed = parse_error(pytest_output, Some("python"));
        assert!(parsed.is_some(), "Should parse verbose pytest output");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "NameError");
        assert_eq!(p.message, "name 'x' is not defined");
        assert_eq!(p.file, Some(PathBuf::from("test_app.py")));
        assert_eq!(p.line, Some(5));
        assert_eq!(p.function_name, Some("test_foo".to_string()));
    }

    /// Bug 2: Pytest summary-only output (no full traceback, just FAILED line).
    /// When pytest only shows the short summary, the parser should extract
    /// the error from the `FAILED file::test - ErrorType: message` line.
    #[test]
    fn test_bug2_pytest_summary_only() {
        let pytest_output = "\
========================= short test summary info =========================
FAILED test_app.py::test_foo - NameError: name 'compute' is not defined
1 failed in 0.12s";

        let parsed = parse_error(pytest_output, Some("python"));
        assert!(parsed.is_some(), "Should parse pytest summary-only output");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "NameError");
        assert!(p.message.contains("name 'compute' is not defined"));
    }

    /// Bug 2: Pytest verbose output with captured output, progress bars, etc.
    /// Extra noise must not prevent error extraction.
    #[test]
    fn test_bug2_verbose_pytest_with_captured_output() {
        let pytest_output = "\
============================= test session starts ==============================
platform linux -- Python 3.11.5, pytest-7.4.0, pluggy-1.2.0
rootdir: /home/user/project
collected 3 items

test_app.py::test_alpha PASSED                                        [ 33%]
test_app.py::test_beta PASSED                                         [ 66%]
test_app.py::test_gamma FAILED                                        [100%]

============================= FAILURES =============================
_____________ test_gamma _____________

    def test_gamma():
        data = load_data()
>       process(data)

Traceback (most recent call last):
  File \"test_app.py\", line 15, in test_gamma
    process(data)
  File \"app.py\", line 42, in process
    return data.transform()
AttributeError: 'NoneType' object has no attribute 'transform'

-- Captured stdout --
Loading data from cache...
Done.

========================= short test summary info =========================
FAILED test_app.py::test_gamma - AttributeError: 'NoneType' object has no attribute 'transform'
1 failed, 2 passed in 0.45s";

        let parsed = parse_error(pytest_output, Some("python"));
        assert!(
            parsed.is_some(),
            "Should parse verbose pytest output with captured stdout"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "AttributeError");
        assert!(p.message.contains("has no attribute 'transform'"));
        // Should extract from the LAST traceback frame
        assert_eq!(p.file, Some(PathBuf::from("app.py")));
        assert_eq!(p.line, Some(42));
        assert_eq!(p.function_name, Some("process".to_string()));
    }

    /// Bug 2: Pytest verbose output where the error line is followed by
    /// pytest decoration lines (=== and summary). The error_type extraction
    /// must not pick up decoration lines as the error.
    #[test]
    fn test_bug2_pytest_error_line_not_confused_with_decoration() {
        let pytest_output = "\
Traceback (most recent call last):
  File \"test_app.py\", line 8, in test_something
    result = do_thing()
TypeError: do_thing() missing 1 required positional argument: 'config'

========================= short test summary info =========================
FAILED test_app.py::test_something - TypeError: do_thing() missing 1 required positional argument: 'config'
1 failed in 0.05s";

        let parsed = parse_python_error(pytest_output);
        assert!(
            parsed.is_some(),
            "Should parse traceback followed by pytest decoration"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "TypeError");
        assert!(p.message.contains("missing 1 required positional argument"));
    }

    // Ensure strict-format tests still work (regression guard)

    #[test]
    fn test_strict_tsc_still_preferred_over_fallback() {
        // When file prefix IS present, strict parser should produce file/line/col
        let err = "app.ts(5,10): error TS2304: Cannot find name 'foo'.";
        let parsed = parse_tsc_error(err).unwrap();
        assert_eq!(parsed.file, Some(PathBuf::from("app.ts")));
        assert_eq!(parsed.line, Some(5));
        assert_eq!(parsed.column, Some(10));
        assert_eq!(parsed.error_type, "TS2304");
    }

    #[test]
    fn test_strict_go_still_preferred_over_fallback() {
        // When file prefix IS present, strict parser should produce file/line/col
        let err = "./main.go:4:7: undefined: fmt";
        let parsed = parse_go_error(err).unwrap();
        assert_eq!(parsed.file, Some(PathBuf::from("./main.go")));
        assert_eq!(parsed.line, Some(4));
        assert_eq!(parsed.column, Some(7));
        assert_eq!(parsed.error_type, "undefined");
    }

    // ================================================================
    // Bug fix: "imported but not used" alternate phrasing must be
    // handled by classify_go_error, detect_language, and parse_go_error
    // ================================================================

    #[test]
    fn test_classify_go_error_imported_but_not_used() {
        // "imported but not used" is an alternate Go compiler phrasing
        assert_eq!(
            classify_go_error("\"fmt\" imported but not used"),
            "unused_import",
            "classify_go_error must handle 'imported but not used'"
        );
        assert_eq!(
            classify_go_error("\"os/exec\" imported but not used"),
            "unused_import"
        );
        assert_eq!(
            classify_go_error("imported but not used: \"os\""),
            "unused_import"
        );
    }

    #[test]
    fn test_detect_language_go_imported_but_not_used() {
        assert_eq!(
            detect_language("./main.go:3:8: \"fmt\" imported but not used"),
            "go",
            "detect_language must recognise 'imported but not used' as Go"
        );
        // Also bare (no file prefix)
        assert_eq!(
            detect_language("\"fmt\" imported but not used"),
            "go",
            "detect_language must recognise bare 'imported but not used' as Go"
        );
        assert_eq!(detect_language("imported but not used: \"os\""), "go");
    }

    #[test]
    fn test_parse_go_error_imported_but_not_used() {
        let err = "./main.go:3:8: \"fmt\" imported but not used";
        let parsed = parse_go_error(err);
        assert!(parsed.is_some(), "Should parse 'imported but not used'");
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
        assert_eq!(p.file, Some(PathBuf::from("./main.go")));
        assert_eq!(p.line, Some(3));
        assert!(p.message.contains("imported but not used"));
    }

    #[test]
    fn test_parse_go_error_simplified_imported_but_not_used() {
        // Bare "imported but not used" without file prefix
        let err = "\"fmt\" imported but not used";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse bare 'imported but not used'"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
    }

    #[test]
    fn test_parse_go_error_simplified_imported_but_not_used_alt() {
        // Alternate bare format: "imported but not used: \"os\""
        let err = "imported but not used: \"os\"";
        let parsed = parse_go_error(err);
        assert!(
            parsed.is_some(),
            "Should parse bare 'imported but not used: \"os\"'"
        );
        let p = parsed.unwrap();
        assert_eq!(p.error_type, "unused_import");
    }

    #[test]
    fn test_parse_error_autodetect_imported_but_not_used() {
        let err = "./main.go:3:8: \"fmt\" imported but not used";
        let parsed = parse_error(err, None);
        assert!(
            parsed.is_some(),
            "Should auto-detect 'imported but not used' as Go"
        );
        let p = parsed.unwrap();
        assert_eq!(p.language, "go");
        assert_eq!(p.error_type, "unused_import");
    }
}
