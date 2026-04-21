//! CFG command - Show control flow graph
//!
//! Extracts and displays the control flow graph for a function.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::{get_cfg_context, Language};
use tldr_core::types::CfgInfo;

use crate::commands::daemon_router::{params_with_file_function, try_daemon_route};
use crate::output::{format_cfg_text, OutputFormat, OutputWriter};

/// Extract control flow graph for a function
#[derive(Debug, Args)]
pub struct CfgArgs {
    /// Source file path
    pub file: PathBuf,

    /// Function name to analyze
    pub function: String,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl CfgArgs {
    /// Run the cfg command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language from file extension or argument
        let language = self.lang.unwrap_or_else(|| {
            Language::from_path(&self.file).unwrap_or(Language::Python)
        });

        // Try daemon first for cached result (use file's parent as project root)
        let project = self.file.parent().unwrap_or(&self.file);
        if let Some(cfg) = try_daemon_route::<CfgInfo>(
            project,
            "cfg",
            params_with_file_function(&self.file, &self.function),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_cfg_text(&cfg);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&cfg)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Extracting CFG for {} in {}...",
            self.function,
            self.file.display()
        ));

        // Get CFG
        let cfg = get_cfg_context(
            self.file.to_str().unwrap_or_default(),
            &self.function,
            language,
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_cfg_text(&cfg);
            writer.write_text(&text)?;
        } else {
            writer.write(&cfg)?;
        }

        Ok(())
    }
}
