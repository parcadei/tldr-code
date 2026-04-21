# CLI Quality Commands - Known Bugs

This document tracks bugs discovered during the CLI quality commands test suite development.
**DO NOT FIX THESE BUGS** - they are documented here for future reference.

## Bug Summary Table

| ID | File | Function | Bug Description | Severity | Test Blocked |
|----|------|----------|-----------------|----------|--------------|
| 1 | smells.rs | `run()` | Daemon routing may return cached results without proper format conversion | Medium | N/A - intermittent |
| 2 | health.rs | `run()` | Quick mode doesn't validate --detail compatibility before running | Low | N/A - caught by validate() |
| 3 | debt.rs | `run()` | Category validation bypassed by clap value_parser - error message unclear | Low | `test_debt_invalid_category` |
| 4 | churn.rs | `analyze_churn()` | Error message format inconsistent - "NotGitRepository" vs "not a git repository" | Low | N/A |
| 5 | maintainability.rs | `run()` | validate_file_path doesn't return specific error code for nonexistent files | Low | N/A |
| 6 | coverage.rs | `run()` | No validation for negative threshold values | Low | N/A |
| 7 | hotspots.rs | `run()` | Error message format inconsistent with other commands for non-git repos | Low | N/A |
| 8 | debt.rs | `parse_language()` | Dead code - function marked with #[allow(dead_code)] but never used | Info | N/A |
| 9 | smells.rs | `run()` | Nonexistent path returns success with empty results instead of error | Medium | `test_smells_nonexistent_path` |
| 10 | health.rs | `detail_parser()` | Help text lists "dead_code" but actual valid value is "dead" | Medium | N/A |
| 11 | health.rs | `detail_parser()` | Help text lists "martin" but actual valid value is "metrics" | Medium | N/A |

---

## Detailed Bug Reports

### Issue 1: Smells command - Daemon routing format conversion

**File:** `crates/tldr-cli/src/commands/smells.rs`  
**Function:** `SmellsArgs::run()`  
**Severity:** Medium

**Description:**
When the daemon returns cached results, the format conversion logic may not properly handle text format requests. The daemon returns structured data that gets formatted as text, but there may be edge cases where format consistency is not maintained.

**Code Location:**
```rust
// Lines 91-105 in smells.rs
if let Some(report) = try_daemon_route::<SmellsReport>(...) {
    if writer.is_text() {
        let text = format_smells_text(&report);
        writer.write_text(&text)?;
        return Ok(());
    } else {
        writer.write(&report)?;
        return Ok(());
    }
}
```

**Severity:** Medium - Potential format inconsistency with daemon mode

---

### Issue 2: Health command - Validation order

**File:** `crates/tldr-cli/src/commands/health.rs`  
**Function:** `HealthArgs::run()`  
**Severity:** Low

**Description:**
The validate() function is called after creating the OutputWriter, which means resources are allocated before validation. This is a minor inefficiency but doesn't affect correctness.

**Code Location:**
```rust
// Lines 114-116 in health.rs
pub fn run(&self, format: OutputFormat, quiet: bool, lang: Option<Language>) -> Result<()> {
    self.validate()?;  // Called after writer creation would be more efficient
    let writer = OutputWriter::new(format, quiet);
```

**Severity:** Low - Code organization issue

---

### Issue 3: Debt command - Category validation

**File:** `crates/tldr-cli/src/commands/debt.rs`  
**Function:** `DebtArgs::run()`  
**Test:** `test_debt_invalid_category`

**Description:**
The category argument uses clap's value_parser to validate input against allowed values. However, when an invalid category is provided, the error message comes from clap rather than the custom validation logic in the run() function. This leads to inconsistent error handling between CLI parsing and runtime validation.

**Expected Behavior:**
When an invalid category is provided, clap should reject it at parse time with a clear error message.

**Actual Behavior:**
The clap validation works, but the error message format is different from other validation errors in the codebase.

**Code Location:**
```rust
// Lines 36-37 in debt.rs
#[arg(short = 'c', long, value_parser = ["reliability", "security", "maintainability", "efficiency", "changeability", "testability"])]
pub category: Option<String>,
```

**Severity:** Low - Validation inconsistency

---

### Issue 4: Churn command - Error message format inconsistency

**File:** `crates/tldr-cli/src/commands/churn.rs`  
**Function:** `analyze_churn()`  
**Severity:** Low

**Description:**
When the churn command is run on a non-git repository, the error message format may vary depending on how the error is propagated. The error type is `ChurnError::NotGitRepository` but the display message may not match other commands' error formats.

**Code Location:**
```rust
// Lines 101-105 in churn.rs
if !is_git_repository(path)? {
    return Err(ChurnError::NotGitRepository {
        path: path.clone(),
    });
}
```

**Severity:** Low - Error message inconsistency

---

### Issue 5: Maintainability command - Error handling

**File:** `crates/tldr-cli/src/commands/maintainability.rs`  
**Function:** `MaintainabilityArgs::run()`  
**Severity:** Low

**Description:**
The maintainability command uses `validate_file_path()` which returns a generic anyhow error instead of a specific error code for nonexistent files. This is inconsistent with other commands that explicitly check path existence and return specific error messages.

**Code Location:**
```rust
// Lines 39-42 in maintainability.rs
let validated_path = validate_file_path(
    self.path.to_str().unwrap_or_default(),
    None,
)?;
```

**Severity:** Low - Error handling inconsistency

---

### Issue 6: Coverage command - Threshold validation

**File:** `crates/tldr-cli/src/commands/coverage.rs`  
**Function:** `CoverageArgs`  
**Severity:** Low

**Description:**
The threshold parameter accepts any f64 value without validation. Negative values or values over 100% are accepted but may produce confusing results.

**Code Location:**
```rust
// Lines 90-91 in coverage.rs
#[arg(long, default_value = "80.0")]
pub threshold: f64,
```

**Severity:** Low - Input validation issue

---

### Issue 7: Hotspots command - Error message format

**File:** `crates/tldr-cli/src/commands/hotspots.rs`  
**Function:** `HotspotsArgs::run()`  
**Severity:** Low

**Description:**
The hotspots command uses the core `analyze_hotspots` function which returns its own error type. The error message format for non-git repositories may differ from the churn command despite similar functionality.

**Code Location:**
```rust
// Lines 84 in hotspots.rs
let report = analyze_hotspots(&self.path, &options)?;
```

**Severity:** Low - Error message inconsistency

---

### Issue 8: Debt command - Dead code

**File:** `crates/tldr-cli/src/commands/debt.rs`  
**Function:** `parse_language()`  
**Severity:** Info

**Description:**
The `parse_language()` function is marked with `#[allow(dead_code)]` and is not used anywhere in the module. The language parsing is handled by the global CLI flag instead.

**Code Location:**
```rust
// Lines 107-130 in debt.rs
#[allow(dead_code)]
fn parse_language(lang: &str) -> Option<Language> {
    // ... function body
}
```

**Severity:** Info - Code cleanup opportunity

---

### Issue 9: Smells command - Nonexistent path handling

**File:** `crates/tldr-cli/src/commands/smells.rs`  
**Function:** `SmellsArgs::run()`  
**Test:** `test_smells_nonexistent_path`

**Description:**
When the smells command is given a nonexistent path, it returns success with an empty result instead of failing with an error. This is inconsistent with other commands like `health` and `debt` which properly validate path existence.

**Expected Behavior:**
- Exit code: non-zero
- Error message: "Path not found" or similar

**Actual Behavior:**
- Exit code: 0 (success)
- Returns empty JSON: `{"smells": [], "files_scanned": 0, ...}`

**Code Location:**
```rust
// Lines 87-90 in smells.rs
pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
    let writer = OutputWriter::new(format, quiet);
    // Missing: path existence validation
```

**Severity:** Medium - Missing path validation

---

### Issue 10: Health command - Detail option mismatch (dead_code vs dead)

**File:** `crates/tldr-cli/src/commands/health.rs`  
**Function:** `detail_parser()`  

**Description:**
The help text and detail_parser list "dead_code" as a valid detail option, but the actual sub-analysis name in the report is "dead". Using "dead_code" results in an error.

**Expected Behavior:**
`--detail dead_code` should work as documented

**Actual Behavior:**
`--detail dead_code` fails with: "Sub-analysis 'dead_code' not found. Available: complexity, cohesion, dead, metrics, coupling, similar"

**Code Location:**
```rust
// Lines 72-82 in health.rs - lists "dead_code"
fn detail_parser(s: &str) -> Result<String, String> {
    let valid = [
        "complexity",
        "cohesion",
        "dead_code",  // <-- This is wrong, should be "dead"
        "martin",     // <-- This is wrong, should be "metrics"
        ...
    ];
```

**Severity:** Medium - Documentation/code mismatch

---

### Issue 11: Health command - Detail option mismatch (martin vs metrics)

**File:** `crates/tldr-cli/src/commands/health.rs`  
**Function:** `detail_parser()`  

**Description:**
Similar to Issue 10, the help text lists "martin" as a valid detail option, but the actual sub-analysis name is "metrics".

**Expected Behavior:**
`--detail martin` should work as documented

**Actual Behavior:**
`--detail martin` fails with: "Sub-analysis 'martin' not found. Available: complexity, cohesion, dead, metrics, coupling, similar"

**Code Location:**
Same as Issue 10 - the detail_parser lists "martin" but the actual analysis is named "metrics".

**Severity:** Medium - Documentation/code mismatch

---

## Test Coverage Summary

### Commands Covered
- `smells` - Code smell detection (God Class, Long Method, etc.)
- `health` - Comprehensive code health dashboard
- `debt` - Technical debt analysis using SQALE method
- `churn` - Git-based file churn analysis
- `maintainability` - Maintainability Index calculation
- `coverage` - Parse and report code coverage from existing reports
- `hotspots` - Identify high-risk code regions (churn x complexity)

### Test Categories
1. **Help Tests** - Verify help text for all commands
2. **Basic Functionality** - Core command operations
3. **CLI Arguments** - All command-line options and flags
4. **Output Formats** - JSON, text, compact formats
5. **Error Handling** - Missing files, invalid inputs
6. **Global Options** - --quiet, --verbose, --format, --lang
7. **Command-Specific Options** - Each command's unique flags
8. **Validation** - Input validation and conflict detection

### Ignored Tests
Tests marked with `#[ignore]` are blocked by the bugs documented above:
- `test_debt_invalid_category` - Blocked by Issue 3 (clap validation vs runtime validation)
- `test_smells_nonexistent_path` - Blocked by Issue 9 (smells doesn't validate path existence)

To run ignored tests: `cargo test --test cli_quality_tests -- --ignored`

---

## Commands Overview

### Smells Command
Detects code smells: God Class, Long Method, Long Parameter List, Feature Envy, Data Clumps
- Args: `--threshold`, `--smell-type`, `--suggest`
- Thresholds: strict, default, relaxed

### Health Command
Aggregates multiple analyzers: complexity, cohesion, dead, metrics, coupling, similar
- Args: `--detail`, `--quick`, `--preset`
- Detail options: complexity, cohesion, dead, metrics, coupling, similar, all
- Conflicts: --quick + --detail=coupling/similar
- **BUG**: Help text incorrectly lists "dead_code" and "martin" (see Issues 10, 11)

### Debt Command
SQALE-based technical debt analysis
- Args: `--category`, `--top`, `--min-debt`, `--hourly-rate`
- Categories: reliability, security, maintainability, efficiency, changeability, testability

### Churn Command
Git-based file churn analysis
- Args: `--days`, `--top`, `--exclude`, `--authors`, `--hotspots`
- Requires: Git repository

### Maintainability Command
Maintainability Index calculation with optional Halstead metrics
- Args: `--halstead`, `--lang`

### Coverage Command
Parses existing coverage reports (Cobertura, LCOV, coverage.py)
- Args: `--report-format`, `--threshold`, `--by-file`, `--uncovered`, `--uncovered-only`, `--filter`, `--sort`
- Formats: cobertura, lcov, coveragepy, auto

### Hotspots Command
Identifies high-risk code regions (churn x complexity)
- Args: `--days`, `--top`, `--by-function`, `--show-trend`, `--min-commits`, `--exclude`, `--threshold`, `--since`
- Requires: Git repository
