//! Slice command - Program slicing
//!
//! Computes backward or forward program slices from a line.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};

use tldr_core::{get_slice_rich, Language, SliceDirection};

use crate::commands::daemon_router::{params_with_file_function_line, try_daemon_route};
use crate::output::{OutputFormat, OutputWriter};

/// Compute program slice from a line
#[derive(Debug, Args)]
pub struct SliceArgs {
    /// Source file path
    pub file: PathBuf,

    /// Function name containing the line
    pub function: String,

    /// Line number to slice from
    pub line: u32,

    /// Slice direction: backward (what affects this line) or forward (what this line affects)
    #[arg(long, short = 'd', default_value = "backward")]
    pub direction: SliceDirectionArg,

    /// Variable to filter by (optional - traces all if not specified)
    #[arg(long)]
    pub variable: Option<String>,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

/// CLI wrapper for slice direction
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum SliceDirectionArg {
    /// Backward slice - what affects this line?
    #[default]
    Backward,
    /// Forward slice - what does this line affect?
    Forward,
}

impl From<SliceDirectionArg> for SliceDirection {
    fn from(arg: SliceDirectionArg) -> Self {
        match arg {
            SliceDirectionArg::Backward => SliceDirection::Backward,
            SliceDirectionArg::Forward => SliceDirection::Forward,
        }
    }
}

/// Rich slice line for output
#[derive(Debug, Serialize, Deserialize)]
struct SliceLine {
    line: u32,
    code: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    definitions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    uses: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dep_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dep_label: Option<String>,
}

/// Edge in slice output
#[derive(Debug, Serialize, Deserialize)]
struct SliceEdgeOutput {
    from_line: u32,
    to_line: u32,
    dep_type: String,
    label: String,
}

/// Slice result output format (backward-compatible: keeps `lines` as Vec<u32>)
#[derive(Debug, Serialize, Deserialize)]
struct SliceOutput {
    file: PathBuf,
    function: String,
    criterion_line: u32,
    direction: String,
    variable: Option<String>,
    /// Bare line numbers (backward-compatible)
    lines: Vec<u32>,
    /// Rich line data with code and metadata
    #[serde(skip_serializing_if = "Vec::is_empty")]
    slice_lines: Vec<SliceLine>,
    /// Dependency edges within the slice
    #[serde(skip_serializing_if = "Vec::is_empty")]
    edges: Vec<SliceEdgeOutput>,
    line_count: usize,
}

/// Legacy daemon output (old format without rich data)
#[derive(Debug, Serialize, Deserialize)]
struct LegacySliceOutput {
    file: PathBuf,
    function: String,
    criterion_line: u32,
    direction: String,
    variable: Option<String>,
    lines: Vec<u32>,
    line_count: usize,
}

impl SliceArgs {
    /// Run the slice command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_path(&self.file).unwrap_or(Language::Python));

        let direction: SliceDirection = self.direction.into();
        let direction_str = match direction {
            SliceDirection::Backward => "backward",
            SliceDirection::Forward => "forward",
        };

        // Try daemon first for cached result (use file's parent as project root)
        let project = self.file.parent().unwrap_or(&self.file);
        if let Some(output) = try_daemon_route::<LegacySliceOutput>(
            project,
            "slice",
            params_with_file_function_line(&self.file, &self.function, self.line),
        ) {
            // Daemon returns legacy format -- enrich with source code if possible
            let source_lines = read_file_lines(&self.file);
            if writer.is_text() {
                let mut text = String::new();
                text.push_str(&format!(
                    "Program Slice ({} from line {})\n",
                    output.direction, output.criterion_line
                ));
                text.push_str(&format!(
                    "Function: {}::{}\n",
                    output.file.display(),
                    output.function
                ));
                if let Some(var) = &output.variable {
                    text.push_str(&format!("Variable: {}\n", var));
                }
                text.push_str(&format!(
                    "\nSlice contains {} lines:\n\n",
                    output.lines.len()
                ));
                for &line_num in &output.lines {
                    let code = source_lines
                        .get((line_num as usize).wrapping_sub(1))
                        .map(|s| s.trim_end())
                        .unwrap_or("");
                    let marker = if line_num == output.criterion_line {
                        ">"
                    } else {
                        " "
                    };
                    let criterion_flag = if line_num == output.criterion_line {
                        "  <-- criterion"
                    } else {
                        ""
                    };
                    text.push_str(&format!(
                        "{} {:>5} | {}{}\n",
                        marker, line_num, code, criterion_flag
                    ));
                }
                writer.write_text(&text)?;
                return Ok(());
            } else {
                // Convert legacy to new format for JSON
                let slice_lines: Vec<SliceLine> = output
                    .lines
                    .iter()
                    .map(|&l| {
                        let code = source_lines
                            .get((l as usize).wrapping_sub(1))
                            .map(|s| s.trim_end().to_string())
                            .unwrap_or_default();
                        SliceLine {
                            line: l,
                            code,
                            definitions: Vec::new(),
                            uses: Vec::new(),
                            dep_type: None,
                            dep_label: None,
                        }
                    })
                    .collect();
                let rich_output = SliceOutput {
                    file: output.file,
                    function: output.function,
                    criterion_line: output.criterion_line,
                    direction: output.direction,
                    variable: output.variable,
                    line_count: output.line_count,
                    lines: output.lines,
                    slice_lines,
                    edges: Vec::new(),
                };
                writer.write(&rich_output)?;
                return Ok(());
            }
        }

        // Fallback to direct compute with rich output
        writer.progress(&format!(
            "Computing {} slice for line {} in {}::{}...",
            direction_str,
            self.line,
            self.file.display(),
            self.function
        ));

        // Get rich slice
        let rich = get_slice_rich(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            self.line,
            direction,
            self.variable.as_deref(),
            language,
        )?;

        // Build backward-compatible line list
        let lines: Vec<u32> = rich.nodes.iter().map(|n| n.line).collect();

        // Build rich line data
        let slice_lines: Vec<SliceLine> = rich
            .nodes
            .iter()
            .map(|n| SliceLine {
                line: n.line,
                code: n.code.clone(),
                definitions: n.definitions.clone(),
                uses: n.uses.clone(),
                dep_type: n.dep_type.clone(),
                dep_label: n.dep_label.clone(),
            })
            .collect();

        // Build edge output
        let edges: Vec<SliceEdgeOutput> = rich
            .edges
            .iter()
            .map(|e| SliceEdgeOutput {
                from_line: e.from_line,
                to_line: e.to_line,
                dep_type: e.dep_type.clone(),
                label: e.label.clone(),
            })
            .collect();

        let data_count = edges.iter().filter(|e| e.dep_type == "data").count();
        let ctrl_count = edges.iter().filter(|e| e.dep_type == "control").count();

        let output = SliceOutput {
            file: self.file.clone(),
            function: self.function.clone(),
            criterion_line: self.line,
            direction: direction_str.to_string(),
            variable: self.variable.clone(),
            line_count: lines.len(),
            lines,
            slice_lines,
            edges,
        };

        // Output based on format
        if writer.is_text() {
            let text = format_rich_text(&output, data_count, ctrl_count);
            writer.write_text(&text)?;
        } else {
            writer.write(&output)?;
        }

        Ok(())
    }
}

/// Format rich slice as compact text for LLM consumption
fn format_rich_text(output: &SliceOutput, data_count: usize, ctrl_count: usize) -> String {
    let mut text = String::new();

    text.push_str(&format!(
        "Program Slice ({} from line {})\n",
        output.direction, output.criterion_line
    ));
    text.push_str(&format!(
        "Function: {}::{}\n",
        output.file.display(),
        output.function
    ));
    if let Some(var) = &output.variable {
        text.push_str(&format!("Variable: {}\n", var));
    }

    // Count non-blank lines for accurate summary
    let non_blank_count = output
        .slice_lines
        .iter()
        .filter(|sl| !sl.code.trim().is_empty())
        .count();

    // Summary line with dep counts
    if data_count > 0 || ctrl_count > 0 {
        text.push_str(&format!(
            "\nSlice contains {} lines ({} data deps, {} control deps):\n\n",
            non_blank_count, data_count, ctrl_count
        ));
    } else {
        text.push_str(&format!("\nSlice contains {} lines:\n\n", non_blank_count));
    }

    // Code lines with annotations
    // Track previous defs/uses to avoid repeating identical annotations
    // (PDG nodes span multiple lines but carry one set of defs/uses)
    let mut prev_defs: Option<&Vec<String>> = None;
    let mut prev_uses: Option<&Vec<String>> = None;

    for sl in &output.slice_lines {
        // Skip blank lines — they waste tokens and carry no insight
        if sl.code.trim().is_empty() {
            continue;
        }

        let marker = if sl.line == output.criterion_line {
            ">"
        } else {
            " "
        };

        // Only show defs/uses on the first line of each node span
        let same_as_prev = prev_defs == Some(&sl.definitions) && prev_uses == Some(&sl.uses);

        let mut annotations = Vec::new();
        if !same_as_prev {
            if !sl.definitions.is_empty() {
                annotations.push(format!("[defines: {}]", sl.definitions.join(", ")));
            }
            if !sl.uses.is_empty() {
                annotations.push(format!("[uses: {}]", sl.uses.join(", ")));
            }
        }
        if let Some(dt) = &sl.dep_type {
            if dt == "control" && !same_as_prev {
                annotations.push("ctrl".to_string());
            }
        }

        prev_defs = Some(&sl.definitions);
        prev_uses = Some(&sl.uses);

        let criterion_flag = if sl.line == output.criterion_line {
            "  <-- criterion"
        } else {
            ""
        };

        let annotation_str = if annotations.is_empty() {
            String::new()
        } else {
            format!("     {}", annotations.join(" "))
        };

        text.push_str(&format!(
            "{} {:>5} | {}{}{}\n",
            marker, sl.line, sl.code, annotation_str, criterion_flag
        ));
    }

    // Dependencies section
    if !output.edges.is_empty() {
        text.push_str("\nDependencies:\n");
        for edge in &output.edges {
            if edge.dep_type == "data" && !edge.label.is_empty() {
                text.push_str(&format!(
                    "  {}@{} <- {}@{} (data: {})\n",
                    edge.label, edge.to_line, edge.label, edge.from_line, edge.label
                ));
            } else {
                text.push_str(&format!(
                    "  {} <- {} ({})\n",
                    edge.to_line, edge.from_line, edge.dep_type
                ));
            }
        }
    }

    text
}

/// Read file lines for source enrichment
fn read_file_lines(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|c| c.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}
