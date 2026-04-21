//! Architecture analysis CLI command (Phase 10)
//!
//! This command provides:
//! - Layer detection and architecture analysis
//! - Cycle detection using Tarjan SCC algorithm
//! - Rule generation (`--generate-rules`)
//! - Rule checking (`--check-rules`)
//!
//! Auto-routes through daemon when available for ~35x speedup (basic arch mode only).
//!
//! # Mitigations
//!
//! - A32: Memory limits via `--max-nodes`
//! - A33: Timeout via `--timeout`
//! - A1/A4: Tarjan SCC for cycle detection

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Args;

use tldr_core::analysis::{
    architecture_analysis, build_import_graph, check_rules, find_circular_dependencies_tarjan,
    generate_rules,
};
use tldr_core::callgraph::build_project_call_graph;
use tldr_core::limits::{AnalysisLimits, AnalysisProgress, TimeoutContext};
use tldr_core::types::{
    ArchRulesFile, ArchitectureReport, CycleGranularity, Language, RulesGenerationContext,
};

use crate::commands::daemon_router::{params_with_path, try_daemon_route};
use crate::output::OutputFormat;
use crate::signals::is_interrupted;

/// Architecture analysis command arguments
#[derive(Debug, Args)]
pub struct ArchArgs {
    /// Path to analyze (file or directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Detect cycles using Tarjan SCC algorithm
    #[arg(long)]
    pub cycles: bool,

    /// Cycle detection granularity: function or file
    #[arg(long, default_value = "file")]
    pub granularity: CycleGranularity,

    /// Generate architecture rules from detected layers
    #[arg(long)]
    pub generate_rules: bool,

    /// Check code against architecture rules file
    #[arg(long, value_name = "FILE")]
    pub check_rules: Option<PathBuf>,

    /// Exit 1 on any violation (including warnings)
    #[arg(long)]
    pub strict: bool,

    /// Check for transitive violations
    #[arg(long)]
    pub transitive: bool,

    /// Maximum nodes to process (memory limit)
    #[arg(long, default_value = "50000")]
    pub max_nodes: usize,

    /// Timeout in seconds (0 = no timeout)
    #[arg(long, default_value = "300")]
    pub timeout: u64,

    /// Maximum items per layer to display (truncation limit)
    #[arg(long, default_value = "100")]
    pub max_items: usize,
}

impl ArchArgs {
    /// Run the architecture analysis command.
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let start = Instant::now();

        // Try daemon first for basic arch analysis (no special modes)
        // Only route through daemon if no special flags are set
        let is_basic_mode = !self.cycles
            && !self.generate_rules
            && self.check_rules.is_none();

        if is_basic_mode {
            if let Some(arch_report) = try_daemon_route::<ArchitectureReport>(
                &self.path,
                "arch",
                params_with_path(Some(&self.path)),
            ) {
                let mut progress = AnalysisProgress::new();
                progress.set_elapsed(start.elapsed());
                return self.output_arch_report(&arch_report, format, &progress);
            }
        }

        // Fallback to direct compute

        // Set up limits
        let limits = AnalysisLimits::default()
            .with_max_nodes(self.max_nodes)
            .with_timeout(self.timeout);

        let timeout_ctx = TimeoutContext::new(self.timeout);
        let mut progress = AnalysisProgress::new();

        // Detect language
        let language = self.lang.unwrap_or_else(|| {
            if self.path.is_file() {
                Language::from_path(&self.path).unwrap_or(Language::Python)
            } else {
                Language::from_directory(&self.path).unwrap_or(Language::Python)
            }
        });

        // Build call graph
        if !quiet {
            eprintln!("Building call graph for {}...", self.path.display());
        }

        let call_graph = build_project_call_graph(&self.path, language, None, true)?;

        // Check timeout
        timeout_ctx.check().map_err(|e| anyhow::anyhow!("{}", e))?;

        // Check for interrupt
        if is_interrupted() {
            progress.truncate("Interrupted");
            return self.output_partial_results(format, &progress);
        }

        // Check node limit (count edges as proxy for complexity)
        let node_count = call_graph.edge_count();
        if node_count > limits.max_nodes {
            progress.truncate(format!(
                "Node limit exceeded: {} > {}",
                node_count, limits.max_nodes
            ));
            eprintln!(
                "Warning: Node limit exceeded ({} > {}). Use --max-nodes to increase.",
                node_count, limits.max_nodes
            );
        }

        progress.nodes_processed = node_count;

        // Run architecture analysis
        let arch_report = architecture_analysis(&call_graph)?;

        // Check timeout
        timeout_ctx.check().map_err(|e| anyhow::anyhow!("{}", e))?;

        // Handle different modes
        if let Some(rules_file) = &self.check_rules {
            // Check rules mode
            self.run_check_rules(rules_file, language, format, quiet)
        } else if self.generate_rules {
            // Generate rules mode
            self.run_generate_rules(&arch_report, format, quiet)
        } else if self.cycles {
            // Cycle detection mode
            self.run_cycle_detection(&call_graph, format, quiet)
        } else {
            // Default: architecture analysis
            progress.set_elapsed(start.elapsed());
            self.output_arch_report(&arch_report, format, &progress)
        }
    }

    /// Run cycle detection using Tarjan SCC.
    fn run_cycle_detection(
        &self,
        call_graph: &tldr_core::types::ProjectCallGraph,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<()> {
        if !quiet {
            eprintln!(
                "Detecting cycles at {:?} level using Tarjan SCC...",
                self.granularity
            );
        }

        let report = find_circular_dependencies_tarjan(call_graph, self.granularity);

        match format {
            OutputFormat::Json | OutputFormat::Sarif => {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            OutputFormat::Text => {
                println!("Cycle Detection Report");
                println!("======================");
                println!("Granularity: {:?}", report.granularity);
                println!("Cycles found: {}", report.summary.cycle_count);
                println!("Largest cycle: {} nodes", report.summary.largest_cycle);
                println!();

                if report.cycles.is_empty() {
                    println!("No cycles detected.");
                } else {
                    for (i, scc) in report.cycles.iter().enumerate() {
                        println!("Cycle {} ({} nodes):", i + 1, scc.size);
                        for node in &scc.nodes {
                            println!("  - {}", node);
                        }
                        println!();
                    }
                }
            }
            OutputFormat::Compact => {
                println!("{}", serde_json::to_string(&report)?);
            }
            OutputFormat::Dot => {
                // Dot format not applicable, use JSON
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }

        // Exit with error if cycles found
        if report.summary.cycle_count > 0 {
            std::process::exit(1);
        }

        Ok(())
    }

    /// Run rule generation.
    fn run_generate_rules(
        &self,
        arch_report: &tldr_core::types::ArchitectureReport,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<()> {
        if !quiet {
            eprintln!("Generating architecture rules...");
        }

        let context = RulesGenerationContext::new(self.path.clone());
        let rules = generate_rules(arch_report, &context);

        match format {
            OutputFormat::Json | OutputFormat::Sarif => {
                println!("{}", serde_json::to_string_pretty(&rules)?);
            }
            OutputFormat::Text => {
                // Output as YAML-like text for readability
                println!("# Architecture Rules");
                println!("# Generated from: {}", self.path.display());
                println!();
                println!("version: \"{}\"", rules.version);
                if let Some(ts) = &rules.generated_at {
                    println!("generated_at: \"{}\"", ts);
                }
                println!();
                println!("layers:");
                if let Some(high) = &rules.layers.high {
                    println!("  high:");
                    println!("    description: \"{}\"", high.description);
                    println!("    directories:");
                    for dir in &high.directories {
                        println!("      - \"{}\"", dir);
                    }
                }
                if let Some(middle) = &rules.layers.middle {
                    println!("  middle:");
                    println!("    description: \"{}\"", middle.description);
                    println!("    directories:");
                    for dir in &middle.directories {
                        println!("      - \"{}\"", dir);
                    }
                }
                if let Some(low) = &rules.layers.low {
                    println!("  low:");
                    println!("    description: \"{}\"", low.description);
                    println!("    directories:");
                    for dir in &low.directories {
                        println!("      - \"{}\"", dir);
                    }
                }
                println!();
                println!("rules:");
                for rule in &rules.rules {
                    println!("  - id: \"{}\"", rule.id);
                    println!("    constraint: \"{}\"", rule.constraint);
                    println!("    severity: {}", rule.severity);
                    println!("    rationale: \"{}\"", rule.rationale);
                    println!();
                }
            }
            OutputFormat::Compact => {
                println!("{}", serde_json::to_string(&rules)?);
            }
            OutputFormat::Dot => {
                // Dot format not applicable, use JSON
                println!("{}", serde_json::to_string_pretty(&rules)?);
            }
        }

        Ok(())
    }

    /// Run rule checking.
    fn run_check_rules(
        &self,
        rules_file: &PathBuf,
        language: Language,
        format: OutputFormat,
        quiet: bool,
    ) -> Result<()> {
        if !quiet {
            eprintln!("Checking architecture rules from {}...", rules_file.display());
        }

        // Load rules
        let rules_content = std::fs::read_to_string(rules_file)?;
        let rules: ArchRulesFile = if rules_file.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            serde_yaml::from_str(&rules_content)?
        } else {
            serde_json::from_str(&rules_content)?
        };

        // Build import graph (not call graph - A22)
        if !quiet {
            eprintln!("Building import graph...");
        }
        let import_graph = build_import_graph(&self.path, language)?;

        // Get layer mappings from architecture analysis
        let call_graph = build_project_call_graph(&self.path, language, None, true)?;
        let arch_report = architecture_analysis(&call_graph)?;

        // Check rules
        let mut report = check_rules(&rules, &import_graph, &arch_report.inferred_layers);

        // Check transitive violations if requested
        if self.transitive {
            let transitive_violations = tldr_core::analysis::check_transitive_violations(
                &rules,
                &import_graph,
                &arch_report.inferred_layers,
            );
            for v in transitive_violations {
                report.add_violation(v);
            }
        }

        match format {
            OutputFormat::Json | OutputFormat::Sarif => {
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
            OutputFormat::Text => {
                println!("Architecture Rules Check");
                println!("========================");
                println!("Rules checked: {}", report.summary.rules_checked);
                println!("Files scanned: {}", report.summary.files_scanned);
                println!();

                if report.pass {
                    println!("Result: PASS");
                    println!("No violations found.");
                } else {
                    println!("Result: FAIL");
                    println!("Errors: {}", report.summary.error_count);
                    println!("Warnings: {}", report.summary.warn_count);
                    println!();

                    for v in &report.violations {
                        let marker = if v.severity == tldr_core::types::RuleSeverity::Error {
                            "ERROR"
                        } else {
                            "WARN"
                        };
                        println!(
                            "[{}] {}: {} imports {} ({} -> {})",
                            marker,
                            v.rule_id,
                            v.from_file.display(),
                            v.imports_file.display(),
                            v.from_layer,
                            v.to_layer
                        );
                        if v.transitive && !v.path.is_empty() {
                            print!("  Path: ");
                            for (i, p) in v.path.iter().enumerate() {
                                if i > 0 {
                                    print!(" -> ");
                                }
                                print!("{}", p.display());
                            }
                            println!();
                        }
                    }
                }
            }
            OutputFormat::Compact => {
                println!("{}", serde_json::to_string(&report)?);
            }
            OutputFormat::Dot => {
                // Dot format not applicable, use JSON
                println!("{}", serde_json::to_string_pretty(&report)?);
            }
        }

        // Exit code
        if !report.pass {
            std::process::exit(1);
        } else if self.strict && report.has_violations() {
            std::process::exit(1);
        }

        Ok(())
    }

    /// Output architecture report.
    fn output_arch_report(
        &self,
        report: &tldr_core::types::ArchitectureReport,
        format: OutputFormat,
        progress: &AnalysisProgress,
    ) -> Result<()> {
        // Calculate truncation for each layer
        let entry_total = report.entry_layer.len();
        let middle_total = report.middle_layer.len();
        let leaf_total = report.leaf_layer.len();
        let max_items = self.max_items;

        let entry_shown = max_items.min(entry_total);
        let middle_shown = max_items.min(middle_total);
        let leaf_shown = max_items.min(leaf_total);
        let is_truncated = entry_shown < entry_total || middle_shown < middle_total || leaf_shown < leaf_total;

        match format {
            OutputFormat::Json | OutputFormat::Sarif => {
                #[derive(serde::Serialize)]
                struct Output<'a> {
                    report: TruncatedArchReport<'a>,
                    progress: &'a AnalysisProgress,
                }
                #[derive(serde::Serialize)]
                struct TruncatedArchReport<'a> {
                    entry_layer: &'a [tldr_core::types::FunctionRef],
                    middle_layer: &'a [tldr_core::types::FunctionRef],
                    leaf_layer: &'a [tldr_core::types::FunctionRef],
                    directories: &'a std::collections::HashMap<std::path::PathBuf, tldr_core::types::DirStats>,
                    circular_dependencies: &'a [tldr_core::types::CircularDep],
                    inferred_layers: &'a std::collections::HashMap<std::path::PathBuf, tldr_core::types::LayerType>,
                    truncated: bool,
                    total_counts: ArchCounts,
                    shown_counts: ArchCounts,
                }
                #[derive(serde::Serialize)]
                struct ArchCounts {
                    entry: usize,
                    middle: usize,
                    leaf: usize,
                }

                let truncated_report = TruncatedArchReport {
                    entry_layer: &report.entry_layer[..entry_shown],
                    middle_layer: &report.middle_layer[..middle_shown],
                    leaf_layer: &report.leaf_layer[..leaf_shown],
                    directories: &report.directories,
                    circular_dependencies: &report.circular_dependencies,
                    inferred_layers: &report.inferred_layers,
                    truncated: is_truncated,
                    total_counts: ArchCounts {
                        entry: entry_total,
                        middle: middle_total,
                        leaf: leaf_total,
                    },
                    shown_counts: ArchCounts {
                        entry: entry_shown,
                        middle: middle_shown,
                        leaf: leaf_shown,
                    },
                };

                let output = Output {
                    report: truncated_report,
                    progress,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
            OutputFormat::Text => {
                println!("Architecture Analysis");
                println!("=====================");
                println!();

                println!("Layers detected:");
                if is_truncated {
                    println!("  Entry layer: {} functions (showing {})", entry_total, entry_shown);
                    println!("  Middle layer: {} functions (showing {})", middle_total, middle_shown);
                    println!("  Leaf layer: {} functions (showing {})", leaf_total, leaf_shown);
                } else {
                    println!("  Entry layer: {} functions", entry_total);
                    println!("  Middle layer: {} functions", middle_total);
                    println!("  Leaf layer: {} functions", leaf_total);
                }
                println!();

                if entry_shown > 0 {
                    println!("Entry layer functions:");
                    for func in report.entry_layer.iter().take(entry_shown) {
                        println!("  - {} (in {})", func.name, func.file.display());
                    }
                    println!();
                }

                if middle_shown > 0 {
                    println!("Middle layer functions:");
                    for func in report.middle_layer.iter().take(middle_shown) {
                        println!("  - {} (in {})", func.name, func.file.display());
                    }
                    println!();
                }

                if leaf_shown > 0 {
                    println!("Leaf layer functions:");
                    for func in report.leaf_layer.iter().take(leaf_shown) {
                        println!("  - {} (in {})", func.name, func.file.display());
                    }
                    println!();
                }

                println!("Directory classification:");
                for (dir, layer) in &report.inferred_layers {
                    println!("  {}: {:?}", dir.display(), layer);
                }
                println!();

                if !report.circular_dependencies.is_empty() {
                    println!(
                        "Circular dependencies: {} found",
                        report.circular_dependencies.len()
                    );
                    for dep in &report.circular_dependencies {
                        println!("  {} <-> {}", dep.a.display(), dep.b.display());
                    }
                } else {
                    println!("Circular dependencies: none");
                }

                if is_truncated {
                    println!();
                    println!("Note: Output truncated. Use --max-items to show more items per layer.");
                }

                if progress.truncated {
                    println!();
                    println!(
                        "Warning: {}",
                        progress.truncation_reason.as_deref().unwrap_or("Analysis truncated")
                    );
                }
            }
            OutputFormat::Compact => {
                #[derive(serde::Serialize)]
                struct TruncatedCompactReport<'a> {
                    entry_layer: &'a [tldr_core::types::FunctionRef],
                    middle_layer: &'a [tldr_core::types::FunctionRef],
                    leaf_layer: &'a [tldr_core::types::FunctionRef],
                    inferred_layers: &'a std::collections::HashMap<std::path::PathBuf, tldr_core::types::LayerType>,
                    truncated: bool,
                    total_counts: ArchCounts,
                    shown_counts: ArchCounts,
                }
                #[derive(serde::Serialize)]
                struct ArchCounts {
                    entry: usize,
                    middle: usize,
                    leaf: usize,
                }

                let truncated_report = TruncatedCompactReport {
                    entry_layer: &report.entry_layer[..entry_shown],
                    middle_layer: &report.middle_layer[..middle_shown],
                    leaf_layer: &report.leaf_layer[..leaf_shown],
                    inferred_layers: &report.inferred_layers,
                    truncated: is_truncated,
                    total_counts: ArchCounts {
                        entry: entry_total,
                        middle: middle_total,
                        leaf: leaf_total,
                    },
                    shown_counts: ArchCounts {
                        entry: entry_shown,
                        middle: middle_shown,
                        leaf: leaf_shown,
                    },
                };
                println!("{}", serde_json::to_string(&truncated_report)?);
            }
            OutputFormat::Dot => {
                // Dot format not applicable, use JSON with truncation info
                #[derive(serde::Serialize)]
                struct TruncatedDotReport<'a> {
                    entry_layer: &'a [tldr_core::types::FunctionRef],
                    middle_layer: &'a [tldr_core::types::FunctionRef],
                    leaf_layer: &'a [tldr_core::types::FunctionRef],
                    directories: &'a std::collections::HashMap<std::path::PathBuf, tldr_core::types::DirStats>,
                    circular_dependencies: &'a [tldr_core::types::CircularDep],
                    inferred_layers: &'a std::collections::HashMap<std::path::PathBuf, tldr_core::types::LayerType>,
                    truncated: bool,
                    total_counts: ArchCounts,
                    shown_counts: ArchCounts,
                }
                #[derive(serde::Serialize)]
                struct ArchCounts {
                    entry: usize,
                    middle: usize,
                    leaf: usize,
                }

                let truncated_report = TruncatedDotReport {
                    entry_layer: &report.entry_layer[..entry_shown],
                    middle_layer: &report.middle_layer[..middle_shown],
                    leaf_layer: &report.leaf_layer[..leaf_shown],
                    directories: &report.directories,
                    circular_dependencies: &report.circular_dependencies,
                    inferred_layers: &report.inferred_layers,
                    truncated: is_truncated,
                    total_counts: ArchCounts {
                        entry: entry_total,
                        middle: middle_total,
                        leaf: leaf_total,
                    },
                    shown_counts: ArchCounts {
                        entry: entry_shown,
                        middle: middle_shown,
                        leaf: leaf_shown,
                    },
                };
                println!("{}", serde_json::to_string_pretty(&truncated_report)?);
            }
        }

        Ok(())
    }

    /// Output partial results when interrupted or limit exceeded.
    fn output_partial_results(&self, format: OutputFormat, progress: &AnalysisProgress) -> Result<()> {
        match format {
            OutputFormat::Json | OutputFormat::Compact | OutputFormat::Sarif | OutputFormat::Dot => {
                println!("{}", serde_json::to_string_pretty(&progress)?);
            }
            OutputFormat::Text => {
                println!("Analysis incomplete");
                println!("==================");
                if let Some(reason) = &progress.truncation_reason {
                    println!("Reason: {}", reason);
                }
                println!("Nodes processed: {}", progress.nodes_processed);
            }
        }
        Ok(())
    }
}
