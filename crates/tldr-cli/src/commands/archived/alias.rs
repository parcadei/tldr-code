//! Alias analysis CLI command
//!
//! Provides Andersen-style flow-insensitive points-to analysis to determine
//! when two references may or must refer to the same object.
//!
//! # Usage
//!
//! ```bash
//! tldr alias <file> <function> [-f json|text|dot]
//! ```
//!
//! # Output
//!
//! - JSON: Full AliasInfo structure with may-alias, must-alias, points-to sets
//! - Text: Human-readable summary with alias pairs
//! - DOT: Graphviz visualization of alias relationships
//!
//! # Reference
//! - alias/spec.md

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::alias::{compute_alias_from_ssa, AliasInfo, AliasOutputFormat};
use tldr_core::ssa::{construct_minimal_ssa_with_statements, SsaFunction};
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Analyze alias relationships in a function using points-to analysis
#[derive(Debug, Args)]
pub struct AliasArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Check if two specific variables may alias (comma-separated: x_0,y_0)
    #[arg(long)]
    pub check: Option<String>,

    /// Show points-to set for a specific variable
    #[arg(long)]
    pub points_to: Option<String>,

    /// Show verbose output with allocation sites
    #[arg(long, short = 'v')]
    pub verbose: bool,
}

impl AliasArgs {
    /// Run the alias analysis command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        writer.progress(&format!(
            "Analyzing aliases for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Read source file - ensure it exists
        if !self.file.exists() {
            return Err(anyhow::anyhow!(
                "File not found: {}",
                self.file.display()
            ));
        }

        // Read source for statement-level SSA enrichment
        let source = std::fs::read_to_string(&self.file)?;
        let statements: Vec<String> = source.lines().map(|l| l.to_string()).collect();

        // Get CFG for the function
        let cfg = get_cfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Get DFG for variable references
        let dfg = get_dfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Construct SSA with source statements (required for alias analysis)
        let ssa: SsaFunction = construct_minimal_ssa_with_statements(&cfg, &dfg, &statements)?;

        // Run alias analysis
        let result = compute_alias_from_ssa(&ssa)
            .map_err(|e| anyhow::anyhow!("Alias analysis failed: {}", e))?;

        // Handle specific queries
        if let Some(ref check_vars) = self.check {
            return self.handle_check_query(&result, check_vars, &writer);
        }

        if let Some(ref var) = self.points_to {
            return self.handle_points_to_query(&result, var, &writer);
        }

        // Output based on format
        match format {
            OutputFormat::Text => {
                let text = result.to_text();
                writer.write_text(&text)?;
            }
            OutputFormat::Json | OutputFormat::Compact => {
                let json = result.to_json();
                writer.write_text(&json)?;
            }
            OutputFormat::Dot => {
                let dot = result.to_dot();
                writer.write_text(&dot)?;
            }
            OutputFormat::Sarif => {
                // SARIF not applicable for alias analysis, fall back to JSON
                let json = result.to_json();
                writer.write_text(&json)?;
            }
        }

        Ok(())
    }

    /// Handle --check query: check if two variables may/must alias
    fn handle_check_query(
        &self,
        result: &AliasInfo,
        check_vars: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        let parts: Vec<&str> = check_vars.split(',').collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!(
                "Invalid --check format. Expected: 'var1,var2'. Got: '{}'",
                check_vars
            ));
        }

        let var1 = parts[0].trim();
        let var2 = parts[1].trim();

        let may_alias = result.may_alias_check(var1, var2);
        let must_alias = result.must_alias_check(var1, var2);

        let output = serde_json::json!({
            "var1": var1,
            "var2": var2,
            "may_alias": may_alias,
            "must_alias": must_alias,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;

        Ok(())
    }

    /// Handle --points-to query: show points-to set for a variable
    fn handle_points_to_query(
        &self,
        result: &AliasInfo,
        var: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        let points_to = result.get_points_to(var);

        let mut sorted: Vec<_> = points_to.iter().cloned().collect();
        sorted.sort();

        let output = serde_json::json!({
            "variable": var,
            "points_to": sorted,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;

        Ok(())
    }
}
