# CLI Basic Commands - Known Bugs

This document tracks bugs discovered during the CLI basic commands test suite development.
**DO NOT FIX THESE BUGS** - they are documented here for future reference.

## Bug Summary Table

| ID | File | Function | Bug Description | Severity | Test Blocked |
|----|------|----------|-----------------|----------|--------------|
| 1 | imports.rs | `run()` | Unsupported language returns generic error instead of exit code 11 | Medium | `test_imports_unsupported_language` |
| 2 | extract.rs | `run()` | Unsupported language returns generic error instead of exit code 11 | Medium | `test_extract_unsupported_language` |
| 3 | importers.rs | `run()` | Text format not implemented (falls back to JSON) | Low | `test_importers_text_format` |
| 4 | importers.rs | N/A | Invalid language argument not validated | Low | `test_importers_invalid_language` |
| 5 | output.rs | `write_text()` | Quiet flag suppresses all text output instead of just progress | High | `test_tree_text_format`, `test_structure_text_output` |

---

## Detailed Bug Reports

### Issue 1: imports command - Unsupported language error handling

**File:** `crates/tldr-cli/src/commands/imports.rs`  
**Function:** `ImportsArgs::run()`  
**Test:** `test_imports_unsupported_language`

**Description:**
When an unsupported file extension is provided (e.g., `.xyz`), the `imports` command returns a generic error with exit code 1 instead of the expected exit code 11 (for unsupported language).

**Expected Behavior:**
- Exit code: 11
- Error message: "Unsupported language" or similar

**Actual Behavior:**
- Exit code: 1
- Generic error message

**Code Location:**
```rust
// Line 47-50 in imports.rs
let language = detect_or_parse_language(
    self.lang.as_ref().map(|l| l.as_str()),
    &self.file,
)?;
```

The `detect_or_parse_language` function returns a generic error instead of mapping to `TldrError::UnsupportedLanguage`.

**Severity:** Medium - Error handling inconsistency

---

### Issue 2: extract command - Unsupported language error handling

**File:** `crates/tldr-cli/src/commands/extract.rs`  
**Function:** `ExtractArgs::run()`  
**Test:** `test_extract_unsupported_language`

**Description:**
Similar to Issue 1, the `extract` command returns a generic error instead of exit code 11 when given an unsupported file extension.

**Expected Behavior:**
- Exit code: 11
- Error message: "Unsupported language"

**Actual Behavior:**
- Exit code: 1
- Generic error message

**Code Location:**
```rust
// Line 51 in extract.rs
let result = extract_file(&self.file, None)?;
```

The `extract_file` function handles language detection internally but doesn't return a specific error code for unsupported languages.

**Severity:** Medium - Error handling inconsistency

---

### Issue 3: importers command - Text format not implemented

**File:** `crates/tldr-cli/src/commands/importers.rs`  
**Function:** `ImportersArgs::run()`  
**Test:** `test_importers_text_format`

**Description:**
The `importers` command accepts the `-f text` flag but outputs JSON format instead of human-readable text. The `OutputWriter::is_text()` check is not being used to format output.

**Expected Behavior:**
When `-f text` is specified, output should be human-readable text format similar to other commands.

**Actual Behavior:**
Always outputs JSON regardless of format flag.

**Code Location:**
```rust
// Line 61-64 in importers.rs
let result = find_importers(&self.path, &self.module, language)?;

// Output based on format
writer.write(&result)?;
```

The command doesn't check `writer.is_text()` and format output accordingly.

**Severity:** Low - Feature not implemented

---

### Issue 4: importers command - Invalid language not validated

**File:** `crates/tldr-cli/src/commands/importers.rs`  
**Function:** N/A (clap parsing)  
**Test:** `test_importers_invalid_language`

**Description:**
The `--lang` argument accepts any string value without validation. Invalid language strings like "invalid_language" are passed through to the core functions which may not handle them gracefully.

**Expected Behavior:**
- Exit code: non-zero
- Error message: "Invalid language" or similar

**Actual Behavior:**
Command may succeed or fail with unclear error depending on how the core handles invalid languages.

**Code Location:**
```rust
// Line 28-30 in importers.rs
/// Programming language (auto-detected from directory if not specified)
#[arg(long, short = 'l')]
pub lang: Option<Language>,
```

Clap's `Language` enum parsing accepts unknown strings without proper validation.

**Severity:** Low - Input validation issue

---

### Issue 5: output.rs - Quiet flag suppresses text output completely

**File:** `crates/tldr-cli/src/output.rs`  
**Function:** `OutputWriter::write_text()`  
**Tests:** `test_tree_text_format`, `test_structure_text_output`

**Description:**
The `write_text()` method returns early without writing any output when the `quiet` flag is set. This is incorrect behavior - the quiet flag should only suppress progress/error messages (written to stderr), not the actual command output (written to stdout).

**Expected Behavior:**
Text format output should be displayed even when `-q` flag is used. Only progress messages should be suppressed.

**Actual Behavior:**
When `-q` is combined with `-f text`, no output is produced.

**Code Location:**
```rust
// Lines 72-80 in output.rs
pub fn write_text(&self, text: &str) -> io::Result<()> {
    if self.quiet {
        return Ok(());  // BUG: This suppresses all text output!
    }
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    writeln!(handle, "{}", text)?;
    Ok(())
}
```

The quiet check should NOT apply to the main command output - only to progress messages. The `write()` method for JSON doesn't have this issue because it doesn't check the quiet flag.

**Severity:** High - Breaks text format functionality when quiet mode is enabled

---

## Test Coverage Summary

### Commands Covered
- `tree` - File tree command
- `structure` - Structure extraction command
- `imports` - Parse import statements
- `extract` - Extract complete module info
- `importers` - Find files that import a module

### Test Categories
1. **Help Tests** - Verify help text for all commands
2. **Basic Functionality** - Core command operations
3. **CLI Arguments** - All command-line options and flags
4. **Output Formats** - JSON, text, compact formats
5. **Error Handling** - Missing files, unsupported languages
6. **Aliases** - Short command aliases (t, s, e)
7. **Global Options** - --quiet, --verbose, --format, --lang
8. **Schema Validation** - Output structure verification

### Ignored Tests
Tests marked with `#[ignore]` are blocked by the bugs documented above:
- `test_imports_unsupported_language` - Blocked by Issue 1
- `test_extract_unsupported_language` - Blocked by Issue 2
- `test_importers_text_format` - Blocked by Issue 3
- `test_importers_invalid_language` - Blocked by Issue 4
- `test_tree_text_format` - Blocked by Issue 5
- `test_structure_text_output` - Blocked by Issue 5

To run ignored tests: `cargo test --test cli_basic_tests -- --ignored`
