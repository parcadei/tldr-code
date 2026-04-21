//! DFG command - Show data flow graph
//!
//! Extracts and displays the data flow graph with def-use chains.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{get_dfg_context, Language};
use tldr_core::types::DfgInfo;

use crate::commands::daemon_router::{params_with_file_function, try_daemon_route};
use crate::output::{format_dfg_text, OutputFormat, OutputWriter};

/// Extract data flow graph for a function
#[derive(Debug, Args)]
pub struct DfgArgs {
    /// Source file path
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl DfgArgs {
    /// Run the dfg command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        // Try daemon first for cached result (use file's parent as project root)
        let project = self.file.parent().unwrap_or(&self.file);
        if let Some(dfg) = try_daemon_route::<DfgInfo>(
            project,
            "dfg",
            params_with_file_function(&self.file, &self.function),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_dfg_text(&dfg);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&dfg)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Extracting DFG for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Get DFG
        let dfg = get_dfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_dfg_text(&dfg);
            writer.write_text(&text)?;
        } else {
            writer.write(&dfg)?;
        }

        Ok(())
    }
}
