//! Extract command - Extract complete module info from a file
//!
//! Returns functions, classes, imports, and call graph for a single file.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::types::ModuleInfo;
use tldr_core::{extract_file, Language};

use crate::commands::daemon_router::{params_with_file, try_daemon_route};
use crate::output::{format_module_info_text, OutputFormat, OutputWriter};

/// Extract complete module info from a file
#[derive(Debug, Args)]
pub struct ExtractArgs {
    /// File to extract
    pub file: PathBuf,

    /// Programming language (auto-detected from file extension if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl ExtractArgs {
    /// Run the extract command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result (use file's parent as project root)
        let project = self.file.parent().unwrap_or(&self.file);
        if let Some(result) =
            try_daemon_route::<ModuleInfo>(project, "extract", params_with_file(&self.file))
        {
            if writer.is_text() {
                writer.write_text(&format_module_info_text(&result))?;
            } else {
                writer.write(&result)?;
            }
            return Ok(());
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Extracting module info from {}...",
            self.file.display()
        ));

        // Extract module info - the core function handles language detection
        let result = extract_file(&self.file, None)?;

        // Output based on format
        if writer.is_text() {
            writer.write_text(&format_module_info_text(&result))?;
        } else {
            writer.write(&result)?;
        }

        Ok(())
    }
}
