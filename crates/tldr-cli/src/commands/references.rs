//! References command - Find all references to a symbol
//!
//! Wires tldr-core::analysis::references to the CLI (Session 7 Phase 12).
//!
//! # Features
//! - Text search with word boundary matching
//! - AST-based verification to filter false positives
//! - Reference kind classification (call, read, write, import, type)
//! - Search scope optimization based on symbol visibility
//! - Multiple output formats: JSON, text
//!
//! # Risk Mitigations
//! - S7-R31: Command registered in mod.rs and main.rs
//! - S7-R32: format_references_text implemented below
//! - S7-R33: find_references exported from tldr_core::analysis::references
//! - S7-R46: Tab alignment - expand tabs to spaces in context
//! - S7-R50: No suggestions on no match - suggest similar symbols
//! - S7-R51: Too many references - respect limit, show count
//! - S7-R56: Path not found error - include tried path in message
//! - S7-R57: Unsupported language - list supported languages in error

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::analysis::references::{
    find_references, ReferenceKind, ReferencesOptions, ReferencesReport, SearchScope,
};

use crate::output::{common_path_prefix, strip_prefix_display, OutputFormat, OutputWriter};

/// Find all references to a symbol
///
/// Search for all occurrences of a symbol (function, variable, class, etc.)
/// across the codebase using text search followed by AST verification.
///
/// # Examples
///
/// ```bash
/// # Find all references to 'analyze_dependencies'
/// tldr references analyze_dependencies .
///
/// # Include the definition in results
/// tldr references login . --include-definition
///
/// # Filter by reference kinds
/// tldr references process_data . --kinds call,import
///
/// # Output as text
/// tldr references MyClass . --format text
/// ```
#[derive(Debug, Args)]
pub struct ReferencesArgs {
    /// Symbol to find references for
    pub symbol: String,

    /// Path to search in (directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Output format override (backwards compatibility, prefer global --format/-f)
    #[arg(long = "output", short = 'o', hide = true)]
    pub output: Option<String>,

    /// Language filter: python, typescript, go, rust
    #[arg(long, short = 'l')]
    pub lang: Option<String>,

    /// Include definition location in results
    #[arg(long)]
    pub include_definition: bool,

    /// Filter by reference kinds (comma-separated: call,read,write,import,type)
    #[arg(long, short = 't')]
    pub kinds: Option<String>,

    /// Search scope: local, file, workspace
    #[arg(long, short = 's', default_value = "workspace")]
    pub scope: String,

    /// Maximum number of results to return
    #[arg(long, short = 'n', default_value = "20")]
    pub limit: usize,

    /// Number of context lines before and after (not implemented yet)
    #[arg(long, short = 'C', default_value = "0")]
    pub context_lines: usize,

    /// Minimum confidence threshold (0.0-1.0). References below this are filtered out.
    #[arg(long, default_value = "0.0")]
    pub min_confidence: f64,
}

impl ReferencesArgs {
    /// Run the references command
    pub fn run(&self, cli_format: OutputFormat, quiet: bool) -> Result<()> {
        // Validate path exists
        if !self.path.exists() {
            // S7-R56: Path not found error - include tried path in message
            anyhow::bail!(
                "Path not found: '{}'. Please provide a valid file or directory.",
                self.path.display()
            );
        }

        // Resolve format: hidden -o override takes precedence, else global -f
        let output_format = match self.output.as_deref() {
            Some("text") => OutputFormat::Text,
            Some("compact") => OutputFormat::Compact,
            Some("json") => OutputFormat::Json,
            _ => cli_format,
        };

        let writer = OutputWriter::new(output_format, quiet);

        // Parse reference kinds filter
        let kinds = self.kinds.as_ref().map(|k| parse_kinds(k));

        // Parse search scope
        let scope = parse_scope(&self.scope);

        // Build options
        let options = ReferencesOptions {
            include_definition: self.include_definition,
            kinds,
            scope,
            language: self.lang.clone(),
            limit: Some(self.limit),
            definition_file: None,
            context_lines: self.context_lines,
        };

        writer.progress(&format!(
            "Finding references to '{}' in {}...",
            self.symbol,
            self.path.display()
        ));

        // Run analysis
        let report = find_references(&self.symbol, &self.path, &options)?;

        // Filter by minimum confidence if specified
        let report = filter_by_min_confidence(report, self.min_confidence);

        // Output based on format
        match output_format {
            OutputFormat::Text => {
                let text = format_references_text(&report);
                writer.write_text(&text)?;
            }
            _ => {
                // JSON output (default)
                writer.write(&report)?;
            }
        }

        // S7-R50: If no references found, give helpful message
        if report.total_references == 0 && !quiet {
            eprintln!();
            eprintln!(
                "No references found for '{}'. Searched {} files.",
                self.symbol, report.stats.files_searched
            );
            eprintln!("Suggestions:");
            eprintln!("  - Check the symbol spelling");
            eprintln!("  - Try a different search scope with --scope workspace");
            eprintln!("  - Verify the path contains relevant source files");
        }

        Ok(())
    }
}

/// Parse comma-separated reference kinds
fn parse_kinds(s: &str) -> Vec<ReferenceKind> {
    s.split(',')
        .filter_map(|k| match k.trim().to_lowercase().as_str() {
            "call" => Some(ReferenceKind::Call),
            "read" => Some(ReferenceKind::Read),
            "write" => Some(ReferenceKind::Write),
            "import" => Some(ReferenceKind::Import),
            "type" => Some(ReferenceKind::Type),
            "definition" => Some(ReferenceKind::Definition),
            "other" => Some(ReferenceKind::Other),
            _ => None,
        })
        .collect()
}

/// Filter a report by minimum confidence threshold.
///
/// Removes references with confidence below `min_confidence` and
/// updates `total_references` to match. References with `None` confidence
/// are treated as 0.0.
fn filter_by_min_confidence(mut report: ReferencesReport, min_confidence: f64) -> ReferencesReport {
    if min_confidence > 0.0 {
        report
            .references
            .retain(|r| r.confidence.unwrap_or(0.0) >= min_confidence);
        report.total_references = report.references.len();
    }
    report
}

/// Parse search scope string
fn parse_scope(s: &str) -> SearchScope {
    match s.to_lowercase().as_str() {
        "local" => SearchScope::Local,
        "file" => SearchScope::File,
        _ => SearchScope::Workspace,
    }
}

/// Format the references report as human-readable text
///
/// # S7-R32: format_references_text implementation
/// # S7-R46: Tab alignment - expand tabs to spaces in context
fn format_references_text(report: &ReferencesReport) -> String {
    use std::path::Path;

    let mut output = String::new();

    // Collect all file paths (definition + references) to compute common prefix
    let mut all_paths: Vec<&Path> = report.references.iter().map(|r| r.file.as_path()).collect();
    if let Some(def) = &report.definition {
        all_paths.push(def.file.as_path());
    }
    let prefix = if all_paths.is_empty() {
        PathBuf::new()
    } else {
        common_path_prefix(&all_paths)
    };

    // Header
    output.push_str(&format!(
        "References to: {} ({})\n",
        report.symbol,
        report
            .definition
            .as_ref()
            .map(|d| d.kind.as_str())
            .unwrap_or("unknown")
    ));
    output.push('\n');

    // Definition (if found)
    if let Some(def) = &report.definition {
        output.push_str("Definition:\n");
        let def_display = strip_prefix_display(&def.file, &prefix);
        output.push_str(&format!(
            "  {}:{}:{} [{}]\n",
            def_display,
            def.line,
            def.column,
            def.kind.as_str()
        ));
        if let Some(sig) = &def.signature {
            // S7-R46: Expand tabs to spaces
            let sig_clean = sig.replace('\t', "    ");
            output.push_str(&format!("    {}\n", sig_clean.trim()));
        }
        output.push('\n');
    }

    // References
    output.push_str(&format!(
        "References ({} found in {}ms):\n",
        report.total_references, report.stats.search_time_ms
    ));

    for r in &report.references {
        let ref_display = strip_prefix_display(&r.file, &prefix);
        output.push_str(&format!(
            "  {}:{}:{} [{}]\n",
            ref_display,
            r.line,
            r.column,
            r.kind.as_str()
        ));
        // S7-R46: Expand tabs to spaces in context
        let context_clean = r.context.replace('\t', "    ");
        output.push_str(&format!("    {}\n", context_clean.trim()));
        output.push('\n');
    }

    // Stats
    output.push_str(&format!(
        "Search: {} files, {} candidates -> {} verified\n",
        report.stats.files_searched,
        report.stats.candidates_found,
        report.stats.verified_references
    ));
    output.push_str(&format!("Scope: {}\n", report.search_scope.as_str()));

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tldr_core::analysis::references::{Definition, DefinitionKind, Reference, ReferenceStats};

    fn make_test_report() -> ReferencesReport {
        ReferencesReport {
            symbol: "test_func".to_string(),
            definition: Some(Definition {
                file: PathBuf::from("src/lib.py"),
                line: 42,
                column: 5,
                kind: DefinitionKind::Function,
                signature: Some("def test_func(x: int) -> str:".to_string()),
            }),
            references: vec![
                Reference::new(
                    PathBuf::from("src/main.py"),
                    10,
                    8,
                    ReferenceKind::Call,
                    "result = test_func(42)".to_string(),
                ),
                Reference::new(
                    PathBuf::from("tests/test_lib.py"),
                    25,
                    12,
                    ReferenceKind::Import,
                    "from src.lib import test_func".to_string(),
                ),
            ],
            total_references: 2,
            search_scope: SearchScope::Workspace,
            stats: ReferenceStats {
                files_searched: 10,
                candidates_found: 5,
                verified_references: 2,
                search_time_ms: 127,
            },
        }
    }

    #[test]
    fn test_format_references_text() {
        let report = make_test_report();
        let text = format_references_text(&report);

        assert!(text.contains("References to: test_func (function)"));
        assert!(text.contains("Definition:"));
        assert!(text.contains("src/lib.py:42:5 [function]"));
        assert!(text.contains("def test_func(x: int) -> str:"));
        assert!(text.contains("References (2 found in 127ms)"));
        assert!(text.contains("src/main.py:10:8 [call]"));
        assert!(text.contains("tests/test_lib.py:25:12 [import]"));
        assert!(text.contains("Search: 10 files, 5 candidates -> 2 verified"));
        assert!(text.contains("Scope: workspace"));
    }

    #[test]
    fn test_parse_kinds() {
        let kinds = parse_kinds("call,import,type");
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&ReferenceKind::Call));
        assert!(kinds.contains(&ReferenceKind::Import));
        assert!(kinds.contains(&ReferenceKind::Type));
    }

    #[test]
    fn test_parse_kinds_case_insensitive() {
        let kinds = parse_kinds("CALL,Read,WRITE");
        assert_eq!(kinds.len(), 3);
        assert!(kinds.contains(&ReferenceKind::Call));
        assert!(kinds.contains(&ReferenceKind::Read));
        assert!(kinds.contains(&ReferenceKind::Write));
    }

    #[test]
    fn test_parse_scope() {
        assert_eq!(parse_scope("local"), SearchScope::Local);
        assert_eq!(parse_scope("file"), SearchScope::File);
        assert_eq!(parse_scope("workspace"), SearchScope::Workspace);
        assert_eq!(parse_scope("WORKSPACE"), SearchScope::Workspace);
        assert_eq!(parse_scope("unknown"), SearchScope::Workspace); // default
    }

    #[test]
    fn test_tab_expansion_in_context() {
        let mut report = make_test_report();
        report.references[0] = Reference::new(
            PathBuf::from("src/main.py"),
            10,
            8,
            ReferenceKind::Call,
            "\tresult = test_func(42)".to_string(), // Leading tab
        );

        let text = format_references_text(&report);
        // Tab should be expanded to 4 spaces
        assert!(text.contains("    result = test_func(42)"));
        assert!(!text.contains('\t'));
    }

    #[test]
    fn test_text_formatter_strips_common_path_prefix() {
        // Use absolute-like paths that share a common prefix
        let mut report = make_test_report();
        report.definition = Some(Definition {
            file: PathBuf::from("/home/user/project/src/lib.py"),
            line: 42,
            column: 5,
            kind: DefinitionKind::Function,
            signature: Some("def test_func(x: int) -> str:".to_string()),
        });
        report.references = vec![
            Reference::new(
                PathBuf::from("/home/user/project/src/main.py"),
                10,
                8,
                ReferenceKind::Call,
                "result = test_func(42)".to_string(),
            ),
            Reference::new(
                PathBuf::from("/home/user/project/tests/test_lib.py"),
                25,
                12,
                ReferenceKind::Import,
                "from src.lib import test_func".to_string(),
            ),
        ];

        let text = format_references_text(&report);

        // The common prefix /home/user/project/ should be stripped
        assert!(
            !text.contains("/home/user/project/"),
            "Text should not contain the absolute common prefix. Got:\n{}",
            text
        );
        // But the relative paths should be present
        assert!(text.contains("src/lib.py:42:5"));
        assert!(text.contains("src/main.py:10:8"));
        assert!(text.contains("tests/test_lib.py:25:12"));
    }

    #[test]
    fn test_default_limit_is_20() {
        // Verify the default limit arg is 20 by parsing default args
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            refs: ReferencesArgs,
        }

        let wrapper = Wrapper::parse_from(["test", "my_symbol"]);
        assert_eq!(
            wrapper.refs.limit, 20,
            "Default limit should be 20, got {}",
            wrapper.refs.limit
        );
    }

    #[test]
    fn test_min_confidence_filtering() {
        // Build a report with references at different confidence levels
        let report = ReferencesReport {
            symbol: "test_func".to_string(),
            definition: None,
            references: vec![
                Reference::with_details(
                    PathBuf::from("src/a.py"),
                    10,
                    1,
                    10,
                    ReferenceKind::Call,
                    "test_func()".to_string(),
                    1.0, // high confidence
                ),
                Reference::with_details(
                    PathBuf::from("src/b.py"),
                    20,
                    1,
                    10,
                    ReferenceKind::Call,
                    "test_func()".to_string(),
                    0.5, // medium confidence
                ),
                Reference::with_details(
                    PathBuf::from("src/c.py"),
                    30,
                    1,
                    10,
                    ReferenceKind::Call,
                    "test_func()".to_string(),
                    0.3, // low confidence
                ),
            ],
            total_references: 3,
            search_scope: SearchScope::Workspace,
            stats: ReferenceStats {
                files_searched: 5,
                candidates_found: 3,
                verified_references: 3,
                search_time_ms: 50,
            },
        };

        // Filter at 0.5 threshold should keep 2 references
        let filtered = filter_by_min_confidence(report.clone(), 0.5);
        assert_eq!(
            filtered.references.len(),
            2,
            "Should have 2 refs with confidence >= 0.5, got {}",
            filtered.references.len()
        );
        assert_eq!(
            filtered.total_references, 2,
            "total_references should be updated after filtering"
        );

        // Filter at 1.0 should keep only 1
        let filtered_high = filter_by_min_confidence(report.clone(), 1.0);
        assert_eq!(filtered_high.references.len(), 1);
        assert_eq!(filtered_high.total_references, 1);

        // Filter at 0.0 should keep all
        let filtered_none = filter_by_min_confidence(report, 0.0);
        assert_eq!(filtered_none.references.len(), 3);
        assert_eq!(filtered_none.total_references, 3);
    }

    #[test]
    fn test_kinds_short_flag_t() {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            refs: ReferencesArgs,
        }

        let wrapper = Wrapper::parse_from(["test", "my_symbol", ".", "-t", "call,import"]);
        assert_eq!(
            wrapper.refs.kinds.as_deref(),
            Some("call,import"),
            "--kinds should be settable via -t short flag"
        );
    }
}
