//! Output Formatting for Reaching Definitions (RD-14, RD-15, RD-16)
//!
//! This module provides output formatters for reaching definitions reports:
//! - JSON output (RD-15): Structured format for programmatic consumption
//! - Text output (RD-14): Human-readable format for CLI display
//! - Variable filtering (RD-16): Filter output to a specific variable
//!
//! # JSON Schema
//!
//! The JSON output follows the schema from session10-spec.md Section 5.4:
//! ```json
//! {
//!   "function": "process_data",
//!   "file": "src/main.py",
//!   "blocks": [...],
//!   "def_use_chains": [...],
//!   "use_def_chains": [...],
//!   "uninitialized": [...],
//!   "stats": {...}
//! }
//! ```
//!
//! # Text Format
//!
//! The text output follows the format from session10-spec.md Section 5.5:
//! ```text
//! Reaching Definitions for: process_data in src/main.py
//!
//! Block 0 (lines 1-3):
//!     GEN:  {x@1, y@2}
//!     KILL: {}
//!     IN:   {}
//!     OUT:  {x@1, y@2}
//! ...
//! ```

use super::chains::{
    BlockReachingDefs, DefUseChain, Definition, ReachingDefsReport, ReachingDefsStats,
    UninitializedUse, UseDefChain,
};

// =============================================================================
// JSON Output (RD-15)
// =============================================================================

/// Format reaching definitions report as JSON.
///
/// The output is "pretty-printed" with indentation for readability.
///
/// # Arguments
/// * `report` - The reaching definitions report to format
///
/// # Returns
/// * `Result<String, serde_json::Error>` - JSON string or error
///
/// # Example
/// ```ignore
/// let report = build_reaching_defs_report(&cfg, &refs, path);
/// let json = format_reaching_defs_json(&report)?;
/// println!("{}", json);
/// ```
pub fn format_reaching_defs_json(report: &ReachingDefsReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// Format reaching definitions report as compact JSON.
///
/// No whitespace or indentation - suitable for machine processing.
///
/// # Arguments
/// * `report` - The reaching definitions report to format
///
/// # Returns
/// * `Result<String, serde_json::Error>` - Compact JSON string or error
pub fn format_reaching_defs_json_compact(
    report: &ReachingDefsReport,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(report)
}

// =============================================================================
// Text Output (RD-14)
// =============================================================================

/// Options controlling which sections appear in text output.
///
/// Controls visibility of per-block details, chains, header, and statistics
/// in the human-readable text formatter.
///
/// # Defaults
///
/// The default configuration shows header, chains, uninitialized warnings,
/// and statistics, but hides per-block GEN/KILL/IN/OUT details (since
/// `show_in_out` defaults to false in the CLI).
///
/// # Examples
///
/// ```ignore
/// // Default: header + chains + stats (no blocks)
/// let opts = ReachingDefsFormatOptions::default();
///
/// // Chains only: just def-use and use-def chains
/// let opts = ReachingDefsFormatOptions::chains_only();
///
/// // Everything: blocks + chains + header + stats
/// let opts = ReachingDefsFormatOptions {
///     show_blocks: true,
///     ..ReachingDefsFormatOptions::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ReachingDefsFormatOptions {
    /// Show per-block GEN/KILL/IN/OUT sets (controlled by --show-in-out)
    pub show_blocks: bool,
    /// Show def-use and use-def chains
    pub show_chains: bool,
    /// Show potentially uninitialized variable warnings
    pub show_uninitialized: bool,
    /// Show the header line ("Reaching Definitions for: ...")
    pub show_header: bool,
    /// Show the statistics summary at the bottom
    pub show_stats: bool,
}

impl Default for ReachingDefsFormatOptions {
    fn default() -> Self {
        Self {
            show_blocks: false,
            show_chains: true,
            show_uninitialized: true,
            show_header: true,
            show_stats: true,
        }
    }
}

impl ReachingDefsFormatOptions {
    /// Create options that show only def-use/use-def chains.
    ///
    /// Hides header, per-block details, uninitialized warnings, and statistics.
    /// Useful for piping into other tools or getting concise output.
    pub fn chains_only() -> Self {
        Self {
            show_blocks: false,
            show_chains: true,
            show_uninitialized: false,
            show_header: false,
            show_stats: false,
        }
    }
}

/// Format reaching definitions report as human-readable text with options.
///
/// Allows controlling which sections appear in the output via
/// `ReachingDefsFormatOptions`.
///
/// # Arguments
/// * `report` - The reaching definitions report to format
/// * `options` - Controls which sections to include in output
///
/// # Returns
/// * `String` - Formatted text output
///
/// # Example
/// ```ignore
/// let report = build_reaching_defs_report(&cfg, &refs, path);
/// let opts = ReachingDefsFormatOptions::chains_only();
/// let text = format_reaching_defs_text_with_options(&report, &opts);
/// println!("{}", text);
/// ```
pub fn format_reaching_defs_text_with_options(
    report: &ReachingDefsReport,
    options: &ReachingDefsFormatOptions,
) -> String {
    let mut output = String::new();

    // Header
    if options.show_header {
        output.push_str(&format!(
            "Reaching Definitions for: {} in {}\n\n",
            report.function,
            report.file.display()
        ));
    }

    // Blocks with GEN/KILL/IN/OUT sets
    if options.show_blocks {
        for block in &report.blocks {
            output.push_str(&format!(
                "Block {} (lines {}-{}):\n",
                block.id, block.lines.0, block.lines.1
            ));

            output.push_str(&format!("    GEN:  {{{}}}\n", format_def_set(&block.gen)));
            output.push_str(&format!("    KILL: {{{}}}\n", format_def_set(&block.kill)));
            output.push_str(&format!(
                "    IN:   {{{}}}\n",
                format_def_set(&block.in_set)
            ));
            output.push_str(&format!("    OUT:  {{{}}}\n", format_def_set(&block.out)));
            output.push('\n');
        }
    }

    // Def-Use Chains
    if options.show_chains {
        output.push_str("Def-Use Chains:\n");
        if report.def_use_chains.is_empty() {
            output.push_str("    (none)\n");
        } else {
            for chain in &report.def_use_chains {
                let uses: Vec<String> = chain
                    .uses
                    .iter()
                    .map(|u| format!("line {}", u.line))
                    .collect();
                let uses_str = if uses.is_empty() {
                    "(unused)".to_string()
                } else {
                    uses.join(", ")
                };
                output.push_str(&format!(
                    "    {}@{} -> used at: {}\n",
                    chain.definition.var, chain.definition.line, uses_str
                ));
            }
        }
        output.push('\n');

        // Use-Def Chains
        output.push_str("Use-Def Chains:\n");
        if report.use_def_chains.is_empty() {
            output.push_str("    (none)\n");
        } else {
            for chain in &report.use_def_chains {
                let defs: Vec<String> = chain
                    .reaching_defs
                    .iter()
                    .map(|d| format!("line {}", d.line))
                    .collect();
                let defs_str = if defs.is_empty() {
                    "(no reaching definition)".to_string()
                } else {
                    defs.join(", ")
                };
                output.push_str(&format!(
                    "    {}@{} <- defined at: {}\n",
                    chain.var, chain.use_site.line, defs_str
                ));
            }
        }
        output.push('\n');
    }

    // Uninitialized Variables
    if options.show_uninitialized {
        output.push_str("Potentially Uninitialized:\n");
        if report.uninitialized.is_empty() {
            output.push_str("    (none detected)\n");
        } else {
            for uninit in &report.uninitialized {
                output.push_str(&format!(
                    "    {} at line {} ({}): {}\n",
                    uninit.var, uninit.line, uninit.severity, uninit.reason
                ));
            }
        }
        output.push('\n');
    }

    // Statistics
    if options.show_stats {
        output.push_str("---\n");
        output.push_str(&format!("Definitions: {}\n", report.stats.definitions));
        output.push_str(&format!("Uses: {}\n", report.stats.uses));
        output.push_str(&format!("Blocks: {}\n", report.stats.blocks));
        if report.stats.iterations > 0 {
            output.push_str(&format!("Iterations: {}\n", report.stats.iterations));
        }
        if report.stats.uninitialized_count > 0 {
            output.push_str(&format!(
                "Uninitialized: {}\n",
                report.stats.uninitialized_count
            ));
        }
    }

    output
}

/// Format reaching definitions report as human-readable text.
///
/// The output includes:
/// - Header with function name and file
/// - Per-block GEN/KILL/IN/OUT sets
/// - Def-use chains
/// - Use-def chains
/// - Uninitialized variable warnings
/// - Statistics summary
///
/// This is the original function that always shows all sections including
/// per-block details. For selective output, use
/// `format_reaching_defs_text_with_options`.
///
/// # Arguments
/// * `report` - The reaching definitions report to format
///
/// # Returns
/// * `String` - Formatted text output
///
/// # Example
/// ```ignore
/// let report = build_reaching_defs_report(&cfg, &refs, path);
/// let text = format_reaching_defs_text(&report);
/// println!("{}", text);
/// ```
pub fn format_reaching_defs_text(report: &ReachingDefsReport) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "Reaching Definitions for: {} in {}\n\n",
        report.function,
        report.file.display()
    ));

    // Blocks with GEN/KILL/IN/OUT sets
    for block in &report.blocks {
        output.push_str(&format!(
            "Block {} (lines {}-{}):\n",
            block.id, block.lines.0, block.lines.1
        ));

        output.push_str(&format!("    GEN:  {{{}}}\n", format_def_set(&block.gen)));
        output.push_str(&format!("    KILL: {{{}}}\n", format_def_set(&block.kill)));
        output.push_str(&format!(
            "    IN:   {{{}}}\n",
            format_def_set(&block.in_set)
        ));
        output.push_str(&format!("    OUT:  {{{}}}\n", format_def_set(&block.out)));
        output.push('\n');
    }

    // Def-Use Chains
    output.push_str("Def-Use Chains:\n");
    if report.def_use_chains.is_empty() {
        output.push_str("    (none)\n");
    } else {
        for chain in &report.def_use_chains {
            let uses: Vec<String> = chain
                .uses
                .iter()
                .map(|u| format!("line {}", u.line))
                .collect();
            let uses_str = if uses.is_empty() {
                "(unused)".to_string()
            } else {
                uses.join(", ")
            };
            output.push_str(&format!(
                "    {}@{} -> used at: {}\n",
                chain.definition.var, chain.definition.line, uses_str
            ));
        }
    }
    output.push('\n');

    // Use-Def Chains
    output.push_str("Use-Def Chains:\n");
    if report.use_def_chains.is_empty() {
        output.push_str("    (none)\n");
    } else {
        for chain in &report.use_def_chains {
            let defs: Vec<String> = chain
                .reaching_defs
                .iter()
                .map(|d| format!("line {}", d.line))
                .collect();
            let defs_str = if defs.is_empty() {
                "(no reaching definition)".to_string()
            } else {
                defs.join(", ")
            };
            output.push_str(&format!(
                "    {}@{} <- defined at: {}\n",
                chain.var, chain.use_site.line, defs_str
            ));
        }
    }
    output.push('\n');

    // Uninitialized Variables
    output.push_str("Potentially Uninitialized:\n");
    if report.uninitialized.is_empty() {
        output.push_str("    (none detected)\n");
    } else {
        for uninit in &report.uninitialized {
            output.push_str(&format!(
                "    {} at line {} ({}): {}\n",
                uninit.var, uninit.line, uninit.severity, uninit.reason
            ));
        }
    }
    output.push('\n');

    // Statistics
    output.push_str("---\n");
    output.push_str(&format!("Definitions: {}\n", report.stats.definitions));
    output.push_str(&format!("Uses: {}\n", report.stats.uses));
    output.push_str(&format!("Blocks: {}\n", report.stats.blocks));
    if report.stats.iterations > 0 {
        output.push_str(&format!("Iterations: {}\n", report.stats.iterations));
    }
    if report.stats.uninitialized_count > 0 {
        output.push_str(&format!(
            "Uninitialized: {}\n",
            report.stats.uninitialized_count
        ));
    }

    output
}

/// Format a set of definitions as "var@line, var@line, ..."
fn format_def_set(defs: &[Definition]) -> String {
    if defs.is_empty() {
        return String::new();
    }

    let mut parts: Vec<String> = defs
        .iter()
        .map(|d| format!("{}@{}", d.var, d.line))
        .collect();

    // Sort for consistent output
    parts.sort();

    parts.join(", ")
}

// =============================================================================
// Variable Filtering (RD-16)
// =============================================================================

/// Filter reaching definitions report to a specific variable.
///
/// Returns a new report containing only information about the specified variable:
/// - Blocks with GEN/KILL/IN/OUT filtered to that variable
/// - Def-use chains for that variable
/// - Use-def chains for that variable
/// - Uninitialized warnings for that variable
/// - Updated statistics
///
/// # Arguments
/// * `report` - The original report
/// * `var` - Variable name to filter by
///
/// # Returns
/// * `ReachingDefsReport` - Filtered report
///
/// # Example
/// ```ignore
/// let report = build_reaching_defs_report(&cfg, &refs, path);
/// let filtered = filter_reaching_defs_by_variable(&report, "x");
/// // filtered only contains info about variable "x"
/// ```
pub fn filter_reaching_defs_by_variable(
    report: &ReachingDefsReport,
    var: &str,
) -> ReachingDefsReport {
    // Filter blocks
    let blocks: Vec<BlockReachingDefs> = report
        .blocks
        .iter()
        .map(|b| BlockReachingDefs {
            id: b.id,
            lines: b.lines,
            gen: filter_definitions(&b.gen, var),
            kill: filter_definitions(&b.kill, var),
            in_set: filter_definitions(&b.in_set, var),
            out: filter_definitions(&b.out, var),
        })
        .collect();

    // Filter def-use chains
    let def_use_chains: Vec<DefUseChain> = report
        .def_use_chains
        .iter()
        .filter(|c| c.definition.var == var)
        .cloned()
        .collect();

    // Filter use-def chains
    let use_def_chains: Vec<UseDefChain> = report
        .use_def_chains
        .iter()
        .filter(|c| c.var == var)
        .cloned()
        .collect();

    // Filter uninitialized
    let uninitialized: Vec<UninitializedUse> = report
        .uninitialized
        .iter()
        .filter(|u| u.var == var)
        .cloned()
        .collect();

    // Compute filtered stats
    let definitions = def_use_chains.len();
    let uses = use_def_chains.len();
    let stats = ReachingDefsStats {
        definitions,
        uses,
        blocks: report.stats.blocks,
        iterations: report.stats.iterations,
        uninitialized_count: uninitialized.len(),
    };

    ReachingDefsReport {
        function: report.function.clone(),
        file: report.file.clone(),
        blocks,
        def_use_chains,
        use_def_chains,
        uninitialized,
        stats,
        uncertain_defs: report.uncertain_defs.clone(),
        confidence: report.confidence,
    }
}

/// Filter a list of definitions to only those matching a variable name
fn filter_definitions(defs: &[Definition], var: &str) -> Vec<Definition> {
    defs.iter().filter(|d| d.var == var).cloned().collect()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::super::chains::UninitSeverity;
    use super::*;
    use std::path::PathBuf;

    /// Create a test report for testing formatters
    fn test_report() -> ReachingDefsReport {
        ReachingDefsReport {
            function: "process_data".to_string(),
            file: PathBuf::from("src/main.py"),
            blocks: vec![
                BlockReachingDefs {
                    id: 0,
                    lines: (1, 3),
                    gen: vec![
                        Definition {
                            var: "x".to_string(),
                            line: 1,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                        Definition {
                            var: "y".to_string(),
                            line: 2,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                    ],
                    kill: vec![],
                    in_set: vec![],
                    out: vec![
                        Definition {
                            var: "x".to_string(),
                            line: 1,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                        Definition {
                            var: "y".to_string(),
                            line: 2,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                    ],
                },
                BlockReachingDefs {
                    id: 1,
                    lines: (4, 6),
                    gen: vec![Definition {
                        var: "z".to_string(),
                        line: 5,
                        column: Some(0),
                        block: 1,
                        source_text: None,
                    }],
                    kill: vec![],
                    in_set: vec![
                        Definition {
                            var: "x".to_string(),
                            line: 1,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                        Definition {
                            var: "y".to_string(),
                            line: 2,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                    ],
                    out: vec![
                        Definition {
                            var: "x".to_string(),
                            line: 1,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                        Definition {
                            var: "y".to_string(),
                            line: 2,
                            column: Some(0),
                            block: 0,
                            source_text: None,
                        },
                        Definition {
                            var: "z".to_string(),
                            line: 5,
                            column: Some(0),
                            block: 1,
                            source_text: None,
                        },
                    ],
                },
            ],
            def_use_chains: vec![
                DefUseChain {
                    definition: Definition {
                        var: "x".to_string(),
                        line: 1,
                        column: Some(0),
                        block: 0,
                        source_text: None,
                    },
                    uses: vec![super::super::chains::Use {
                        line: 5,
                        column: Some(4),
                        block: 1,
                        context: Some("z = x + y".to_string()),
                    }],
                },
                DefUseChain {
                    definition: Definition {
                        var: "y".to_string(),
                        line: 2,
                        column: Some(0),
                        block: 0,
                        source_text: None,
                    },
                    uses: vec![super::super::chains::Use {
                        line: 5,
                        column: Some(8),
                        block: 1,
                        context: Some("z = x + y".to_string()),
                    }],
                },
            ],
            use_def_chains: vec![
                UseDefChain {
                    use_site: super::super::chains::Use {
                        line: 5,
                        column: Some(4),
                        block: 1,
                        context: Some("z = x + y".to_string()),
                    },
                    var: "x".to_string(),
                    reaching_defs: vec![Definition {
                        var: "x".to_string(),
                        line: 1,
                        column: Some(0),
                        block: 0,
                        source_text: None,
                    }],
                },
                UseDefChain {
                    use_site: super::super::chains::Use {
                        line: 5,
                        column: Some(8),
                        block: 1,
                        context: Some("z = x + y".to_string()),
                    },
                    var: "y".to_string(),
                    reaching_defs: vec![Definition {
                        var: "y".to_string(),
                        line: 2,
                        column: Some(0),
                        block: 0,
                        source_text: None,
                    }],
                },
            ],
            uninitialized: vec![],
            stats: ReachingDefsStats {
                definitions: 3,
                uses: 2,
                blocks: 2,
                iterations: 2,
                uninitialized_count: 0,
            },
            uncertain_defs: vec![],
            confidence: super::super::chains::Confidence::High,
        }
    }

    /// Create a test report with uninitialized variable
    fn test_report_with_uninit() -> ReachingDefsReport {
        let mut report = test_report();
        report.uninitialized.push(UninitializedUse {
            var: "w".to_string(),
            line: 10,
            column: Some(0),
            block: 2,
            reason: "definition does not reach this use on all paths".to_string(),
            severity: UninitSeverity::Possible,
        });
        report.stats.uninitialized_count = 1;
        report
    }

    // =========================================================================
    // JSON Output Tests (RD-15)
    // =========================================================================

    #[test]
    fn test_json_output_valid() {
        let report = test_report();
        let json = format_reaching_defs_json(&report);

        assert!(json.is_ok(), "JSON serialization should succeed");
        let json_str = json.unwrap();
        assert!(!json_str.is_empty(), "JSON should not be empty");

        // Verify it's valid JSON by parsing
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        assert!(
            parsed.is_ok(),
            "JSON should be parseable: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn test_json_roundtrip() {
        let report = test_report();
        let json = format_reaching_defs_json(&report).unwrap();

        // Deserialize back
        let parsed: Result<ReachingDefsReport, _> = serde_json::from_str(&json);
        assert!(parsed.is_ok(), "Should deserialize: {:?}", parsed.err());

        let parsed = parsed.unwrap();
        assert_eq!(parsed.function, report.function);
        assert_eq!(parsed.stats.definitions, report.stats.definitions);
        assert_eq!(parsed.def_use_chains.len(), report.def_use_chains.len());
    }

    #[test]
    fn test_json_compact() {
        let report = test_report();
        let compact = format_reaching_defs_json_compact(&report).unwrap();
        let pretty = format_reaching_defs_json(&report).unwrap();

        // Compact should be shorter (no whitespace)
        assert!(compact.len() < pretty.len(), "Compact should be smaller");
        assert!(!compact.contains('\n'), "Compact should not have newlines");
    }

    #[test]
    fn test_json_contains_expected_fields() {
        let report = test_report();
        let json = format_reaching_defs_json(&report).unwrap();

        // Check for expected top-level fields
        assert!(
            json.contains("\"function\""),
            "Should contain function field"
        );
        assert!(json.contains("\"file\""), "Should contain file field");
        assert!(json.contains("\"blocks\""), "Should contain blocks field");
        assert!(
            json.contains("\"def_use_chains\""),
            "Should contain def_use_chains field"
        );
        assert!(
            json.contains("\"use_def_chains\""),
            "Should contain use_def_chains field"
        );
        assert!(
            json.contains("\"uninitialized\""),
            "Should contain uninitialized field"
        );
        assert!(json.contains("\"stats\""), "Should contain stats field");
    }

    #[test]
    fn test_json_in_field_renamed() {
        let report = test_report();
        let json = format_reaching_defs_json(&report).unwrap();

        // The 'in_set' field should be serialized as 'in' per the spec
        // (This is set by #[serde(rename = "in")] on BlockReachingDefs.in_set)
        assert!(
            json.contains("\"in\""),
            "in_set should be serialized as 'in'"
        );
    }

    // =========================================================================
    // Text Output Tests (RD-14)
    // =========================================================================

    #[test]
    fn test_text_format_header() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("Reaching Definitions for: process_data"),
            "Should have function name in header"
        );
        assert!(
            text.contains("src/main.py"),
            "Should have file path in header"
        );
    }

    #[test]
    fn test_text_format_blocks() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        // Should have block headers
        assert!(text.contains("Block 0"), "Should have Block 0");
        assert!(text.contains("Block 1"), "Should have Block 1");

        // Should have line ranges
        assert!(
            text.contains("lines 1-3"),
            "Should have line range for block 0"
        );
        assert!(
            text.contains("lines 4-6"),
            "Should have line range for block 1"
        );

        // Should have GEN/KILL/IN/OUT
        assert!(text.contains("GEN:"), "Should have GEN set");
        assert!(text.contains("KILL:"), "Should have KILL set");
        assert!(text.contains("IN:"), "Should have IN set");
        assert!(text.contains("OUT:"), "Should have OUT set");
    }

    #[test]
    fn test_text_format_def_use_chains() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("Def-Use Chains:"),
            "Should have def-use chains section"
        );
        assert!(text.contains("x@1 ->"), "Should have x@1 definition");
        assert!(text.contains("y@2 ->"), "Should have y@2 definition");
        assert!(text.contains("used at:"), "Should show uses");
        assert!(
            text.contains("line 5"),
            "Should show line 5 as use location"
        );
    }

    #[test]
    fn test_text_format_use_def_chains() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("Use-Def Chains:"),
            "Should have use-def chains section"
        );
        assert!(
            text.contains("<- defined at:"),
            "Should show definitions reaching use"
        );
    }

    #[test]
    fn test_text_format_uninit_none() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("Potentially Uninitialized:"),
            "Should have uninit section"
        );
        assert!(
            text.contains("(none detected)"),
            "Should say none detected when empty"
        );
    }

    #[test]
    fn test_text_format_uninit_present() {
        let report = test_report_with_uninit();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("w at line 10"),
            "Should show uninit variable w"
        );
        assert!(
            text.contains("definition does not reach"),
            "Should show reason"
        );
    }

    #[test]
    fn test_text_format_stats() {
        let report = test_report();
        let text = format_reaching_defs_text(&report);

        assert!(
            text.contains("Definitions: 3"),
            "Should show definition count"
        );
        assert!(text.contains("Uses: 2"), "Should show use count");
        assert!(text.contains("Blocks: 2"), "Should show block count");
        assert!(
            text.contains("Iterations: 2"),
            "Should show iteration count"
        );
    }

    // =========================================================================
    // Variable Filtering Tests (RD-16)
    // =========================================================================

    #[test]
    fn test_filter_by_variable() {
        let report = test_report();
        let filtered = filter_reaching_defs_by_variable(&report, "x");

        // Should only have chains for x
        assert!(
            filtered
                .def_use_chains
                .iter()
                .all(|c| c.definition.var == "x"),
            "All def-use chains should be for x"
        );
        assert!(
            filtered.use_def_chains.iter().all(|c| c.var == "x"),
            "All use-def chains should be for x"
        );
    }

    #[test]
    fn test_filter_by_variable_blocks() {
        let report = test_report();
        let filtered = filter_reaching_defs_by_variable(&report, "x");

        // Blocks should be filtered too
        for block in &filtered.blocks {
            assert!(
                block.gen.iter().all(|d| d.var == "x"),
                "GEN should only contain x"
            );
            assert!(
                block.kill.iter().all(|d| d.var == "x"),
                "KILL should only contain x"
            );
            assert!(
                block.in_set.iter().all(|d| d.var == "x"),
                "IN should only contain x"
            );
            assert!(
                block.out.iter().all(|d| d.var == "x"),
                "OUT should only contain x"
            );
        }
    }

    #[test]
    fn test_filter_by_variable_stats_updated() {
        let report = test_report();
        let filtered = filter_reaching_defs_by_variable(&report, "x");

        // Stats should reflect filtered counts
        assert_eq!(filtered.stats.definitions, 1, "Should have 1 x definition");
        assert_eq!(filtered.stats.uses, 1, "Should have 1 x use");
        // Block count stays the same
        assert_eq!(filtered.stats.blocks, report.stats.blocks);
    }

    #[test]
    fn test_filter_nonexistent_variable() {
        let report = test_report();
        let filtered = filter_reaching_defs_by_variable(&report, "nonexistent");

        // Should have empty chains
        assert!(filtered.def_use_chains.is_empty());
        assert!(filtered.use_def_chains.is_empty());
        assert_eq!(filtered.stats.definitions, 0);
        assert_eq!(filtered.stats.uses, 0);
    }

    #[test]
    fn test_filter_preserves_metadata() {
        let report = test_report();
        let filtered = filter_reaching_defs_by_variable(&report, "x");

        // Function name and file should be preserved
        assert_eq!(filtered.function, report.function);
        assert_eq!(filtered.file, report.file);
        // Number of blocks should be preserved
        assert_eq!(filtered.blocks.len(), report.blocks.len());
    }

    // =========================================================================
    // Edge Case Tests
    // =========================================================================

    #[test]
    fn test_empty_report() {
        let report = ReachingDefsReport::default();

        // JSON should work
        let json = format_reaching_defs_json(&report);
        assert!(json.is_ok());

        // Text should work
        let text = format_reaching_defs_text(&report);
        assert!(!text.is_empty());

        // Filter should work
        let filtered = filter_reaching_defs_by_variable(&report, "x");
        assert!(filtered.def_use_chains.is_empty());
    }

    #[test]
    fn test_format_def_set_empty() {
        let result = format_def_set(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_def_set_single() {
        let defs = vec![Definition {
            var: "x".to_string(),
            line: 1,
            column: None,
            block: 0,
            source_text: None,
        }];
        let result = format_def_set(&defs);
        assert_eq!(result, "x@1");
    }

    #[test]
    fn test_format_def_set_multiple() {
        let defs = vec![
            Definition {
                var: "y".to_string(),
                line: 2,
                column: None,
                block: 0,
                source_text: None,
            },
            Definition {
                var: "x".to_string(),
                line: 1,
                column: None,
                block: 0,
                source_text: None,
            },
        ];
        let result = format_def_set(&defs);
        // Should be sorted
        assert_eq!(result, "x@1, y@2");
    }

    // =========================================================================
    // Uncertain Findings Tests - ReachingDefsReport enrichment
    // =========================================================================

    #[test]
    fn test_reaching_defs_report_has_uncertain_fields() {
        use super::super::chains::Confidence;
        let report = ReachingDefsReport::default();
        assert!(report.uncertain_defs.is_empty());
        assert_eq!(report.confidence, Confidence::Low);
    }

    #[test]
    fn test_uncertain_def_construction() {
        use super::super::chains::UncertainDef;
        let ud = UncertainDef {
            var: "x".to_string(),
            line: 10,
            reason: "assignment pattern not recognized for this language".to_string(),
        };
        assert_eq!(ud.var, "x");
        assert_eq!(ud.line, 10);
        assert!(ud.reason.contains("not recognized"));
    }

    #[test]
    fn test_uncertain_def_serialization() {
        use super::super::chains::UncertainDef;
        let ud = UncertainDef {
            var: "result".to_string(),
            line: 25,
            reason: "complex destructuring assignment".to_string(),
        };
        let json = serde_json::to_string(&ud).unwrap();
        assert!(json.contains("\"var\""));
        assert!(json.contains("\"line\""));
        assert!(json.contains("\"reason\""));
        let deserialized: UncertainDef = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.var, ud.var);
    }

    // =========================================================================
    // Format Options Tests (chains-only / show-in-out)
    // =========================================================================

    #[test]
    fn test_format_options_default() {
        let opts = ReachingDefsFormatOptions::default();
        assert!(!opts.show_blocks, "Default should not show blocks");
        assert!(opts.show_chains, "Default should show chains");
        assert!(opts.show_uninitialized, "Default should show uninitialized");
        assert!(opts.show_header, "Default should show header");
        assert!(opts.show_stats, "Default should show stats");
    }

    #[test]
    fn test_chains_only_hides_blocks() {
        let report = test_report();
        let opts = ReachingDefsFormatOptions::chains_only();
        let text = format_reaching_defs_text_with_options(&report, &opts);

        // Should NOT have block details
        assert!(!text.contains("Block 0"), "chains_only should hide Block 0");
        assert!(!text.contains("Block 1"), "chains_only should hide Block 1");
        assert!(!text.contains("GEN:"), "chains_only should hide GEN");
        assert!(!text.contains("KILL:"), "chains_only should hide KILL");
        assert!(!text.contains("IN:"), "chains_only should hide IN sets");
        assert!(!text.contains("OUT:"), "chains_only should hide OUT sets");

        // Should have chains
        assert!(
            text.contains("Def-Use Chains:"),
            "chains_only should show def-use chains"
        );
        assert!(
            text.contains("Use-Def Chains:"),
            "chains_only should show use-def chains"
        );

        // Should NOT have header or stats
        assert!(
            !text.contains("Reaching Definitions for:"),
            "chains_only should hide header"
        );
        assert!(
            !text.contains("Definitions:"),
            "chains_only should hide stats"
        );
    }

    #[test]
    fn test_show_in_out_shows_blocks() {
        let report = test_report();
        let opts = ReachingDefsFormatOptions {
            show_blocks: true,
            show_chains: true,
            show_uninitialized: true,
            show_header: true,
            show_stats: true,
        };
        let text = format_reaching_defs_text_with_options(&report, &opts);

        // Should have block details
        assert!(text.contains("Block 0"), "show_blocks should show Block 0");
        assert!(text.contains("Block 1"), "show_blocks should show Block 1");
        assert!(text.contains("GEN:"), "show_blocks should show GEN");
        assert!(text.contains("KILL:"), "show_blocks should show KILL");
        assert!(text.contains("IN:"), "show_blocks should show IN sets");
        assert!(text.contains("OUT:"), "show_blocks should show OUT sets");

        // Should also have chains
        assert!(text.contains("Def-Use Chains:"), "should also show chains");
        assert!(
            text.contains("Use-Def Chains:"),
            "should also show use-def chains"
        );
    }

    #[test]
    fn test_default_hides_blocks() {
        let report = test_report();
        let opts = ReachingDefsFormatOptions::default();
        let text = format_reaching_defs_text_with_options(&report, &opts);

        // Default (show_blocks=false) should NOT show blocks
        assert!(!text.contains("Block 0"), "Default should hide Block 0");
        assert!(!text.contains("GEN:"), "Default should hide GEN");

        // Should show header + chains + stats
        assert!(
            text.contains("Reaching Definitions for:"),
            "Default should show header"
        );
        assert!(
            text.contains("Def-Use Chains:"),
            "Default should show chains"
        );
        assert!(text.contains("Definitions:"), "Default should show stats");
    }

    #[test]
    fn test_default_format_unchanged() {
        // The no-args format_reaching_defs_text should still work identically
        // (it always shows blocks for backward compat)
        let report = test_report();
        let text = format_reaching_defs_text(&report);
        assert!(
            text.contains("Block 0"),
            "Original function should still show blocks"
        );
        assert!(
            text.contains("GEN:"),
            "Original function should still show GEN"
        );
        assert!(
            text.contains("Def-Use Chains:"),
            "Original function should still show chains"
        );
    }

    #[test]
    fn test_reaching_defs_uncertain_in_json() {
        use super::super::chains::{Confidence, UncertainDef};
        let mut report = ReachingDefsReport {
            function: "test_func".to_string(),
            ..Default::default()
        };
        report.uncertain_defs.push(UncertainDef {
            var: "x".to_string(),
            line: 10,
            reason: "pattern not recognized".to_string(),
        });
        report.confidence = Confidence::Medium;

        let json_str = format_reaching_defs_json(&report).unwrap();
        let json: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(
            json.get("uncertain_defs").is_some(),
            "JSON should have uncertain_defs key"
        );
        assert!(
            json.get("confidence").is_some(),
            "JSON should have confidence key"
        );

        let uncertain = json["uncertain_defs"].as_array().unwrap();
        assert_eq!(uncertain.len(), 1);
        assert_eq!(uncertain[0]["var"], "x");
        assert_eq!(json["confidence"], "medium");
    }
}
