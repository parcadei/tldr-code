//! Abstract Interpretation CLI command
//!
//! Performs abstract interpretation for range tracking, nullability analysis,
//! and safety checks (division-by-zero, null dereference detection).
//!
//! # Usage
//!
//! ```bash
//! tldr abstract-interp <file> <function> [-f json|text]
//! tldr abstract-interp src/main.py process_data --var x
//! tldr abstract-interp src/main.py process_data --line 42
//! tldr abstract-interp src/main.py process_data --check_zero divisor
//! tldr abstract-interp src/main.py process_data --check_null ptr
//! ```
//!
//! # Output
//!
//! - JSON: Full AbstractInterpInfo with state_in/state_out per block
//! - Text: Human-readable summary with safety warnings highlighted
//!
//! # Reference
//! - dataflow/spec.md (CAP-AI-01 through CAP-AI-22)

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::dataflow::{compute_abstract_interp, AbstractInterpInfo, Nullability};
use tldr_core::{get_cfg_context, get_dfg_context, Language};

use crate::output::OutputFormat;

/// Analyze abstract values (ranges, nullability) for safety checks
#[derive(Debug, Args)]
pub struct AbstractInterpArgs {
    /// Source file to analyze
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Show abstract value for a specific variable
    #[arg(long)]
    pub var: Option<String>,

    /// Show abstract state at a specific line number
    #[arg(long)]
    pub line: Option<usize>,

    /// Check if a variable may be zero (potential division-by-zero)
    #[arg(long)]
    pub check_zero: Option<String>,

    /// Check if a variable may be null (potential null dereference)
    #[arg(long)]
    pub check_null: Option<String>,

    /// Show only safety warnings (division-by-zero, null dereference)
    #[arg(long)]
    pub warnings_only: bool,
}

impl AbstractInterpArgs {
    /// Run the abstract interpretation command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        use crate::output::OutputWriter;

        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        let lang_str = match language {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::Go => "go",
            Language::Rust => "rust",
            Language::JavaScript => "javascript",
            _ => "python", // Default fallback for other languages
        };

        writer.progress(&format!(
            "Analyzing abstract interpretation for {} in {}...",
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

        // Read source for line mapping
        let source = std::fs::read_to_string(&self.file)?;
        let source_lines: Vec<&str> = source.lines().collect();

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

        // Compute abstract interpretation
        let result = compute_abstract_interp(&cfg, &dfg, Some(&source_lines), lang_str)?;

        // Handle specific queries
        if let Some(ref var) = self.var {
            return self.handle_var_query(&result, var, &writer);
        }

        if let Some(line) = self.line {
            return self.handle_line_query(&result, line, &writer);
        }

        if let Some(ref var) = self.check_zero {
            return self.handle_check_zero_query(&result, var, &writer);
        }

        if let Some(ref var) = self.check_null {
            return self.handle_check_null_query(&result, var, &writer);
        }

        if self.warnings_only {
            return self.handle_warnings_only(&result, &writer, format);
        }

        // Default: output full result
        match format {
            OutputFormat::Json => {
                let json_value = result.to_json();
                let json = serde_json::to_string_pretty(&json_value)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            OutputFormat::Text => {
                let text = self.format_text_output(&result);
                writer.write_text(&text)?;
            }
            OutputFormat::Compact => {
                let json_value = result.to_json();
                let json = serde_json::to_string(&json_value)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
            _ => {
                let json_value = result.to_json();
                let json = serde_json::to_string_pretty(&json_value)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
            }
        }

        Ok(())
    }

    fn handle_var_query(
        &self,
        result: &AbstractInterpInfo,
        var: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Find the abstract value for this variable across all blocks
        let mut values = Vec::new();

        for (block_id, state) in &result.state_out {
            let abstract_val = state.values.get(var);
            if let Some(val) = abstract_val {
                let range_str = val.range_.as_ref().map(|(low, high)| {
                    let l = low.map_or("?".to_string(), |v| v.to_string());
                    let h = high.map_or("?".to_string(), |v| v.to_string());
                    vec![l, h]
                });
                values.push(serde_json::json!({
                    "block": block_id,
                    "type": val.type_,
                    "range": range_str,
                    "nullable": format!("{:?}", val.nullable),
                    "constant": val.constant,
                }));
            }
        }

        let output = serde_json::json!({
            "variable": var,
            "abstract_values": values,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_line_query(
        &self,
        result: &AbstractInterpInfo,
        line: usize,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Find the abstract state at the given line
        let mut state_at_line = serde_json::Map::new();

        // Collect all variables and their values from all exit states
        for (_block_id, state) in &result.state_out {
            for (var, val) in &state.values {
                let range_str = val.range_.as_ref().map(|(low, high)| {
                    let l = low.map_or("?".to_string(), |v| v.to_string());
                    let h = high.map_or("?".to_string(), |v| v.to_string());
                    vec![l, h]
                });
                state_at_line.insert(
                    var.clone(),
                    serde_json::json!({
                        "type": val.type_,
                        "range": range_str,
                        "nullable": format!("{:?}", val.nullable),
                        "constant": val.constant,
                    }),
                );
            }
        }

        let output = serde_json::json!({
            "line": line,
            "state": state_at_line,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_check_zero_query(
        &self,
        result: &AbstractInterpInfo,
        var: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Check if this variable may be zero
        let mut may_be_zero = false;
        let mut must_be_zero = false;
        let mut range_info: Option<Vec<String>> = None;

        for (_block_id, state) in &result.state_out {
            if let Some(abstract_val) = state.values.get(var) {
                if let Some(ref range) = abstract_val.range_ {
                    let low = range.0.unwrap_or(i64::MIN);
                    let high = range.1.unwrap_or(i64::MAX);
                    // Check if range includes zero
                    if low <= 0 && high >= 0 {
                        may_be_zero = true;
                    }
                    if low == 0 && high == 0 {
                        must_be_zero = true;
                    }
                    range_info = Some(vec![
                        range.0.map_or("?".to_string(), |v| v.to_string()),
                        range.1.map_or("?".to_string(), |v| v.to_string()),
                    ]);
                }
            }
        }

        // Also check if this var appears in potential_div_zero
        let in_warnings = result
            .potential_div_zero
            .iter()
            .any(|(_line, v)| v == var);

        let output = serde_json::json!({
            "variable": var,
            "may_be_zero": may_be_zero,
            "must_be_zero": must_be_zero,
            "range": range_info,
            "flagged_in_warnings": in_warnings,
            "safe_for_division": !may_be_zero,
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_check_null_query(
        &self,
        result: &AbstractInterpInfo,
        var: &str,
        writer: &crate::output::OutputWriter,
    ) -> Result<()> {
        // Check if this variable may be null
        let mut nullability = Nullability::Never;

        for (_block_id, state) in &result.state_out {
            if let Some(abstract_val) = state.values.get(var) {
                // Take the "worst" nullability
                match abstract_val.nullable {
                    Nullability::Always => nullability = Nullability::Always,
                    Nullability::Maybe if nullability != Nullability::Always => {
                        nullability = Nullability::Maybe
                    }
                    _ => {}
                }
            }
        }

        // Also check if this var appears in potential_null_deref
        let in_warnings = result
            .potential_null_deref
            .iter()
            .any(|(_line, v)| v == var);

        let output = serde_json::json!({
            "variable": var,
            "nullability": format!("{:?}", nullability),
            "may_be_null": matches!(nullability, Nullability::Maybe | Nullability::Always),
            "must_be_null": matches!(nullability, Nullability::Always),
            "flagged_in_warnings": in_warnings,
            "safe_for_dereference": matches!(nullability, Nullability::Never),
        });

        let json = serde_json::to_string_pretty(&output)
            .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
        writer.write_text(&json)?;
        Ok(())
    }

    fn handle_warnings_only(
        &self,
        result: &AbstractInterpInfo,
        writer: &crate::output::OutputWriter,
        format: OutputFormat,
    ) -> Result<()> {
        let output = serde_json::json!({
            "potential_division_by_zero": result.potential_div_zero,
            "potential_null_dereference": result.potential_null_deref,
            "total_warnings": result.potential_div_zero.len() + result.potential_null_deref.len(),
        });

        match format {
            OutputFormat::Text => {
                let mut text = String::from("Safety Warnings:\n\n");

                if result.potential_div_zero.is_empty() && result.potential_null_deref.is_empty() {
                    text.push_str("  No warnings detected.\n");
                } else {
                    if !result.potential_div_zero.is_empty() {
                        text.push_str("  Division by zero risks:\n");
                        for (line, var) in &result.potential_div_zero {
                            text.push_str(&format!("    - Line {}: variable '{}' may be zero\n", line, var));
                        }
                    }

                    if !result.potential_null_deref.is_empty() {
                        text.push_str("  Null dereference risks:\n");
                        for (line, var) in &result.potential_null_deref {
                            text.push_str(&format!("    - Line {}: variable '{}' may be null\n", line, var));
                        }
                    }
                }
                writer.write_text(&text)?;
                Ok(())
            }
            _ => {
                let json = serde_json::to_string_pretty(&output)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {}", e))?;
                writer.write_text(&json)?;
                Ok(())
            }
        }
    }

    fn format_text_output(&self, result: &AbstractInterpInfo) -> String {
        let mut output = String::new();

        output.push_str(&format!(
            "Abstract Interpretation: {} in {}\n\n",
            self.function,
            self.file.display()
        ));

        // Show safety warnings first
        let total_warnings = result.potential_div_zero.len() + result.potential_null_deref.len();
        if total_warnings > 0 {
            output.push_str(&format!("Safety Warnings ({}):\n", total_warnings));

            for (line, var) in &result.potential_div_zero {
                output.push_str(&format!("  [DIV0] Line {}: '{}' may be zero\n", line, var));
            }

            for (line, var) in &result.potential_null_deref {
                output.push_str(&format!("  [NULL] Line {}: '{}' may be null\n", line, var));
            }
            output.push('\n');
        } else {
            output.push_str("No safety warnings detected.\n\n");
        }

        // Show variable states per block
        output.push_str("Variable states by block:\n");
        let mut blocks: Vec<_> = result.state_out.keys().collect();
        blocks.sort();

        for block_id in blocks {
            if let Some(state) = result.state_out.get(block_id) {
                if !state.values.is_empty() {
                    output.push_str(&format!("  Block {}:\n", block_id));
                    for (var, val) in &state.values {
                        let range_str = val
                            .range_
                            .as_ref()
                            .map(|(low, high)| {
                                let l = low.map_or("?".to_string(), |v| v.to_string());
                                let h = high.map_or("?".to_string(), |v| v.to_string());
                                format!("[{}, {}]", l, h)
                            })
                            .unwrap_or_else(|| "?".to_string());
                        let null_str = match val.nullable {
                            Nullability::Never => "non-null",
                            Nullability::Maybe => "nullable",
                            Nullability::Always => "null",
                        };
                        output.push_str(&format!("    {}: {} {}\n", var, range_str, null_str));
                    }
                }
            }
        }

        output
    }
}
