//! Available Expressions Analysis CLI command
//!
//! Computes available expressions at each program point for Common Subexpression
//! Elimination (CSE) detection.
//!
//! # Usage
//!
//! ```bash
//! tldr available <file> <function> [-f json|text]
//! tldr available src/main.py process_data --check "a + b"
//! tldr available src/main.py process_data --at_line 42
//! ```
//!
//! # Output
//!
//! - JSON: Full AvailableExprsInfo with avail_in/avail_out per block
//! - Text: Human-readable summary with redundant computations highlighted
//!
//! # Reference
//! - dataflow/spec.md (CAP-AE-01 through CAP-AE-12)

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::dataflow::{compute_available_exprs_with_source_and_lang, AvailableExprsInfo};
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Analyze available expressions for CSE detection
#[derive(Debug, Args)]
pub struct AvailableArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Check if a specific expression is available (e.g., "a + b")
    #[arg(long)]
    pub check: Option<String>,

    /// Show expressions available at a specific line number
    #[arg(long)]
    pub at_line: Option<usize>,

    /// Show what kills a specific expression
    #[arg(long)]
    pub killed_by: Option<String>,

    /// Show only CSE opportunities, skip per-block details
    #[arg(long)]
    pub cse_only: bool,
}

impl AvailableArgs {
    /// Run the available expressions analysis command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_path(&self.file).unwrap_or(Language::Python));

        writer.progress(&format!(
            "Analyzing available expressions for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Read source file - ensure it exists
        if !self.file.exists() {
            return Err(anyhow::anyhow!("File not found: {}", self.file.display()));
        }

        // Read source for line mapping
        let source = std::fs::read_to_string(&self.file)?;
        let source_lines: Vec<String> = source.lines().map(|s| s.to_string()).collect();

        // Get CFG for the function
        let cfg = get_cfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Get DFG for expression extraction
        let dfg = get_dfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Compute available expressions (with AST-based extraction for multi-language support)
        let result = compute_available_exprs_with_source_and_lang(
            &cfg,
            &dfg,
            &source_lines,
            Some(language),
        )?;

        // Handle specific queries
        if let Some(ref expr) = self.check {
            return self.handle_check_query(&result, expr, &writer);
        }

        if let Some(line) = self.at_line {
            return self.handle_at_line_query(&result, line, &writer);
        }

        if let Some(ref expr) = self.killed_by {
            return self.handle_killed_by_query(&result, expr, &writer);
        }

        // Default: output full result
        match format {
            OutputFormat::Json => {
                let json = serde_json::to_string_pretty(&result)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            OutputFormat::Text => {
                let text = self.format_text_output(&result);
                writer.write_text(&text)?;
            }
            OutputFormat::Compact => {
                let json = serde_json::to_string(&result)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            _ => {
                let json = serde_json::to_string_pretty(&result)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
        }

        Ok(())
    }

    fn handle_check_query(
        &self,
        result: &AvailableExprsInfo,
        expr: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Find blocks where this expression is available
        let mut available_in_blocks = Vec::new();

        for (block_id, exprs) in &result.avail_in {
            if exprs.iter().any(|e| e.text == expr) {
                available_in_blocks.push(*block_id);
            }
        }

        let output = serde_json::json!({
            "expression": expr,
            "available_in_blocks": available_in_blocks,
            "is_redundant": result.redundant_computations().iter().any(|(text, _, _)| text == expr),
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_at_line_query(
        &self,
        result: &AvailableExprsInfo,
        line: usize,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Find expressions available at the given line
        let mut available_exprs = Vec::new();

        for exprs in result.avail_in.values() {
            for expr in exprs {
                if expr.line <= line && !available_exprs.contains(&expr.text) {
                    available_exprs.push(expr.text.clone());
                }
            }
        }

        // Also check avail_out for completeness
        for exprs in result.avail_out.values() {
            for expr in exprs {
                if expr.line <= line && !available_exprs.contains(&expr.text) {
                    available_exprs.push(expr.text.clone());
                }
            }
        }

        let output = serde_json::json!({
            "line": line,
            "available_expressions": available_exprs,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_killed_by_query(
        &self,
        result: &AvailableExprsInfo,
        expr: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Find what variables kill this expression
        let mut killers = Vec::new();

        // Find the expression to get its operands
        for exprs in result.avail_in.values() {
            for e in exprs {
                if e.text == expr {
                    killers.extend(e.operands.iter().cloned());
                    break;
                }
            }
        }

        // Also check avail_out
        for exprs in result.avail_out.values() {
            for e in exprs {
                if e.text == expr {
                    for op in &e.operands {
                        if !killers.contains(op) {
                            killers.push(op.clone());
                        }
                    }
                    break;
                }
            }
        }

        let output = serde_json::json!({
            "expression": expr,
            "killed_by_redefinition_of": killers,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn format_text_output(&self, result: &AvailableExprsInfo) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "Available Expressions Analysis: {} in {}\n\n",
            self.function,
            self.file.display()
        ));

        // Show redundant computations (CSE opportunities)
        // redundant_computations returns Vec<(text, first_line, redundant_line)>
        let redundant = result.redundant_computations();
        if !redundant.is_empty() {
            output.push_str("CSE Opportunities (redundant computations):\n");
            for (expr_text, first_line, redundant_line) in &redundant {
                output.push_str(&format!(
                    "  - '{}' first at line {}, redundant at line {}\n",
                    expr_text, first_line, redundant_line
                ));
            }
            output.push('\n');
        } else {
            output.push_str("No redundant computations detected.\n\n");
        }

        // Show available expressions per block (unless --cse-only)
        if !self.cse_only {
            output.push_str("Available expressions by block:\n");
            let mut blocks: Vec<_> = result.avail_in.keys().collect();
            blocks.sort();

            for block_id in blocks {
                if let Some(exprs) = result.avail_in.get(block_id) {
                    if !exprs.is_empty() {
                        let expr_strs: Vec<_> = exprs.iter().map(|e| e.text.as_str()).collect();
                        output.push_str(&format!(
                            "  Block {}: {}\n",
                            block_id,
                            expr_strs.join(", ")
                        ));
                    }
                }
            }
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use tldr_core::dataflow::available::{
        AvailableExprsInfo, Confidence, ExprInstance, Expression,
    };

    /// Build an AvailableArgs with the given cse_only flag.
    fn make_args(cse_only: bool) -> AvailableArgs {
        AvailableArgs {
            file: PathBuf::from("test.py"),
            function: "example".to_string(),
            lang: None,
            check: None,
            at_line: None,
            killed_by: None,
            cse_only,
        }
    }

    /// Build an AvailableExprsInfo with one CSE opportunity and one block
    /// of available expressions.
    fn make_result_with_cse() -> AvailableExprsInfo {
        let expr_a = Expression::new("a + b", vec!["a", "b"], 2);
        let expr_b = Expression::new("c * d", vec!["c", "d"], 3);
        // Duplicate of expr_a at a later line -- triggers CSE
        let expr_a_dup = Expression::new("a + b", vec!["a", "b"], 4);

        let mut avail_in: HashMap<usize, HashSet<Expression>> = HashMap::new();
        let mut set = HashSet::new();
        set.insert(expr_a.clone());
        set.insert(expr_b.clone());
        avail_in.insert(0, set);

        let avail_out: HashMap<usize, HashSet<Expression>> = HashMap::new();

        let mut all_exprs = HashSet::new();
        all_exprs.insert(expr_a.clone());
        all_exprs.insert(expr_b.clone());

        // ExprInstance list that will trigger redundant_computations()
        let instances_with_blocks = vec![
            ExprInstance::new(expr_a.clone(), 0),
            ExprInstance::new(expr_b.clone(), 0),
            ExprInstance::new(expr_a_dup.clone(), 0),
        ];

        AvailableExprsInfo {
            avail_in,
            avail_out,
            all_exprs,
            entry_block: 0,
            expr_instances: vec![expr_a.clone(), expr_b.clone(), expr_a_dup.clone()],
            expr_instances_with_blocks: instances_with_blocks,
            defs_per_line: HashMap::new(),
            line_to_block: HashMap::new(),
            uncertain_exprs: Vec::new(),
            confidence: Confidence::High,
        }
    }

    #[test]
    fn test_cse_only_flag_hides_blocks() {
        let args = make_args(true);
        let result = make_result_with_cse();
        let output = args.format_text_output(&result);

        // CSE section should always be present
        assert!(
            output.contains("CSE Opportunities"),
            "CSE Opportunities section must be present with --cse-only. Got:\n{}",
            output,
        );

        // Per-block section should be hidden
        assert!(
            !output.contains("Available expressions by block:"),
            "Per-block section must be hidden with --cse-only. Got:\n{}",
            output,
        );
    }

    #[test]
    fn test_default_shows_blocks() {
        let args = make_args(false);
        let result = make_result_with_cse();
        let output = args.format_text_output(&result);

        // CSE section should be present
        assert!(
            output.contains("CSE Opportunities"),
            "CSE Opportunities section must be present by default. Got:\n{}",
            output,
        );

        // Per-block section should also be present
        assert!(
            output.contains("Available expressions by block:"),
            "Per-block section must be present by default. Got:\n{}",
            output,
        );
    }
}
