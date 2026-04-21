//! Imports command - Parse import statements from a file
//!
//! Returns an array of ImportInfo objects with module, names, is_from, and alias.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use colored::Colorize;

use tldr_core::types::ImportInfo;
use tldr_core::{detect_or_parse_language, get_imports, Language};

use crate::commands::daemon_router::{params_with_file, try_daemon_route};
use crate::output::{format_imports_text, OutputFormat, OutputWriter};

/// Parse import statements from a file
#[derive(Debug, Args)]
pub struct ImportsArgs {
    /// File to parse
    pub file: PathBuf,

    /// Programming language (auto-detect if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,
}

impl ImportsArgs {
    /// Run the imports command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Try daemon first for cached result (use file's parent as project root)
        let project = self.file.parent().unwrap_or(&self.file);
        if let Some(result) =
            try_daemon_route::<Vec<ImportInfo>>(project, "imports", params_with_file(&self.file))
        {
            if writer.is_text() {
                writer.write_text(&format!(
                    "{} ({} imports)\n\n{}",
                    self.file.display().to_string().bold(),
                    result.len(),
                    format_imports_text(&result),
                ))?;
            } else {
                writer.write(&result)?;
            }
            return Ok(());
        }

        // Fallback to direct compute

        // Detect or parse language (uses shared validator M28)
        let language =
            detect_or_parse_language(self.lang.as_ref().map(|l| l.as_str()), &self.file)?;

        writer.progress(&format!(
            "Parsing imports from {} ({:?})...",
            self.file.display(),
            language
        ));

        // Get imports
        let result = get_imports(&self.file, language)?;

        // Output based on format
        if writer.is_text() {
            writer.write_text(&format!(
                "{} ({} imports)\n\n{}",
                self.file.display().to_string().bold(),
                result.len(),
                format_imports_text(&result),
            ))?;
        } else {
            writer.write(&result)?;
        }

        Ok(())
    }
}
