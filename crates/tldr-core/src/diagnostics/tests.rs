//! Test module for diagnostics CLI command (Session 6 spec)
//!
//! These tests define expected behavior BEFORE implementation.
//! Tests are designed to FAIL until the modules are implemented.
//!
//! # Test Categories
//!
//! ## 1. Tool invocation tests
//! - pyright, ruff, tsc, eslint output parsing
//! - cargo check, clippy output parsing
//! - go vet, golangci-lint output parsing
//!
//! ## 2. Unified format tests
//! - All tools produce Diagnostic structs with consistent format
//! - Line/column conversion (0-indexed to 1-indexed)
//!
//! ## 3. Severity filtering tests
//! - Filter by Error, Warning, Information, Hint
//!
//! ## 4. Auto-detection tests
//! - Detect which tools are installed
//! - Select appropriate tools per language
//!
//! ## 5. Parallel execution tests
//! - Tools run concurrently
//! - Timeout handling
//!
//! ## 6. Output format tests
//! - SARIF output (valid schema)
//! - GitHub Actions workflow commands
//! - JSON and text formats
//!
//! ## 7. Exit code tests
//! - Non-zero on errors
//! - Configurable --strict mode
//!
//! Reference: session6-spec.md

use std::path::PathBuf;

use super::*;

// =============================================================================
// Test Fixture Setup Module
// =============================================================================

/// Test fixture utilities for diagnostics tests
pub mod fixtures {
    // -------------------------------------------------------------------------
    // Pyright JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample pyright --outputjson output
    pub const PYRIGHT_OUTPUT: &str = r#"{
  "version": "1.1.350",
  "time": "0.5s",
  "generalDiagnostics": [
    {
      "file": "src/auth.py",
      "severity": "error",
      "message": "Argument of type \"str\" cannot be assigned to parameter \"user_id\" of type \"int\"",
      "rule": "reportArgumentType",
      "range": {
        "start": {"line": 41, "character": 4},
        "end": {"line": 41, "character": 14}
      }
    },
    {
      "file": "src/utils.py",
      "severity": "warning",
      "message": "Variable \"x\" is not accessed",
      "rule": "reportUnusedVariable",
      "range": {
        "start": {"line": 10, "character": 0},
        "end": {"line": 10, "character": 1}
      }
    },
    {
      "file": "src/config.py",
      "severity": "information",
      "message": "Type of \"config\" is \"dict[str, Any]\"",
      "rule": "reportGeneralTypeIssues",
      "range": {
        "start": {"line": 5, "character": 0},
        "end": {"line": 5, "character": 6}
      }
    }
  ],
  "summary": {
    "filesAnalyzed": 3,
    "errorCount": 1,
    "warningCount": 1,
    "informationCount": 1,
    "timeInSec": 0.5
  }
}"#;

    // -------------------------------------------------------------------------
    // Ruff JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample ruff check --output-format json output
    pub const RUFF_OUTPUT: &str = r#"[
  {
    "cell": null,
    "code": "E501",
    "filename": "src/auth.py",
    "location": {"column": 1, "row": 58},
    "end_location": {"column": 121, "row": 58},
    "message": "Line too long (120 > 100 characters)",
    "noqa_row": 58,
    "url": "https://docs.astral.sh/ruff/rules/line-too-long"
  },
  {
    "cell": null,
    "code": "F401",
    "filename": "src/utils.py",
    "location": {"column": 1, "row": 1},
    "end_location": {"column": 15, "row": 1},
    "message": "'os' imported but unused",
    "noqa_row": 1,
    "url": "https://docs.astral.sh/ruff/rules/unused-import"
  }
]"#;

    // -------------------------------------------------------------------------
    // TSC Output Fixtures (Text, not JSON)
    // -------------------------------------------------------------------------

    /// Sample tsc --noEmit --pretty false output
    pub const TSC_OUTPUT: &str = r#"src/auth.ts(42,5): error TS2339: Property 'foo' does not exist on type 'Bar'.
src/auth.ts(58,10): error TS2345: Argument of type 'string' is not assignable to parameter of type 'number'.
src/utils.ts(15,1): warning TS6385: 'x' is deprecated."#;

    // -------------------------------------------------------------------------
    // ESLint JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample eslint -f json output
    pub const ESLINT_OUTPUT: &str = r#"[
  {
    "filePath": "/project/src/auth.ts",
    "messages": [
      {
        "ruleId": "no-unused-vars",
        "severity": 2,
        "message": "'x' is defined but never used.",
        "line": 10,
        "column": 5,
        "endLine": 10,
        "endColumn": 6
      },
      {
        "ruleId": "prefer-const",
        "severity": 1,
        "message": "'result' is never reassigned. Use 'const' instead.",
        "line": 25,
        "column": 5
      }
    ],
    "errorCount": 1,
    "warningCount": 1
  },
  {
    "filePath": "/project/src/utils.ts",
    "messages": [],
    "errorCount": 0,
    "warningCount": 0
  }
]"#;

    // -------------------------------------------------------------------------
    // Cargo/Clippy JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample cargo check --message-format=json output (one message per line)
    pub const CARGO_OUTPUT: &str = r#"{"reason":"compiler-message","package_id":"myproject 0.1.0","manifest_path":"/project/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"myproject","src_path":"/project/src/lib.rs"},"message":{"code":{"code":"unused_variables","explanation":null},"level":"warning","message":"unused variable: `x`","spans":[{"file_name":"src/main.rs","byte_start":100,"byte_end":101,"line_start":10,"line_end":10,"column_start":5,"column_end":6,"is_primary":true,"text":[{"text":"    let x = 5;","highlight_start":5,"highlight_end":6}],"label":"help: if this is intentional, prefix it with an underscore: `_x`"}],"children":[],"rendered":"warning: unused variable: `x`\n --> src/main.rs:10:5\n  |\n10 |     let x = 5;\n  |         ^ help: if this is intentional, prefix it with an underscore: `_x`\n  |\n  = note: `#[warn(unused_variables)]` on by default\n\n"}}
{"reason":"compiler-message","package_id":"myproject 0.1.0","manifest_path":"/project/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"myproject","src_path":"/project/src/lib.rs"},"message":{"code":{"code":"E0308","explanation":"Expected type did not match the received type."},"level":"error","message":"mismatched types","spans":[{"file_name":"src/lib.rs","byte_start":200,"byte_end":210,"line_start":20,"line_end":20,"column_start":10,"column_end":20,"is_primary":true,"text":[{"text":"    return \"hello\";","highlight_start":10,"highlight_end":17}],"label":"expected `i32`, found `&str`"}],"children":[],"rendered":"error[E0308]: mismatched types\n  --> src/lib.rs:20:10\n   |\n20 |     return \"hello\";\n   |            ^^^^^^^ expected `i32`, found `&str`\n\n"}}"#;

    // -------------------------------------------------------------------------
    // Go Vet JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample go vet -json output
    pub const GO_VET_OUTPUT: &str = r#"{"file":"/project/main.go","line":15,"col":5,"message":"printf: Sprintf format %d has arg x of wrong type string"}
{"file":"/project/utils.go","line":30,"col":1,"message":"unreachable code"}"#;

    // -------------------------------------------------------------------------
    // golangci-lint JSON Output Fixtures
    // -------------------------------------------------------------------------

    /// Sample golangci-lint run --out-format json output
    pub const GOLANGCI_LINT_OUTPUT: &str = r#"{
  "Issues": [
    {
      "FromLinter": "govet",
      "Text": "printf: Sprintf format %d has arg x of wrong type string",
      "Severity": "warning",
      "SourceLines": ["fmt.Sprintf(\"%d\", x)"],
      "Pos": {
        "Filename": "main.go",
        "Offset": 150,
        "Line": 15,
        "Column": 5
      }
    },
    {
      "FromLinter": "staticcheck",
      "Text": "this value of err is never used",
      "Severity": "error",
      "SourceLines": ["err := doSomething()"],
      "Pos": {
        "Filename": "utils.go",
        "Offset": 300,
        "Line": 25,
        "Column": 1
      }
    }
  ]
}"#;
}

// =============================================================================
// Tool Output Parsing Tests - Pyright
// =============================================================================

#[cfg(test)]
mod pyright_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::parse_pyright_output;

    /// Test parsing pyright JSON output
    /// Contract: Produces Diagnostic structs with correct file/line/severity
    #[test]
    fn parse_pyright_json() {
        let diagnostics = parse_pyright_output(PYRIGHT_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 3);

        let error = &diagnostics[0];
        assert_eq!(error.file, PathBuf::from("src/auth.py"));
        assert_eq!(error.line, 42); // 0-indexed to 1-indexed conversion
        assert_eq!(error.column, 5);
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.code, Some("reportArgumentType".to_string()));
        assert_eq!(error.source, "pyright");
    }

    /// Test pyright 0-indexed to 1-indexed line conversion
    /// Contract: Pyright uses 0-indexed lines, we convert to 1-indexed
    #[test]
    fn pyright_line_conversion() {
        // Pyright JSON has line 41 (0-indexed), should become 42 (1-indexed)
        let diagnostics = parse_pyright_output(PYRIGHT_OUTPUT).unwrap();
        assert_eq!(diagnostics[0].line, 42);
    }

    /// Test pyright severity mapping
    /// Contract: "error" -> Error, "warning" -> Warning, "information" -> Information
    #[test]
    fn pyright_severity_mapping() {
        let diagnostics = parse_pyright_output(PYRIGHT_OUTPUT).unwrap();

        let severities: Vec<_> = diagnostics.iter().map(|d| d.severity).collect();
        assert_eq!(
            severities,
            vec![Severity::Error, Severity::Warning, Severity::Information]
        );
    }
}

// =============================================================================
// Tool Output Parsing Tests - Ruff
// =============================================================================

#[cfg(test)]
mod ruff_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::parse_ruff_output;

    /// Test parsing ruff JSON output
    /// Contract: Produces Diagnostic structs with URL for documentation
    #[test]
    fn parse_ruff_json() {
        let diagnostics = parse_ruff_output(RUFF_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 2);

        let e501 = &diagnostics[0];
        assert_eq!(e501.file, PathBuf::from("src/auth.py"));
        assert_eq!(e501.line, 58);
        assert_eq!(e501.column, 1);
        assert_eq!(e501.code, Some("E501".to_string()));
        assert_eq!(e501.source, "ruff");
        assert_eq!(
            e501.url,
            Some("https://docs.astral.sh/ruff/rules/line-too-long".to_string())
        );
    }

    /// Test ruff warnings are mapped to Warning severity
    /// Contract: All ruff lint issues are mapped to Warning (not Error)
    #[test]
    fn ruff_severity_is_warning() {
        // Ruff lint issues are warnings by default
        let diagnostics = parse_ruff_output(RUFF_OUTPUT).unwrap();
        for diag in &diagnostics {
            assert_eq!(diag.severity, Severity::Warning);
        }
    }
}

// =============================================================================
// Tool Output Parsing Tests - TSC
// =============================================================================

#[cfg(test)]
mod tsc_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::{parse_tsc_text, tsc_output_regex};

    /// Test parsing tsc text output (no native JSON)
    /// Contract: Parses "file(line,col): severity TScode: message" format
    #[test]
    fn parse_tsc_output() {
        let diagnostics = parse_tsc_text(TSC_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 3);

        let ts2339 = &diagnostics[0];
        assert_eq!(ts2339.file, PathBuf::from("src/auth.ts"));
        assert_eq!(ts2339.line, 42);
        assert_eq!(ts2339.column, 5);
        assert_eq!(ts2339.severity, Severity::Error);
        assert_eq!(ts2339.code, Some("TS2339".to_string()));
        assert_eq!(ts2339.source, "tsc");
    }

    /// Test tsc regex pattern matching
    /// Contract: Regex matches "file(line,col): severity TScode: message"
    #[test]
    fn tsc_regex_pattern() {
        let line = "src/auth.ts(42,5): error TS2339: Property 'foo' does not exist on type 'Bar'.";
        let regex = tsc_output_regex();
        let captures = regex.captures(line).unwrap();

        assert_eq!(captures.get(1).unwrap().as_str(), "src/auth.ts");
        assert_eq!(captures.get(2).unwrap().as_str(), "42");
        assert_eq!(captures.get(3).unwrap().as_str(), "5");
        assert_eq!(captures.get(4).unwrap().as_str(), "error");
        assert_eq!(captures.get(5).unwrap().as_str(), "TS2339");
    }
}

// =============================================================================
// Tool Output Parsing Tests - ESLint
// =============================================================================

#[cfg(test)]
mod eslint_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::parse_eslint_output;

    /// Test parsing eslint JSON output
    /// Contract: Produces Diagnostic structs with severity 2=error, 1=warning
    #[test]
    fn parse_eslint_json() {
        let diagnostics = parse_eslint_output(ESLINT_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 2);

        let error = &diagnostics[0];
        assert_eq!(error.file, PathBuf::from("/project/src/auth.ts"));
        assert_eq!(error.line, 10);
        assert_eq!(error.column, 5);
        assert_eq!(error.severity, Severity::Error); // severity: 2
        assert_eq!(error.code, Some("no-unused-vars".to_string()));
        assert_eq!(error.source, "eslint");

        let warning = &diagnostics[1];
        assert_eq!(warning.severity, Severity::Warning); // severity: 1
    }

    /// Test eslint severity mapping
    /// Contract: ESLint severity 2=Error, 1=Warning
    #[test]
    fn eslint_severity_mapping() {
        let diagnostics = parse_eslint_output(ESLINT_OUTPUT).unwrap();

        assert_eq!(diagnostics[0].severity, Severity::Error); // severity: 2
        assert_eq!(diagnostics[1].severity, Severity::Warning); // severity: 1
    }

    /// Test eslint handles files with no issues
    /// Contract: Files with empty messages array produce no diagnostics
    #[test]
    fn eslint_empty_messages() {
        // ESLINT_OUTPUT has two files, one with no messages
        let diagnostics = parse_eslint_output(ESLINT_OUTPUT).unwrap();
        assert_eq!(diagnostics.len(), 2); // Only the ones with messages
    }
}

// =============================================================================
// Tool Output Parsing Tests - Cargo/Clippy
// =============================================================================

#[cfg(test)]
mod cargo_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::parse_cargo_output;

    /// Test parsing cargo check JSON output
    /// Contract: Parses NDJSON format with "reason": "compiler-message"
    #[test]
    fn parse_cargo_json() {
        let diagnostics = parse_cargo_output(CARGO_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 2);

        let warning = &diagnostics[0];
        assert_eq!(warning.file, PathBuf::from("src/main.rs"));
        assert_eq!(warning.line, 10);
        assert_eq!(warning.column, 5);
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.code, Some("unused_variables".to_string()));
        assert_eq!(warning.source, "cargo");

        let error = &diagnostics[1];
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.code, Some("E0308".to_string()));
    }

    /// Test cargo level mapping
    /// Contract: "warning" -> Warning, "error" -> Error
    #[test]
    fn cargo_level_mapping() {
        let diagnostics = parse_cargo_output(CARGO_OUTPUT).unwrap();

        assert_eq!(diagnostics[0].severity, Severity::Warning); // level: "warning"
        assert_eq!(diagnostics[1].severity, Severity::Error); // level: "error"
    }

    /// Test cargo NDJSON parsing (one JSON object per line)
    /// Contract: Each line is parsed independently
    #[test]
    fn cargo_ndjson_parsing() {
        // CARGO_OUTPUT has two lines, each is a separate JSON object
        let diagnostics = parse_cargo_output(CARGO_OUTPUT).unwrap();
        assert_eq!(diagnostics.len(), 2);
    }
}

// =============================================================================
// Tool Output Parsing Tests - Go
// =============================================================================

#[cfg(test)]
mod go_parser_tests {
    use super::fixtures::*;
    use super::*;
    use crate::diagnostics::parsers::{parse_go_vet_output, parse_golangci_lint_output};

    /// Test parsing go vet JSON output
    #[test]
    fn parse_go_vet_json() {
        let diagnostics = parse_go_vet_output(GO_VET_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 2);

        let diag = &diagnostics[0];
        assert_eq!(diag.file, PathBuf::from("/project/main.go"));
        assert_eq!(diag.line, 15);
        assert_eq!(diag.column, 5);
        assert_eq!(diag.source, "go vet");
    }

    /// Test parsing golangci-lint JSON output
    #[test]
    fn parse_golangci_lint_json() {
        let diagnostics = parse_golangci_lint_output(GOLANGCI_LINT_OUTPUT).unwrap();

        assert_eq!(diagnostics.len(), 2);

        let warning = &diagnostics[0];
        assert_eq!(warning.file, PathBuf::from("main.go"));
        assert_eq!(warning.severity, Severity::Warning);
        assert_eq!(warning.source, "govet");

        let error = &diagnostics[1];
        assert_eq!(error.severity, Severity::Error);
        assert_eq!(error.source, "staticcheck");
    }
}

// =============================================================================
// Severity Filtering Tests
// =============================================================================

#[cfg(test)]
mod severity_filtering_tests {
    use super::*;
    use crate::diagnostics::filter_diagnostics_by_severity;

    /// Test filtering diagnostics by minimum severity
    /// Contract: Only diagnostics >= min_severity are returned
    #[test]
    fn filter_by_severity() {
        let diagnostics = vec![
            Diagnostic {
                file: PathBuf::from("a.py"),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Error,
                message: "error".to_string(),
                code: None,
                source: "test".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("b.py"),
                line: 2,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Warning,
                message: "warning".to_string(),
                code: None,
                source: "test".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("c.py"),
                line: 3,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Hint,
                message: "hint".to_string(),
                code: None,
                source: "test".to_string(),
                url: None,
            },
        ];

        // With min_severity=Warning, only Error and Warning should pass
        let filtered = filter_diagnostics_by_severity(&diagnostics, Severity::Warning);
        assert_eq!(filtered.len(), 2);
    }

    /// Test filtering with --severity error
    /// Contract: Only errors are shown
    #[test]
    fn filter_errors_only() {
        let diagnostics = vec![
            Diagnostic {
                file: PathBuf::from("a.py"),
                line: 1,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Error,
                message: "error".to_string(),
                code: None,
                source: "test".to_string(),
                url: None,
            },
            Diagnostic {
                file: PathBuf::from("b.py"),
                line: 2,
                column: 1,
                end_line: None,
                end_column: None,
                severity: Severity::Warning,
                message: "warning".to_string(),
                code: None,
                source: "test".to_string(),
                url: None,
            },
        ];

        let filtered = filter_diagnostics_by_severity(&diagnostics, Severity::Error);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].severity, Severity::Error);
    }
}

// =============================================================================
// Auto-Detection Tests
// =============================================================================

#[cfg(test)]
mod auto_detection_tests {

    use crate::diagnostics::runner;
    use crate::types::Language;

    /// Test detecting available tools
    /// Contract: Only returns tools that are installed on PATH
    #[test]
    fn detect_available_tools() {
        // This test depends on what's installed, so we test the mechanism
        let tools = runner::detect_available_tools(Language::Python);

        // Tools should be a subset of all Python tools
        let all_python_tools = ["pyright", "ruff"];
        for tool in &tools {
            assert!(all_python_tools.contains(&tool.name));
        }
    }

    /// Test is_tool_available check
    /// Contract: Returns true if binary exists on PATH
    #[test]
    fn is_tool_available_check() {
        // "which" should always be available on Unix
        #[cfg(unix)]
        assert!(runner::is_tool_available("which"));
        // "nonexistent_tool_xyz" should not be available
        assert!(!runner::is_tool_available("nonexistent_tool_xyz"));
    }

    /// Test per-language tool selection
    /// Contract: Returns appropriate tools for each language
    #[test]
    fn per_language_tool_selection() {
        let python_tools = runner::tools_for_language(Language::Python);
        assert!(python_tools.iter().any(|t| t.name == "pyright"));
        assert!(python_tools.iter().any(|t| t.name == "ruff"));

        let ts_tools = runner::tools_for_language(Language::TypeScript);
        assert!(ts_tools.iter().any(|t| t.name == "tsc"));
        assert!(ts_tools.iter().any(|t| t.name == "eslint"));

        let rust_tools = runner::tools_for_language(Language::Rust);
        assert!(rust_tools.iter().any(|t| t.name == "cargo check"));
        assert!(rust_tools.iter().any(|t| t.name == "clippy"));
    }
}

// =============================================================================
// Parallel Execution Tests
// =============================================================================

#[cfg(test)]
mod parallel_execution_tests {

    /// Test tools run in parallel
    /// Contract: Total time < sum of individual tool times
    #[test]
    #[ignore = "Parallel execution not yet implemented"]
    fn tools_run_in_parallel() {
        // Create two mock tools that each take 100ms
        // let start = Instant::now();
        // let report = run_diagnostics_with_tools(
        //     Path::new("."),
        //     &[mock_slow_tool("tool1", 100), mock_slow_tool("tool2", 100)],
        //     200,  // timeout
        // ).unwrap();
        // let elapsed = start.elapsed().as_millis();
        //
        // // Parallel: should be ~100ms, not 200ms
        // assert!(elapsed < 150, "Expected parallel execution, took {}ms", elapsed);

        todo!("Implement tools_run_in_parallel test");
    }

    /// Test tool timeout handling
    /// Contract: Hung tools are killed after timeout
    #[test]
    #[ignore = "Timeout handling not yet implemented"]
    fn tool_timeout_handling() {
        // Create a tool that hangs for 10 seconds
        // let report = run_diagnostics_with_tools(
        //     Path::new("."),
        //     &[mock_slow_tool("hanging_tool", 10000)],
        //     1,  // 1 second timeout
        // ).unwrap();
        //
        // // Tool should be marked as failed
        // assert_eq!(report.tools_run.len(), 1);
        // assert!(!report.tools_run[0].success);
        // assert!(report.tools_run[0].error.is_some());

        todo!("Implement tool_timeout_handling test");
    }
}

// =============================================================================
// SARIF Output Tests
// =============================================================================

#[cfg(test)]
mod sarif_output_tests {

    use super::*;

    /// Test SARIF output has valid schema
    /// Contract: Output matches SARIF 2.1.0 schema
    #[test]
    #[ignore = "SARIF output not yet implemented"]
    fn sarif_valid_schema() {
        let _report = DiagnosticsReport {
            diagnostics: vec![Diagnostic {
                file: PathBuf::from("src/auth.py"),
                line: 42,
                column: 5,
                end_line: None,
                end_column: None,
                severity: Severity::Error,
                message: "Type error".to_string(),
                code: Some("reportArgumentType".to_string()),
                source: "pyright".to_string(),
                url: None,
            }],
            summary: DiagnosticsSummary {
                errors: 1,
                warnings: 0,
                info: 0,
                hints: 0,
                total: 1,
            },
            tools_run: vec![],
            files_analyzed: 1,
        };

        // let sarif = to_sarif(&report);
        //
        // // Check schema URL
        // assert_eq!(sarif["$schema"], SARIF_SCHEMA);
        // assert_eq!(sarif["version"], "2.1.0");
        //
        // // Check runs array
        // let runs = sarif["runs"].as_array().unwrap();
        // assert_eq!(runs.len(), 1);
        //
        // // Check results
        // let results = runs[0]["results"].as_array().unwrap();
        // assert_eq!(results.len(), 1);
        // assert_eq!(results[0]["ruleId"], "reportArgumentType");
        // assert_eq!(results[0]["level"], "error");

        todo!("Implement sarif_valid_schema test");
    }

    /// Test SARIF level mapping
    /// Contract: Error -> "error", Warning -> "warning", Info -> "note", Hint -> "none"
    #[test]
    #[ignore = "SARIF level mapping not yet implemented"]
    fn sarif_level_mapping() {
        // assert_eq!(severity_to_sarif_level(Severity::Error), "error");
        // assert_eq!(severity_to_sarif_level(Severity::Warning), "warning");
        // assert_eq!(severity_to_sarif_level(Severity::Information), "note");
        // assert_eq!(severity_to_sarif_level(Severity::Hint), "none");

        todo!("Implement sarif_level_mapping test");
    }
}

// =============================================================================
// GitHub Actions Output Tests
// =============================================================================

#[cfg(test)]
mod github_actions_tests {
    use super::*;

    /// Test GitHub Actions workflow command format
    /// Contract: ::severity file=path,line=N,col=N::message
    #[test]
    #[ignore = "GitHub Actions output not yet implemented"]
    fn github_actions_format() {
        let _diag = Diagnostic {
            file: PathBuf::from("src/auth.py"),
            line: 42,
            column: 5,
            end_line: None,
            end_column: None,
            severity: Severity::Error,
            message: "Type error".to_string(),
            code: None,
            source: "pyright".to_string(),
            url: None,
        };

        // let output = format_github_actions(&diag);
        // assert_eq!(output, "::error file=src/auth.py,line=42,col=5::Type error");

        todo!("Implement github_actions_format test");
    }

    /// Test GitHub Actions severity keywords
    /// Contract: Error -> "error", Warning -> "warning", others -> "notice"
    #[test]
    #[ignore = "GitHub Actions severity not yet implemented"]
    fn github_actions_severity() {
        // assert_eq!(severity_to_github_actions(Severity::Error), "error");
        // assert_eq!(severity_to_github_actions(Severity::Warning), "warning");
        // assert_eq!(severity_to_github_actions(Severity::Information), "notice");
        // assert_eq!(severity_to_github_actions(Severity::Hint), "notice");

        todo!("Implement github_actions_severity test");
    }
}

// =============================================================================
// Exit Code Tests
// =============================================================================

#[cfg(test)]
mod exit_code_tests {
    use super::*;

    /// Test exit code 0 when no errors
    /// Contract: Success when only warnings (without --strict)
    #[test]
    #[ignore = "Exit codes not yet implemented"]
    fn exit_code_success_with_warnings() {
        let _summary = DiagnosticsSummary {
            errors: 0,
            warnings: 5,
            info: 2,
            hints: 1,
            total: 8,
        };

        // let code = compute_exit_code(&summary, false);  // strict=false
        // assert_eq!(code, 0);

        todo!("Implement exit_code_success_with_warnings test");
    }

    /// Test exit code 1 when errors exist
    /// Contract: Non-zero when errors found
    #[test]
    #[ignore = "Exit codes not yet implemented"]
    fn exit_code_failure_with_errors() {
        let _summary = DiagnosticsSummary {
            errors: 1,
            warnings: 0,
            info: 0,
            hints: 0,
            total: 1,
        };

        // let code = compute_exit_code(&summary, false);
        // assert_eq!(code, 1);

        todo!("Implement exit_code_failure_with_errors test");
    }

    /// Test exit code 1 with --strict and warnings
    /// Contract: Non-zero when warnings exist in strict mode
    #[test]
    #[ignore = "Exit codes not yet implemented"]
    fn exit_code_strict_with_warnings() {
        let _summary = DiagnosticsSummary {
            errors: 0,
            warnings: 1,
            info: 0,
            hints: 0,
            total: 1,
        };

        // let code = compute_exit_code(&summary, true);  // strict=true
        // assert_eq!(code, 1);

        todo!("Implement exit_code_strict_with_warnings test");
    }
}

// =============================================================================
// Edge Case Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    /// Test handling when no tools are installed
    /// Contract: Returns error with install suggestions
    #[test]
    #[ignore = "No tools installed handling not yet implemented"]
    fn no_tools_installed() {
        // Mock scenario where no tools are available
        // let result = run_diagnostics(
        //     Path::new("."),
        //     Language::Python,
        //     DiagnosticsOptions::default(),
        // );
        //
        // assert!(result.is_err());
        // let err = result.unwrap_err();
        // assert!(err.to_string().contains("No diagnostic tools available"));

        todo!("Implement no_tools_installed test");
    }

    /// Test handling when tool crashes
    /// Contract: Captures stderr, marks failed, continues with other tools
    #[test]
    #[ignore = "Tool crash handling not yet implemented"]
    fn tool_crash_handling() {
        // let report = run_diagnostics_with_tools(
        //     Path::new("."),
        //     &[mock_crashing_tool(), mock_working_tool()],
        //     60,
        // ).unwrap();
        //
        // assert_eq!(report.tools_run.len(), 2);
        //
        // let crashed = report.tools_run.iter().find(|t| !t.success).unwrap();
        // assert!(crashed.error.is_some());
        //
        // let working = report.tools_run.iter().find(|t| t.success).unwrap();
        // assert!(working.error.is_none());

        todo!("Implement tool_crash_handling test");
    }

    /// Test empty project
    /// Contract: Returns empty diagnostics (success)
    #[test]
    #[ignore = "Empty project handling not yet implemented"]
    fn empty_project() {
        let _test_dir = tempfile::tempdir().unwrap();

        // let report = run_diagnostics(
        //     test_dir.path(),
        //     Language::Python,
        //     DiagnosticsOptions::default(),
        // ).unwrap();
        //
        // assert!(report.diagnostics.is_empty());
        // assert_eq!(report.summary.total, 0);

        todo!("Implement empty_project test");
    }

    /// Test invalid path
    /// Contract: Returns PathNotFound error
    #[test]
    #[ignore = "Invalid path handling not yet implemented"]
    fn invalid_path() {
        // let result = run_diagnostics(
        //     Path::new("/nonexistent/path"),
        //     Language::Python,
        //     DiagnosticsOptions::default(),
        // );
        //
        // assert!(result.is_err());

        todo!("Implement invalid_path test");
    }
}

// =============================================================================
// Summary Tests
// =============================================================================

#[cfg(test)]
mod summary_tests {
    use super::*;

    /// Test summary counts are correct
    /// Contract: Summary accurately reflects diagnostic counts
    #[test]
    #[ignore = "Summary computation not yet implemented"]
    fn summary_counts_correct() {
        let _diagnostics = [
            Diagnostic {
                severity: Severity::Error,
                ..Default::default()
            },
            Diagnostic {
                severity: Severity::Error,
                ..Default::default()
            },
            Diagnostic {
                severity: Severity::Warning,
                ..Default::default()
            },
            Diagnostic {
                severity: Severity::Information,
                ..Default::default()
            },
            Diagnostic {
                severity: Severity::Hint,
                ..Default::default()
            },
        ];

        // let summary = compute_summary(&diagnostics);
        //
        // assert_eq!(summary.errors, 2);
        // assert_eq!(summary.warnings, 1);
        // assert_eq!(summary.info, 1);
        // assert_eq!(summary.hints, 1);
        // assert_eq!(summary.total, 5);

        todo!("Implement summary_counts_correct test");
    }
}

// Provide a default for Diagnostic to make test setup easier
impl Default for Diagnostic {
    fn default() -> Self {
        Self {
            file: PathBuf::from("test.py"),
            line: 1,
            column: 1,
            end_line: None,
            end_column: None,
            severity: Severity::Error,
            message: "test".to_string(),
            code: None,
            source: "test".to_string(),
            url: None,
        }
    }
}
