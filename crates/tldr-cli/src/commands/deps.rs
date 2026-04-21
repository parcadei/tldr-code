//! Dependency Analysis command - Build and analyze import dependency graphs
//!
//! Wires tldr-core::analysis::deps to the CLI (Session 7 Phase 5).
//!
//! # Features
//! - Internal dependency graph building
//! - Circular dependency detection
//! - External dependency tracking (optional)
//! - Package-level collapsing (optional)
//! - Multiple output formats: JSON, text, DOT
//!
//! # Risk Mitigations
//! - S7-R31: Command registered in mod.rs and main.rs
//! - S7-R33: analyze_dependencies exported from tldr_core::analysis::deps
//! - S7-R34: Uses Language enum from tldr_core
//! - S7-R35: DepsOptions has pub fields
//! - S7-R36: Exit codes documented

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::analysis::deps::{
    analyze_dependencies, format_deps_dot, format_deps_text, DepsOptions,
};
use tldr_core::Language;

use crate::output::{OutputFormat, OutputWriter};

/// Analyze module dependencies
///
/// Build import dependency graphs for a project, detect circular dependencies,
/// and optionally include external (third-party) dependencies.
///
/// # Examples
///
/// ```bash
/// # Analyze current directory
/// tldr deps
///
/// # Analyze with external dependencies
/// tldr deps --include-external
///
/// # Only show circular dependencies
/// tldr deps --show-cycles
///
/// # Output as text
/// tldr deps -f text
///
/// # Output as DOT for graphviz
/// tldr deps -f dot | dot -Tpng -o deps.png
/// ```
#[derive(Debug, Args)]
pub struct DepsArgs {
    /// Path to analyze (directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Output format override (backwards compatibility, prefer global --format/-f)
    #[arg(long = "output", short = 'o', hide = true)]
    pub output: Option<String>,

    /// Programming language filter: python, typescript, go, rust
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    // === Dependency Options ===
    /// Include external (third-party) dependencies in the report
    #[arg(long)]
    pub include_external: bool,

    /// Collapse files into package-level nodes
    #[arg(long)]
    pub collapse_packages: bool,

    /// Maximum transitive depth (None = unlimited)
    #[arg(long, short = 'd')]
    pub depth: Option<usize>,

    // === Cycle Detection ===
    /// Only show circular dependencies (skip full graph)
    #[arg(long)]
    pub show_cycles: bool,

    /// Maximum cycle length to report (default: 10)
    #[arg(long, default_value = "10")]
    pub max_cycle_length: usize,
}

impl DepsArgs {
    /// Resolve the effective output format.
    ///
    /// If the user passed the hidden backward-compat `--output`/`-o` flag,
    /// that value takes precedence. Otherwise the global `--format`/`-f`
    /// value is used.
    pub fn effective_format(&self, global: OutputFormat) -> OutputFormat {
        match self.output.as_deref() {
            Some("text") => OutputFormat::Text,
            Some("dot") => OutputFormat::Dot,
            Some("compact") => OutputFormat::Compact,
            Some("json") => OutputFormat::Json,
            Some(_) => global, // Unknown value falls back to global
            None => global,
        }
    }

    /// Run the deps command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let effective = self.effective_format(format);
        let writer = OutputWriter::new(
            if effective == OutputFormat::Dot {
                OutputFormat::Text // DOT is text-based rendering
            } else {
                effective
            },
            quiet,
        );

        // Build options from args
        let options = DepsOptions {
            include_external: self.include_external,
            collapse_packages: self.collapse_packages,
            max_depth: self.depth,
            show_cycles_only: self.show_cycles,
            max_cycle_length: Some(self.max_cycle_length),
            language: self.lang.as_ref().map(|l| l.as_str().to_string()),
        };

        writer.progress(&format!(
            "Analyzing dependencies in {}...",
            self.path.display()
        ));

        // Run analysis
        let report = analyze_dependencies(&self.path, &options)?;

        // Output based on effective format
        match effective {
            OutputFormat::Dot => {
                let dot = format_deps_dot(&report);
                println!("{}", dot);
            }
            OutputFormat::Text => {
                let text = format_deps_text(&report);
                writer.write_text(&text)?;
            }
            _ => {
                // JSON/Compact/Sarif output
                if self.show_cycles {
                    // Only output cycles when --show-cycles is specified
                    writer.write(&report.circular_dependencies)?;
                } else {
                    writer.write(&report)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tldr_core::analysis::deps::{DepCycle, DepStats, DepsReport};

    fn make_test_report() -> DepsReport {
        let mut internal_deps = BTreeMap::new();
        internal_deps.insert(
            PathBuf::from("src/auth.py"),
            vec![PathBuf::from("src/utils.py"), PathBuf::from("src/db.py")],
        );
        internal_deps.insert(PathBuf::from("src/utils.py"), vec![]);
        internal_deps.insert(
            PathBuf::from("src/db.py"),
            vec![PathBuf::from("src/utils.py")],
        );

        DepsReport {
            root: PathBuf::from("src"),
            language: "python".to_string(),
            internal_dependencies: internal_deps,
            external_dependencies: BTreeMap::new(),
            circular_dependencies: vec![],
            stats: DepStats {
                total_files: 3,
                total_internal_deps: 3,
                total_external_deps: 0,
                max_depth: 2,
                cycles_found: 0,
                leaf_files: 1,
                root_files: 1,
            },
        }
    }

    #[test]
    fn test_format_deps_text() {
        let report = make_test_report();
        let text = format_deps_text(&report);

        // Updated assertions for new spec-compliant format
        assert!(text.contains("Dependency Analysis: src"));
        assert!(text.contains("Language: Python")); // Capitalized per spec
        assert!(text.contains("Internal Dependencies (3 edges, 3 files)"));
        assert!(text.contains("No circular dependencies found"));
    }

    #[test]
    fn test_format_deps_text_with_cycles() {
        let mut report = make_test_report();
        report.circular_dependencies = vec![DepCycle::new(vec![
            PathBuf::from("src/a.py"),
            PathBuf::from("src/b.py"),
            PathBuf::from("src/c.py"),
        ])];
        report.stats.cycles_found = 1;

        let text = format_deps_text(&report);
        assert!(text.contains("[CYCLE]")); // Spec format without number
        assert!(text.contains("src/a.py -> src/b.py -> src/c.py"));
    }

    #[test]
    fn test_format_deps_dot() {
        let report = make_test_report();
        let dot = format_deps_dot(&report);

        assert!(dot.contains("digraph deps {"));
        assert!(dot.contains("rankdir=LR"));
        assert!(dot.contains("\"src/auth.py\" -> \"src/utils.py\""));
        assert!(dot.contains("\"src/auth.py\" -> \"src/db.py\""));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn test_output_field_is_optional() {
        // The output field should be None when not explicitly provided,
        // allowing the global --format flag to take effect.
        let args = DepsArgs {
            path: PathBuf::from("."),
            output: None,
            lang: None,
            include_external: false,
            collapse_packages: false,
            depth: None,
            show_cycles: false,
            max_cycle_length: 10,
        };
        assert!(args.output.is_none(), "output should be None when not set");
    }

    #[test]
    fn test_output_field_backward_compat_override() {
        // When -o is explicitly provided, it should override the global format.
        let args = DepsArgs {
            path: PathBuf::from("."),
            output: Some("text".to_string()),
            lang: None,
            include_external: false,
            collapse_packages: false,
            depth: None,
            show_cycles: false,
            max_cycle_length: 10,
        };
        assert_eq!(
            args.output.as_deref(),
            Some("text"),
            "output should contain the explicit value"
        );
    }

    #[test]
    fn test_effective_format_uses_global_when_no_local() {
        // When output is None, effective_format should return the global format.
        let args = DepsArgs {
            path: PathBuf::from("."),
            output: None,
            lang: None,
            include_external: false,
            collapse_packages: false,
            depth: None,
            show_cycles: false,
            max_cycle_length: 10,
        };
        let effective = args.effective_format(OutputFormat::Text);
        assert_eq!(
            effective,
            OutputFormat::Text,
            "should use global format when no local override"
        );
    }

    #[test]
    fn test_effective_format_uses_local_override() {
        // When output is Some("text"), effective_format should return Text.
        let args = DepsArgs {
            path: PathBuf::from("."),
            output: Some("text".to_string()),
            lang: None,
            include_external: false,
            collapse_packages: false,
            depth: None,
            show_cycles: false,
            max_cycle_length: 10,
        };
        let effective = args.effective_format(OutputFormat::Json);
        assert_eq!(
            effective,
            OutputFormat::Text,
            "should use local text override"
        );
    }

    #[test]
    fn test_effective_format_dot_override() {
        // When output is Some("dot"), effective_format should return Dot.
        let args = DepsArgs {
            path: PathBuf::from("."),
            output: Some("dot".to_string()),
            lang: None,
            include_external: false,
            collapse_packages: false,
            depth: None,
            show_cycles: false,
            max_cycle_length: 10,
        };
        let effective = args.effective_format(OutputFormat::Json);
        assert_eq!(
            effective,
            OutputFormat::Dot,
            "should use local dot override"
        );
    }

    #[test]
    fn test_format_deps_dot_cycle_highlighting() {
        let mut report = make_test_report();

        // Add a cycle: a -> b -> a
        let mut deps = BTreeMap::new();
        deps.insert(PathBuf::from("src/a.py"), vec![PathBuf::from("src/b.py")]);
        deps.insert(PathBuf::from("src/b.py"), vec![PathBuf::from("src/a.py")]);
        report.internal_dependencies = deps;
        report.circular_dependencies = vec![DepCycle::new(vec![
            PathBuf::from("src/a.py"),
            PathBuf::from("src/b.py"),
        ])];

        let dot = format_deps_dot(&report);

        // Cycle edges should be highlighted in red (per spec)
        assert!(dot.contains("color=red") || dot.contains("color=\"red\""));
        assert!(dot.contains("penwidth=2"));
    }
}
