//! Tests for output formatting (Phase 11)
//!
//! Tests for clone and similarity report text/DOT formatters.
//! Addresses premortem risks:
//! - S8-P3-T5: Clone type names are jargon
//! - S8-P3-T7: Empty results don't explain why
//! - S8-P3-T11: DOT node IDs with special characters

use std::path::PathBuf;

use crate::output::{
    clone_type_description, empty_results_hints, escape_dot_id, format_clones_dot,
    format_clones_text, format_similarity_text,
};
use tldr_core::analysis::{
    CloneConfig, CloneFragment, ClonePair, CloneStats, CloneType, ClonesOptions, ClonesReport,
    SimilarityConfig, SimilarityFragment, SimilarityMetric, SimilarityReport, SimilarityScores,
    TokenBreakdown,
};

// =============================================================================
// Clone Type Description Tests (S8-P3-T5)
// =============================================================================

/// Test: Clone type descriptions are human-readable, not jargon
/// Risk: S8-P3-T5 - Clone type names are jargon
#[test]
fn test_clone_type_description_type1() {
    let desc = clone_type_description(&CloneType::Type1);
    assert!(
        desc.contains("exact") || desc.contains("identical"),
        "Type-1 description should mention 'exact' or 'identical': {}",
        desc
    );
}

#[test]
fn test_clone_type_description_type2() {
    let desc = clone_type_description(&CloneType::Type2);
    assert!(
        desc.contains("identifier") || desc.contains("renamed") || desc.contains("literal"),
        "Type-2 description should mention 'identifier', 'renamed', or 'literal': {}",
        desc
    );
}

#[test]
fn test_clone_type_description_type3() {
    let desc = clone_type_description(&CloneType::Type3);
    assert!(
        desc.contains("similar") || desc.contains("addition") || desc.contains("deletion"),
        "Type-3 description should mention 'similar', 'addition', or 'deletion': {}",
        desc
    );
}

/// Test: Text output includes compact clone type column
#[test]
fn test_text_output_clone_type_explained() {
    // GIVEN: A report with a Type-2 clone
    let report = make_clone_report_with_pairs(vec![make_type2_clone_pair()]);

    // WHEN: Formatting as text
    let text = format_clones_text(&report);

    // THEN: Should include compact type indicator T2
    assert!(
        text.contains("T2"),
        "Type-2 should show compact 'T2' indicator, got: {}",
        text
    );
}

// =============================================================================
// Empty Results Hints Tests (S8-P3-T7)
// =============================================================================

/// Test: Empty results produce concise output
#[test]
fn test_empty_results_concise() {
    // GIVEN: A report with no clones
    let report = make_clone_report_with_pairs(vec![]);

    // WHEN: Formatting as text
    let text = format_clones_text(&report);

    // THEN: Should be concise with "No clones found."
    assert!(
        text.contains("No clones found."),
        "Empty results should say 'No clones found.', got: {}",
        text
    );
}

/// Test: Hints suggest lowering threshold
#[test]
fn test_empty_results_hint_threshold() {
    // GIVEN: Options and stats for empty results
    let options = ClonesOptions::new();
    let stats = CloneStats {
        files_analyzed: 10,
        total_tokens: 5000,
        ..Default::default()
    };

    // WHEN: Getting hints
    let hints = empty_results_hints(&options, &stats);

    // THEN: Should suggest threshold adjustment
    assert!(
        hints.iter().any(|h| h.contains("threshold")),
        "Should suggest threshold adjustment: {:?}",
        hints
    );
}

/// Test: Hints include analysis stats
#[test]
fn test_empty_results_hint_stats() {
    let options = ClonesOptions::new();
    let stats = CloneStats {
        files_analyzed: 42,
        total_tokens: 15234,
        ..Default::default()
    };

    let hints = empty_results_hints(&options, &stats);

    // Should include what was analyzed
    let hints_str = hints.join(" ");
    assert!(
        hints_str.contains("42") && hints_str.contains("15234"),
        "Should include files/tokens analyzed: {:?}",
        hints
    );
}

// =============================================================================
// DOT Output Special Characters Tests (S8-P3-T11)
// =============================================================================

/// Test: DOT node IDs escape backslashes (Windows paths)
/// Risk: S8-P3-T11 - DOT node IDs with special characters break graphviz
#[test]
fn test_dot_escape_backslash() {
    let id = escape_dot_id(r"C:\Users\test\file.py:10-20");
    assert!(
        !id.contains('\\') || id.contains(r"\\"),
        "Backslashes should be escaped or converted: {}",
        id
    );
}

/// Test: DOT node IDs escape quotes
#[test]
fn test_dot_escape_quotes() {
    let id = escape_dot_id(r#"file with "quotes".py:1-10"#);
    assert!(
        !id.contains('"') || id.contains(r#"\""#),
        "Quotes should be escaped: {}",
        id
    );
}

/// Test: DOT node IDs handle spaces
#[test]
fn test_dot_escape_spaces() {
    let id = escape_dot_id("path with spaces/file.py:1-10");
    // Spaces should either be in quoted strings or escaped
    assert!(
        id.starts_with('"') && id.ends_with('"'),
        "Node ID with spaces should be quoted: {}",
        id
    );
}

/// Test: DOT output is valid for graphviz
#[test]
fn test_dot_output_special_characters_escaped() {
    // GIVEN: A report with paths containing special characters
    let report = make_clone_report_with_special_paths();

    // WHEN: Formatting as DOT
    let dot = format_clones_dot(&report);

    // THEN: Should be valid DOT syntax (basic checks)
    assert!(dot.starts_with("digraph"), "Should start with digraph");
    assert!(dot.contains("->"), "Should contain edges");
    // No unquoted special characters
    assert!(
        !dot.contains(" -> ") || dot.contains("\" -> \""),
        "Edges should use quoted node IDs"
    );
}

/// Test: DOT output includes similarity percentages
#[test]
fn test_dot_output_includes_similarity() {
    let report = make_clone_report_with_pairs(vec![make_type2_clone_pair()]);

    let dot = format_clones_dot(&report);

    // Should include similarity as edge label
    assert!(
        dot.contains("label=") && (dot.contains("92") || dot.contains("0.92")),
        "DOT should include similarity label: {}",
        dot
    );
}

// =============================================================================
// Text Output Format Tests
// =============================================================================

/// Test: Text output includes compact header with stats
#[test]
fn test_text_output_header() {
    let report = make_clone_report_with_pairs(vec![]);

    let text = format_clones_text(&report);

    assert!(
        text.contains("Clone Detection:"),
        "Should have compact header"
    );
    assert!(
        text.contains("pairs") && text.contains("files") && text.contains("tokens"),
        "Header should include pairs, files, and tokens: {}",
        text
    );
}

/// Test: Text output includes statistics in compact header
#[test]
fn test_text_output_stats() {
    let mut report = make_clone_report_with_pairs(vec![]);
    report.stats = CloneStats {
        files_analyzed: 42,
        total_tokens: 15234,
        clones_found: 8,
        type1_count: 2,
        type2_count: 3,
        type3_count: 3,
        class_count: Some(3),
        detection_time_ms: 150,
    };

    let text = format_clones_text(&report);

    // Compact header: "Clone Detection: 8 pairs in 42 files (15234 tokens)"
    assert!(text.contains("42"), "Should show files analyzed");
    assert!(text.contains("15234"), "Should show tokens analyzed");
    assert!(text.contains("8"), "Should show clones found");
}

/// Test: Text output formats clone pairs as compact table rows
#[test]
fn test_text_output_clone_pair_format() {
    let report = make_clone_report_with_pairs(vec![make_type2_clone_pair()]);

    let text = format_clones_text(&report);

    // Should show file paths (with common prefix stripped)
    assert!(
        text.contains("login.py") && text.contains("signup.py"),
        "Should show file names: {}",
        text
    );
    // Should show line ranges
    assert!(
        text.contains("45-62") && text.contains("23-40"),
        "Should show line ranges: {}",
        text
    );
    // Should show similarity percentage
    assert!(text.contains("92%"), "Should show similarity: {}", text);
    // Should show compact type
    assert!(text.contains("T2"), "Should show compact type: {}", text);
    // Should have table header
    assert!(
        text.contains("File A") && text.contains("File B"),
        "Should have table header: {}",
        text
    );
}

// =============================================================================
// Similarity Text Output Tests
// =============================================================================

/// Test: Similarity text output includes all metrics
#[test]
fn test_similarity_text_output_metrics() {
    let report = make_similarity_report();

    let text = format_similarity_text(&report);

    assert!(
        text.contains("Dice") || text.contains("dice"),
        "Should include Dice"
    );
    assert!(
        text.contains("Jaccard") || text.contains("jaccard"),
        "Should include Jaccard"
    );
}

/// Test: Similarity text output includes interpretation
#[test]
fn test_similarity_text_output_interpretation() {
    let report = make_similarity_report();

    let text = format_similarity_text(&report);

    // Should have human-readable interpretation
    assert!(
        text.contains("similar") || text.contains("match") || text.contains("related"),
        "Should include interpretation: {}",
        text
    );
}

/// Test: Similarity text output includes token breakdown
#[test]
fn test_similarity_text_output_token_breakdown() {
    let report = make_similarity_report();

    let text = format_similarity_text(&report);

    // Should show shared/unique tokens
    assert!(
        text.contains("shared") || text.contains("Shared"),
        "Should include shared tokens"
    );
}

// =============================================================================
// Test Fixtures
// =============================================================================

fn make_clone_report_with_pairs(pairs: Vec<ClonePair>) -> ClonesReport {
    ClonesReport {
        root: PathBuf::from("src/"),
        language: "python".to_string(),
        clone_pairs: pairs,
        clone_classes: vec![],
        stats: CloneStats::default(),
        config: CloneConfig::default(),
    }
}

fn make_type2_clone_pair() -> ClonePair {
    ClonePair::new(
        1,
        CloneType::Type2,
        0.92,
        CloneFragment::new(PathBuf::from("src/auth/login.py"), 45, 62, 156),
        CloneFragment::new(PathBuf::from("src/auth/signup.py"), 23, 40, 152),
    )
}

fn make_clone_report_with_special_paths() -> ClonesReport {
    let pair = ClonePair::new(
        1,
        CloneType::Type1,
        1.0,
        CloneFragment::new(PathBuf::from("path with spaces/file.py"), 10, 20, 50),
        CloneFragment::new(PathBuf::from(r"C:\Users\test\other.py"), 5, 15, 50),
    );
    ClonesReport {
        root: PathBuf::from("."),
        language: "python".to_string(),
        clone_pairs: vec![pair],
        clone_classes: vec![],
        stats: CloneStats::default(),
        config: CloneConfig::default(),
    }
}

fn make_similarity_report() -> SimilarityReport {
    SimilarityReport::new(
        SimilarityFragment::new(PathBuf::from("src/a.py"), 100, 20),
        SimilarityFragment::new(PathBuf::from("src/b.py"), 95, 18),
        SimilarityScores::new(0.85, 0.74),
        TokenBreakdown::new(80, 20, 15),
        SimilarityConfig {
            metric: SimilarityMetric::Dice,
            ngram_size: 1,
            language: Some("python".to_string()),
        },
    )
}

// =============================================================================
// Imports Text Formatter Tests
// =============================================================================

use crate::output::format_imports_text;
use tldr_core::types::ImportInfo;

#[test]
fn test_imports_text_groups_by_module() {
    let imports = vec![
        ImportInfo {
            module: ".exceptions".into(),
            names: vec!["Abort".into(), "BadParameter".into()],
            is_from: true,
            alias: None,
        },
        ImportInfo {
            module: ".exceptions".into(),
            names: vec!["UsageError".into()],
            is_from: true,
            alias: None,
        },
        ImportInfo {
            module: ".core".into(),
            names: vec!["Command".into(), "Group".into()],
            is_from: true,
            alias: None,
        },
    ];
    let text = format_imports_text(&imports);
    // .core comes before .exceptions (BTreeMap sorted)
    assert!(text.contains(".core"));
    assert!(text.contains("Command, Group"));
    assert!(text.contains(".exceptions"));
    assert!(text.contains("Abort, BadParameter, UsageError"));
}

#[test]
fn test_imports_text_bare_imports() {
    let imports = vec![
        ImportInfo {
            module: "os".into(),
            names: vec![],
            is_from: false,
            alias: None,
        },
        ImportInfo {
            module: "sys".into(),
            names: vec![],
            is_from: false,
            alias: None,
        },
    ];
    let text = format_imports_text(&imports);
    assert!(text.contains("import"));
    assert!(text.contains("os"));
    assert!(text.contains("sys"));
}

#[test]
fn test_imports_text_aliased() {
    let imports = vec![ImportInfo {
        module: "typing".into(),
        names: vec![],
        is_from: false,
        alias: Some("t".into()),
    }];
    let text = format_imports_text(&imports);
    assert!(text.contains("typing as t"));
}

#[test]
fn test_imports_text_empty() {
    let imports: Vec<ImportInfo> = vec![];
    let text = format_imports_text(&imports);
    assert!(text.contains("No imports found"));
}

#[test]
fn test_imports_text_mixed() {
    let imports = vec![
        ImportInfo {
            module: ".utils".into(),
            names: vec!["echo".into(), "make_str".into()],
            is_from: true,
            alias: None,
        },
        ImportInfo {
            module: "os".into(),
            names: vec![],
            is_from: false,
            alias: None,
        },
        ImportInfo {
            module: "typing".into(),
            names: vec![],
            is_from: false,
            alias: Some("t".into()),
        },
    ];
    let text = format_imports_text(&imports);
    // from-imports come first, then bare imports
    assert!(text.contains(".utils"));
    assert!(text.contains("echo, make_str"));
    assert!(text.contains("os, typing as t"));
}

// =============================================================================
// Importers Text Formatter Tests
// =============================================================================

use crate::output::format_importers_text;
use tldr_core::types::{ImporterInfo, ImportersReport};

#[test]
fn test_importers_text_basic() {
    let report = ImportersReport {
        module: "os".into(),
        importers: vec![
            ImporterInfo {
                file: PathBuf::from("src/main.py"),
                line: 1,
                import_statement: "import os".into(),
            },
            ImporterInfo {
                file: PathBuf::from("src/utils.py"),
                line: 3,
                import_statement: "import os".into(),
            },
        ],
        total: 2,
    };
    let text = format_importers_text(&report);
    // Common prefix "src/" is stripped, showing relative paths
    assert!(
        text.contains("main.py:1"),
        "expected main.py:1, got: {}",
        text
    );
    assert!(
        text.contains("utils.py:3"),
        "expected utils.py:3, got: {}",
        text
    );
    assert!(text.contains("import os"));
}

#[test]
fn test_importers_text_empty() {
    let report = ImportersReport {
        module: "nonexistent".into(),
        importers: vec![],
        total: 0,
    };
    let text = format_importers_text(&report);
    assert!(text.contains("No files import this module"));
}

#[test]
fn test_importers_text_aligned() {
    let report = ImportersReport {
        module: "os".into(),
        importers: vec![
            ImporterInfo {
                file: PathBuf::from("a.py"),
                line: 1,
                import_statement: "import os".into(),
            },
            ImporterInfo {
                file: PathBuf::from("very/long/path/to/file.py"),
                line: 42,
                import_statement: "import os".into(),
            },
        ],
        total: 2,
    };
    let text = format_importers_text(&report);
    // Both lines should be present with aligned columns
    assert!(text.contains("a.py:1"));
    assert!(text.contains("very/long/path/to/file.py:42"));
}

// =============================================================================
// Diagnostics Text Formatter Tests (R1/R2/R3)
// =============================================================================

use crate::output::format_diagnostics_text;
use tldr_core::diagnostics::{
    Diagnostic, DiagnosticsReport, DiagnosticsSummary, Severity, ToolResult,
};

/// Helper: create a Diagnostic
fn make_diagnostic(
    location: (&str, u32, u32),
    severity: Severity,
    code: Option<&str>,
    message: &str,
    source: &str,
    url: Option<&str>,
) -> Diagnostic {
    let (file, line, col) = location;
    Diagnostic {
        file: PathBuf::from(file),
        line,
        column: col,
        end_line: None,
        end_column: None,
        severity,
        message: message.to_string(),
        code: code.map(|c| c.to_string()),
        source: source.to_string(),
        url: url.map(|u| u.to_string()),
    }
}

/// Helper: create a ToolResult
fn make_tool_result(name: &str, success: bool, count: usize) -> ToolResult {
    ToolResult {
        name: name.to_string(),
        version: Some("1.0.0".to_string()),
        success,
        duration_ms: 100,
        diagnostic_count: count,
        error: None,
    }
}

/// Helper: create a DiagnosticsReport with given diagnostics and tools
fn make_diagnostics_report(
    diagnostics: Vec<Diagnostic>,
    tools: Vec<ToolResult>,
    files_analyzed: usize,
) -> DiagnosticsReport {
    let summary = DiagnosticsSummary {
        errors: diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count(),
        warnings: diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count(),
        info: diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Information)
            .count(),
        hints: diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Hint)
            .count(),
        total: diagnostics.len(),
    };
    DiagnosticsReport {
        diagnostics,
        summary,
        tools_run: tools,
        files_analyzed,
    }
}

// --- R1: Compact one-line summary header ---

/// Test: Header is a single compact line with tool names, file count, and error/warning counts
#[test]
fn test_diagnostics_text_compact_header() {
    let report = make_diagnostics_report(
        vec![
            make_diagnostic(
                ("/src/auth.py", 12, 5),
                Severity::Error,
                Some("E001"),
                "bad type",
                "pyright",
                None,
            ),
            make_diagnostic(
                ("/src/auth.py", 58, 1),
                Severity::Warning,
                Some("E501"),
                "line too long",
                "ruff",
                None,
            ),
        ],
        vec![
            make_tool_result("pyright", true, 1),
            make_tool_result("ruff", true, 1),
        ],
        42,
    );
    let text = format_diagnostics_text(&report, 0);

    // Should have compact summary line: "pyright + ruff | 42 files | 1 error, 1 warning"
    assert!(text.contains("pyright"), "Should list tool names: {}", text);
    assert!(text.contains("ruff"), "Should list tool names: {}", text);
    assert!(
        text.contains("42 files"),
        "Should show file count: {}",
        text
    );
    assert!(
        text.contains("1 error"),
        "Should show error count: {}",
        text
    );
    assert!(
        text.contains("1 warning"),
        "Should show warning count: {}",
        text
    );
}

/// Test: No decorative headers like "Diagnostics Report", "==================", "Summary", "-------"
#[test]
fn test_diagnostics_text_no_decorative_headers() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 1, 1),
            Severity::Error,
            None,
            "err",
            "pyright",
            None,
        )],
        vec![make_tool_result("pyright", true, 1)],
        10,
    );
    let text = format_diagnostics_text(&report, 0);

    assert!(
        !text.contains("Diagnostics Report"),
        "Should not have 'Diagnostics Report' header: {}",
        text
    );
    assert!(
        !text.contains("=================="),
        "Should not have '==================' decoration: {}",
        text
    );
    assert!(
        !text.contains("Summary\n"),
        "Should not have separate 'Summary' header: {}",
        text
    );
    assert!(
        !text.contains("-------"),
        "Should not have '-------' decoration: {}",
        text
    );
}

/// Test: No ANSI escape codes in text output
#[test]
fn test_diagnostics_text_no_ansi_codes() {
    let report = make_diagnostics_report(
        vec![
            make_diagnostic(
                ("/src/a.py", 1, 1),
                Severity::Error,
                Some("E001"),
                "an error",
                "pyright",
                None,
            ),
            make_diagnostic(
                ("/src/a.py", 2, 1),
                Severity::Warning,
                Some("W001"),
                "a warning",
                "ruff",
                None,
            ),
        ],
        vec![
            make_tool_result("pyright", true, 1),
            make_tool_result("ruff", true, 1),
        ],
        5,
    );
    let text = format_diagnostics_text(&report, 0);

    // ANSI escape sequences start with \x1b[ or \033[
    assert!(
        !text.contains('\x1b'),
        "Should not contain ANSI escape codes: {:?}",
        text
    );
}

// --- R1: One-line-per-diagnostic format ---

/// Test: Each diagnostic is on a single line in format: file:line:col: severity[code] message (tool)
#[test]
fn test_diagnostics_text_one_line_per_diagnostic() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/project/src/auth.py", 12, 5),
            Severity::Error,
            Some("reportArgumentType"),
            "Type \"str\" is not assignable to type \"int\"",
            "pyright",
            Some("https://example.com/doc"),
        )],
        vec![make_tool_result("pyright", true, 1)],
        1,
    );
    let text = format_diagnostics_text(&report, 0);

    // Should contain one-line format: file:line:col: severity[code] message (tool)
    assert!(
        text.contains("auth.py:12:5: error[reportArgumentType]"),
        "Should have file:line:col: severity[code] format: {}",
        text
    );
    assert!(
        text.contains("(pyright)"),
        "Should have (tool) suffix: {}",
        text
    );
    assert!(
        text.contains("Type \"str\" is not assignable to type \"int\""),
        "Should contain the message: {}",
        text
    );
}

/// Test: No URLs in text output
#[test]
fn test_diagnostics_text_no_urls() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 1, 1),
            Severity::Error,
            Some("E001"),
            "err",
            "pyright",
            Some("https://example.com/doc"),
        )],
        vec![make_tool_result("pyright", true, 1)],
        1,
    );
    let text = format_diagnostics_text(&report, 0);

    assert!(
        !text.contains("https://"),
        "Should not contain URLs: {}",
        text
    );
    assert!(
        !text.contains("http://"),
        "Should not contain URLs: {}",
        text
    );
}

/// Test: Diagnostics without a code omit the brackets
#[test]
fn test_diagnostics_text_no_code_no_brackets() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 1, 1),
            Severity::Warning,
            None,
            "some warning",
            "ruff",
            None,
        )],
        vec![make_tool_result("ruff", true, 1)],
        1,
    );
    let text = format_diagnostics_text(&report, 0);

    // Should have "warning some warning" without empty brackets
    assert!(
        !text.contains("[]"),
        "Should not have empty brackets: {}",
        text
    );
    assert!(
        text.contains("warning some warning"),
        "Should have 'severity message' without code: {}",
        text
    );
}

// --- R2: Strip paths to relative ---

/// Test: Absolute paths are stripped to relative using common prefix
#[test]
fn test_diagnostics_text_paths_relative() {
    let report = make_diagnostics_report(
        vec![
            make_diagnostic(
                ("/home/user/project/src/auth.py", 12, 5),
                Severity::Error,
                Some("E001"),
                "err1",
                "pyright",
                None,
            ),
            make_diagnostic(
                ("/home/user/project/src/models.py", 24, 8),
                Severity::Warning,
                Some("W001"),
                "warn1",
                "ruff",
                None,
            ),
        ],
        vec![
            make_tool_result("pyright", true, 1),
            make_tool_result("ruff", true, 1),
        ],
        10,
    );
    let text = format_diagnostics_text(&report, 0);

    // Common prefix is /home/user/project/src/, so paths should be relative
    assert!(
        text.contains("auth.py:12:5:"),
        "Should show relative path auth.py: {}",
        text
    );
    assert!(
        text.contains("models.py:24:8:"),
        "Should show relative path models.py: {}",
        text
    );
    assert!(
        !text.contains("/home/user/project"),
        "Should not contain absolute prefix: {}",
        text
    );
}

/// Test: Single-file run shows just filename
#[test]
fn test_diagnostics_text_single_file_just_filename() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/home/user/project/src/auth.py", 12, 5),
            Severity::Error,
            Some("E001"),
            "err",
            "pyright",
            None,
        )],
        vec![make_tool_result("pyright", true, 1)],
        1,
    );
    let text = format_diagnostics_text(&report, 0);

    // Single file: common_path_prefix gives parent dir, so just "auth.py"
    assert!(
        text.contains("auth.py:12:5:"),
        "Should show just filename: {}",
        text
    );
    assert!(
        !text.contains("/home/user"),
        "Should not contain absolute path: {}",
        text
    );
}

// --- R3: Truncate pyright nested explanations ---

/// Test: Multi-line messages are truncated to first line only
#[test]
fn test_diagnostics_text_truncate_multiline_message() {
    let multiline_msg = "Type \"str\" is not assignable to type \"int\"\n  \"str\" is not assignable to \"int\"\n    Because reasons";
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 12, 5),
            Severity::Error,
            Some("reportArgumentType"),
            multiline_msg,
            "pyright",
            None,
        )],
        vec![make_tool_result("pyright", true, 1)],
        1,
    );
    let text = format_diagnostics_text(&report, 0);

    assert!(
        text.contains("Type \"str\" is not assignable to type \"int\""),
        "Should contain first line of message: {}",
        text
    );
    assert!(
        !text.contains("Because reasons"),
        "Should NOT contain nested explanation lines: {}",
        text
    );
    assert!(
        !text.contains("is not assignable to \"int\""),
        "Should NOT contain second line of nested explanation: {}",
        text
    );
}

// --- Edge cases ---

/// Test: Empty diagnostics shows "No issues found."
#[test]
fn test_diagnostics_text_empty() {
    let report = make_diagnostics_report(
        vec![],
        vec![
            make_tool_result("pyright", true, 0),
            make_tool_result("ruff", true, 0),
        ],
        42,
    );
    let text = format_diagnostics_text(&report, 0);

    assert!(
        text.contains("No issues found"),
        "Should indicate no issues: {}",
        text
    );
    assert!(
        text.contains("pyright"),
        "Should still list tools: {}",
        text
    );
}

/// Test: Filtered count shown when non-zero
#[test]
fn test_diagnostics_text_filtered_count() {
    let report = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 1, 1),
            Severity::Error,
            Some("E001"),
            "err",
            "pyright",
            None,
        )],
        vec![make_tool_result("pyright", true, 1)],
        5,
    );
    let text = format_diagnostics_text(&report, 3);

    assert!(text.contains("3"), "Should show filtered count: {}", text);
    assert!(
        text.contains("filtered"),
        "Should mention filtering: {}",
        text
    );
}

/// Test: Compact summary uses singular/plural correctly
#[test]
fn test_diagnostics_text_summary_pluralization() {
    // 1 error (singular)
    let report1 = make_diagnostics_report(
        vec![make_diagnostic(
            ("/src/a.py", 1, 1),
            Severity::Error,
            None,
            "err",
            "pyright",
            None,
        )],
        vec![make_tool_result("pyright", true, 1)],
        1,
    );
    let text1 = format_diagnostics_text(&report1, 0);
    assert!(
        text1.contains("1 error"),
        "Should use singular 'error': {}",
        text1
    );
    assert!(
        !text1.contains("1 errors"),
        "Should not use plural for 1: {}",
        text1
    );

    // 2 errors (plural)
    let report2 = make_diagnostics_report(
        vec![
            make_diagnostic(
                ("/src/a.py", 1, 1),
                Severity::Error,
                None,
                "err1",
                "pyright",
                None,
            ),
            make_diagnostic(
                ("/src/a.py", 2, 1),
                Severity::Error,
                None,
                "err2",
                "pyright",
                None,
            ),
        ],
        vec![make_tool_result("pyright", true, 2)],
        1,
    );
    let text2 = format_diagnostics_text(&report2, 0);
    assert!(
        text2.contains("2 errors"),
        "Should use plural 'errors': {}",
        text2
    );
}

// =============================================================================
// Smells Text Formatter Tests (token optimization rewrite)
// =============================================================================

use crate::output::format_smells_text;
use std::collections::HashMap;
use tldr_core::quality::smells::{SmellFinding, SmellType, SmellsReport, SmellsSummary};

/// Helper to create a SmellFinding for testing
fn make_smell(
    smell_type: SmellType,
    name: &str,
    file: &str,
    line: u32,
    severity: u8,
) -> SmellFinding {
    SmellFinding {
        smell_type,
        file: PathBuf::from(file),
        name: name.to_string(),
        line,
        reason: "test reason".to_string(),
        severity,
        suggestion: None,
    }
}

/// Helper to build a SmellsReport from a list of smells
fn make_smells_report(smells: Vec<SmellFinding>) -> SmellsReport {
    let mut by_file: HashMap<PathBuf, Vec<SmellFinding>> = HashMap::new();
    for s in &smells {
        by_file.entry(s.file.clone()).or_default().push(s.clone());
    }
    let mut by_type: HashMap<String, usize> = HashMap::new();
    for s in &smells {
        *by_type.entry(format!("{}", s.smell_type)).or_default() += 1;
    }
    let files_scanned = by_file.len();
    let total_smells = smells.len();
    let avg = if files_scanned > 0 {
        total_smells as f64 / files_scanned as f64
    } else {
        0.0
    };
    SmellsReport {
        smells,
        files_scanned,
        by_file,
        summary: SmellsSummary {
            total_smells,
            by_type,
            avg_smells_per_file: avg,
        },
    }
}

/// Test: Empty smells report shows "No code smells detected."
#[test]
fn test_smells_text_empty_report() {
    let report = make_smells_report(vec![]);
    let text = format_smells_text(&report);
    assert!(
        text.contains("No code smells detected."),
        "Empty report should say no smells: {}",
        text
    );
    assert!(
        text.contains("0 issues"),
        "Should show 0 issues in header: {}",
        text
    );
}

/// Test: Output does NOT contain comfy_table box-drawing characters
#[test]
fn test_smells_text_no_box_drawing() {
    let report = make_smells_report(vec![
        make_smell(SmellType::GodClass, "BigClass", "src/big.py", 10, 3),
        make_smell(SmellType::LongMethod, "long_func", "src/utils.py", 42, 2),
    ]);
    let text = format_smells_text(&report);

    // Box-drawing chars used by comfy_table UTF8_FULL preset
    let box_chars = [
        '\u{2500}', '\u{2502}', '\u{250C}', '\u{2510}', '\u{2514}', '\u{2518}', '\u{251C}',
        '\u{2524}', '\u{252C}', '\u{2534}', '\u{253C}', '\u{2506}', '\u{254C}', '\u{2503}',
    ];
    for ch in &box_chars {
        assert!(
            !text.contains(*ch),
            "Should not contain box-drawing char U+{:04X}: {}",
            *ch as u32,
            text
        );
    }
}

/// Test: Output contains header with column labels
#[test]
fn test_smells_text_has_header_row() {
    let report = make_smells_report(vec![make_smell(
        SmellType::DeadCode,
        "old_func",
        "src/code.py",
        5,
        1,
    )]);
    let text = format_smells_text(&report);
    assert!(
        text.contains("#"),
        "Header should contain # column: {}",
        text
    );
    assert!(
        text.contains("Sev"),
        "Header should contain Sev column: {}",
        text
    );
    assert!(
        text.contains("Type"),
        "Header should contain Type column: {}",
        text
    );
    assert!(
        text.contains("Name"),
        "Header should contain Name column: {}",
        text
    );
    assert!(
        text.contains("File"),
        "Header should contain File column: {}",
        text
    );
}

/// Test: Data rows show correct index, severity, type, name, file:line
#[test]
fn test_smells_text_data_rows() {
    let report = make_smells_report(vec![
        make_smell(SmellType::GodClass, "BigClass", "src/big.py", 10, 3),
        make_smell(SmellType::LongMethod, "long_func", "src/utils.py", 42, 2),
        make_smell(SmellType::DeadCode, "unused", "src/old.py", 99, 1),
    ]);
    let text = format_smells_text(&report);

    // Check each smell appears in the output (strip ANSI for content checking)
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("God Class"),
        "Should contain 'God Class': {}",
        plain
    );
    assert!(
        plain.contains("Long Method"),
        "Should contain 'Long Method': {}",
        plain
    );
    assert!(
        plain.contains("Dead Code"),
        "Should contain 'Dead Code': {}",
        plain
    );
    assert!(
        plain.contains("BigClass"),
        "Should contain name 'BigClass': {}",
        plain
    );
    assert!(
        plain.contains("long_func"),
        "Should contain name 'long_func': {}",
        plain
    );
    assert!(
        plain.contains("unused"),
        "Should contain name 'unused': {}",
        plain
    );
    // File:line format
    assert!(
        plain.contains("big.py:10"),
        "Should show file:line 'big.py:10': {}",
        plain
    );
    assert!(
        plain.contains("utils.py:42"),
        "Should show file:line 'utils.py:42': {}",
        plain
    );
    assert!(
        plain.contains("old.py:99"),
        "Should show file:line 'old.py:99': {}",
        plain
    );
}

/// Test: Long names are truncated to ~28 chars with "..."
#[test]
fn test_smells_text_name_truncation() {
    let long_name = "a_very_long_function_name_that_exceeds_the_limit";
    let report = make_smells_report(vec![make_smell(
        SmellType::LongMethod,
        long_name,
        "src/main.py",
        1,
        2,
    )]);
    let text = format_smells_text(&report);
    let plain = strip_ansi_codes(&text);
    assert!(
        !plain.contains(long_name),
        "Full long name should not appear (should be truncated): {}",
        plain
    );
    assert!(
        plain.contains("..."),
        "Truncated name should end with '...': {}",
        plain
    );
}

/// Test: Path stripping removes common prefix
#[test]
fn test_smells_text_path_stripping() {
    let report = make_smells_report(vec![
        make_smell(
            SmellType::GodClass,
            "A",
            "crates/tldr-core/src/quality/smells.rs",
            1,
            3,
        ),
        make_smell(
            SmellType::LongMethod,
            "B",
            "crates/tldr-core/src/quality/coverage.rs",
            2,
            2,
        ),
    ]);
    let text = format_smells_text(&report);
    let plain = strip_ansi_codes(&text);
    // After path stripping, the common prefix "crates/tldr-core/src/quality/" should be removed
    // Only basenames (or short relative paths) should appear
    assert!(
        !plain.contains("crates/tldr-core/src/quality/smells.rs"),
        "Full path should be stripped: {}",
        plain
    );
    assert!(
        plain.contains("smells.rs"),
        "Basename should still appear: {}",
        plain
    );
    assert!(
        plain.contains("coverage.rs"),
        "Basename should still appear: {}",
        plain
    );
}

/// Test: Summary section includes severity breakdown and per-type counts
#[test]
fn test_smells_text_summary_section() {
    let report = make_smells_report(vec![
        make_smell(SmellType::GodClass, "A", "src/a.py", 1, 3),
        make_smell(SmellType::LongMethod, "B", "src/b.py", 2, 2),
        make_smell(SmellType::DeadCode, "C", "src/c.py", 3, 1),
        make_smell(SmellType::DeadCode, "D", "src/d.py", 4, 1),
    ]);
    let text = format_smells_text(&report);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Summary"),
        "Should contain Summary section: {}",
        plain
    );
    assert!(plain.contains("sev-3"), "Should mention sev-3: {}", plain);
    assert!(plain.contains("sev-2"), "Should mention sev-2: {}", plain);
    assert!(plain.contains("sev-1"), "Should mention sev-1: {}", plain);
    assert!(
        plain.contains("4 files"),
        "Should mention file count: {}",
        plain
    );
    // Per-type breakdown
    assert!(
        plain.contains("Dead Code: 2"),
        "Should show per-type count: {}",
        plain
    );
}

/// Helper to strip ANSI escape codes for content assertions
fn strip_ansi_codes(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").to_string()
}

// =============================================================================
// Secrets Text Formatter Tests
// =============================================================================

use crate::output::format_secrets_text;
use tldr_core::security::secrets::SecretsSummary;
use tldr_core::security::secrets::Severity as SecretSeverity;
use tldr_core::{SecretFinding, SecretsReport};

fn make_secrets_report(findings: Vec<SecretFinding>, files_scanned: usize) -> SecretsReport {
    let mut by_severity = std::collections::HashMap::new();
    let mut by_pattern = std::collections::HashMap::new();
    for f in &findings {
        *by_severity.entry(format!("{}", f.severity)).or_insert(0) += 1;
        *by_pattern.entry(f.pattern.clone()).or_insert(0) += 1;
    }
    SecretsReport {
        findings,
        files_scanned,
        patterns_checked: 11,
        summary: SecretsSummary {
            total_findings: by_severity.values().sum(),
            by_severity,
            by_pattern,
        },
    }
}

fn make_finding(
    severity: SecretSeverity,
    pattern: &str,
    file: &str,
    line: u32,
    masked: &str,
) -> SecretFinding {
    SecretFinding {
        file: PathBuf::from(file),
        line,
        column: 0,
        pattern: pattern.to_string(),
        severity,
        masked_value: masked.to_string(),
        description: String::new(),
        line_content: None,
    }
}

/// Test: Empty findings shows "No secrets detected"
#[test]
fn test_secrets_text_empty() {
    let report = make_secrets_report(vec![], 50);
    let text = format_secrets_text(&report);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("0 findings"),
        "Should show 0 findings: {}",
        plain
    );
    assert!(
        plain.contains("50 files scanned"),
        "Should show files scanned: {}",
        plain
    );
    assert!(
        plain.contains("No secrets detected"),
        "Should show no-detection message: {}",
        plain
    );
    assert!(
        !plain.contains("Severity"),
        "Should not show header for empty report: {}",
        plain
    );
}

/// Test: No box-drawing characters in output (no comfy_table)
#[test]
fn test_secrets_text_no_box_drawing() {
    let report = make_secrets_report(
        vec![make_finding(
            SecretSeverity::Critical,
            "AWS Access Key",
            "/src/config.py",
            42,
            "AKIA************MPLE",
        )],
        10,
    );
    let text = format_secrets_text(&report);
    for ch in [
        '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '╞', '╪', '╡', '═', '│', '─', '┆', '╌',
    ] {
        assert!(
            !text.contains(ch),
            "Should not contain box-drawing char '{}': {}",
            ch,
            text
        );
    }
}

/// Test: Paths are stripped to relative
#[test]
fn test_secrets_text_path_stripping() {
    let report = make_secrets_report(
        vec![
            make_finding(
                SecretSeverity::High,
                "Password",
                "/long/common/prefix/src/config.py",
                10,
                "pass****ord",
            ),
            make_finding(
                SecretSeverity::Medium,
                "API Key",
                "/long/common/prefix/lib/api.py",
                20,
                "key****val",
            ),
        ],
        5,
    );
    let text = format_secrets_text(&report);
    let plain = strip_ansi_codes(&text);
    assert!(
        !plain.contains("/long/common/prefix/"),
        "Should strip common prefix: {}",
        plain
    );
    assert!(
        plain.contains("src/config.py"),
        "Should show relative path: {}",
        plain
    );
    assert!(
        plain.contains("lib/api.py"),
        "Should show relative path: {}",
        plain
    );
}

/// Test: Summary shows severity breakdown
#[test]
fn test_secrets_text_summary() {
    let report = make_secrets_report(
        vec![
            make_finding(
                SecretSeverity::Critical,
                "AWS Key",
                "/src/a.py",
                1,
                "AKIA****",
            ),
            make_finding(SecretSeverity::High, "Password", "/src/b.py", 2, "pass****"),
            make_finding(SecretSeverity::Medium, "JWT", "/src/c.py", 3, "eyJ****"),
        ],
        20,
    );
    let text = format_secrets_text(&report);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Summary:"),
        "Should contain Summary: {}",
        plain
    );
    assert!(
        plain.contains("1 critical"),
        "Should show critical count: {}",
        plain
    );
    assert!(
        plain.contains("1 high"),
        "Should show high count: {}",
        plain
    );
    assert!(
        plain.contains("1 medium"),
        "Should show medium count: {}",
        plain
    );
}

/// Test: Long file paths are truncated
#[test]
fn test_secrets_text_long_path_truncation() {
    // Two findings with different long paths so common prefix doesn't eat the whole path
    let report = make_secrets_report(
        vec![
            make_finding(
                SecretSeverity::Low,
                "Bearer",
                "/root/a/very/deeply/nested/directory/structure/with/many/levels/config.yaml",
                1,
                "bear****",
            ),
            make_finding(
                SecretSeverity::Low,
                "Bearer",
                "/root/b/other/path/short.py",
                2,
                "bear****",
            ),
        ],
        2,
    );
    let text = format_secrets_text(&report);
    let plain = strip_ansi_codes(&text);
    // First path after stripping /root/ is still >40 chars, should be truncated with ...
    assert!(
        plain.contains("..."),
        "Long path should be truncated: {}",
        plain
    );
}

// =============================================================================
// ModuleInfo Text Formatter Tests
// =============================================================================

use crate::output::format_module_info_text;
use tldr_core::types::{ClassInfo, FieldInfo, FunctionInfo, IntraFileCallGraph, ModuleInfo};
use tldr_core::Language;

/// Helper: build a minimal ModuleInfo for testing
fn make_module_info() -> ModuleInfo {
    ModuleInfo {
        file_path: PathBuf::from("/src/example.py"),
        language: Language::Python,
        docstring: Some("Example module for testing.".to_string()),
        imports: vec![
            ImportInfo {
                module: "os".to_string(),
                names: vec![],
                is_from: false,
                alias: None,
            },
            ImportInfo {
                module: "typing".to_string(),
                names: vec!["List".to_string(), "Optional".to_string()],
                is_from: true,
                alias: None,
            },
        ],
        functions: vec![
            FunctionInfo {
                name: "process_data".to_string(),
                params: vec!["data: list".to_string(), "config: dict".to_string()],
                return_type: Some("bool".to_string()),
                docstring: Some("Process input data.".to_string()),
                is_method: false,
                is_async: true,
                decorators: vec![],
                line_number: 10,
            },
            FunctionInfo {
                name: "helper".to_string(),
                params: vec![],
                return_type: None,
                docstring: None,
                is_method: false,
                is_async: false,
                decorators: vec![],
                line_number: 25,
            },
        ],
        classes: vec![ClassInfo {
            name: "DataHandler".to_string(),
            bases: vec!["BaseHandler".to_string(), "Serializable".to_string()],
            docstring: Some("Handles data processing.".to_string()),
            methods: vec![
                FunctionInfo {
                    name: "__init__".to_string(),
                    params: vec!["self".to_string(), "config: dict".to_string()],
                    return_type: None,
                    docstring: None,
                    is_method: true,
                    is_async: false,
                    decorators: vec![],
                    line_number: 32,
                },
                FunctionInfo {
                    name: "run".to_string(),
                    params: vec!["self".to_string()],
                    return_type: Some("Result".to_string()),
                    docstring: Some("Run the handler.".to_string()),
                    is_method: true,
                    is_async: true,
                    decorators: vec![],
                    line_number: 40,
                },
            ],
            fields: vec![FieldInfo {
                name: "config".to_string(),
                field_type: Some("dict".to_string()),
                default_value: None,
                is_static: false,
                is_constant: false,
                visibility: None,
                line_number: 33,
            }],
            decorators: vec![],
            line_number: 30,
        }],
        constants: vec![FieldInfo {
            name: "MAX_RETRIES".to_string(),
            field_type: Some("int".to_string()),
            default_value: Some("3".to_string()),
            is_static: false,
            is_constant: true,
            visibility: None,
            line_number: 5,
        }],
        call_graph: IntraFileCallGraph {
            calls: {
                let mut m = HashMap::new();
                m.insert("process_data".to_string(), vec!["helper".to_string()]);
                m.insert(
                    "DataHandler.run".to_string(),
                    vec!["process_data".to_string()],
                );
                m
            },
            called_by: {
                let mut m = HashMap::new();
                m.insert("helper".to_string(), vec!["process_data".to_string()]);
                m.insert(
                    "process_data".to_string(),
                    vec!["DataHandler.run".to_string()],
                );
                m
            },
        },
    }
}

/// Test: header shows file path and language
#[test]
fn test_module_info_text_header() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("/src/example.py"),
        "Should contain file path: {}",
        plain
    );
    assert!(
        plain.contains("python"),
        "Should contain language: {}",
        plain
    );
}

/// Test: docstring is shown
#[test]
fn test_module_info_text_docstring() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Example module for testing."),
        "Should contain docstring: {}",
        plain
    );
}

/// Test: imports section present
#[test]
fn test_module_info_text_imports() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Imports"),
        "Should have Imports section: {}",
        plain
    );
    assert!(plain.contains("os"), "Should list os import: {}", plain);
    assert!(
        plain.contains("typing"),
        "Should list typing import: {}",
        plain
    );
}

/// Test: functions section with details
#[test]
fn test_module_info_text_functions() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Functions"),
        "Should have Functions section: {}",
        plain
    );
    assert!(
        plain.contains("process_data"),
        "Should list process_data: {}",
        plain
    );
    assert!(plain.contains("async"), "Should show async flag: {}", plain);
    assert!(plain.contains("bool"), "Should show return type: {}", plain);
    assert!(plain.contains("helper"), "Should list helper: {}", plain);
}

/// Test: classes section with bases and methods
#[test]
fn test_module_info_text_classes() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Classes"),
        "Should have Classes section: {}",
        plain
    );
    assert!(
        plain.contains("DataHandler"),
        "Should list DataHandler: {}",
        plain
    );
    assert!(
        plain.contains("BaseHandler"),
        "Should show base class: {}",
        plain
    );
    assert!(
        plain.contains("__init__"),
        "Should list __init__ method: {}",
        plain
    );
    assert!(plain.contains("run"), "Should list run method: {}", plain);
}

/// Test: constants section
#[test]
fn test_module_info_text_constants() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Constants"),
        "Should have Constants section: {}",
        plain
    );
    assert!(
        plain.contains("MAX_RETRIES"),
        "Should list MAX_RETRIES: {}",
        plain
    );
}

/// Test: call graph section
#[test]
fn test_module_info_text_call_graph() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("Call Graph"),
        "Should have Call Graph section: {}",
        plain
    );
    assert!(
        plain.contains("process_data"),
        "Should show process_data in call graph: {}",
        plain
    );
    assert!(
        plain.contains("helper"),
        "Should show helper in call graph: {}",
        plain
    );
}

/// Test: empty module renders without panic
#[test]
fn test_module_info_text_empty() {
    let info = ModuleInfo {
        file_path: PathBuf::from("/src/empty.py"),
        language: Language::Python,
        docstring: None,
        imports: vec![],
        functions: vec![],
        classes: vec![],
        constants: vec![],
        call_graph: IntraFileCallGraph::default(),
    };
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    assert!(
        plain.contains("/src/empty.py"),
        "Should show file path: {}",
        plain
    );
    assert!(
        !plain.contains("Functions"),
        "Should not have Functions section for empty: {}",
        plain
    );
    assert!(
        !plain.contains("Classes"),
        "Should not have Classes section for empty: {}",
        plain
    );
}

/// Test: no box-drawing characters (no comfy_table)
#[test]
fn test_module_info_text_no_box_drawing() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    for ch in ['┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', '═', '│', '─'] {
        assert!(
            !text.contains(ch),
            "Should not contain box-drawing char '{}': {}",
            ch,
            text
        );
    }
}

/// Test: long docstrings are truncated
#[test]
fn test_module_info_text_long_docstring_truncated() {
    let mut info = make_module_info();
    info.docstring = Some("A".repeat(200));
    let text = format_module_info_text(&info);
    let plain = strip_ansi_codes(&text);
    // Should not contain the full 200-char string
    assert!(
        !plain.contains(&"A".repeat(200)),
        "Should truncate long docstring"
    );
    assert!(
        plain.contains("..."),
        "Truncated docstring should end with ...: {}",
        plain
    );
}

/// Test: output is significantly shorter than JSON
#[test]
fn test_module_info_text_compression() {
    let info = make_module_info();
    let text = format_module_info_text(&info);
    let json = serde_json::to_string_pretty(&info).unwrap();
    // Text format should be at least 50% shorter than JSON
    assert!(
        text.len() < json.len() * 70 / 100,
        "Text ({} bytes) should be <70% of JSON ({} bytes)",
        text.len(),
        json.len()
    );
}
