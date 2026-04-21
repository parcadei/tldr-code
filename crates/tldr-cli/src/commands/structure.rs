//! Structure command - Show code structure
//!
//! Extracts and displays functions, classes, and imports from source files.
//! Auto-routes through daemon when available for ~35x speedup.

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

use tldr_core::types::CodeStructure;
use tldr_core::{get_code_structure, IgnoreSpec, Language};

use crate::commands::daemon_router::{params_with_path_lang, try_daemon_route};
use crate::output::{format_structure_text, OutputFormat, OutputWriter};

/// Extract code structure (functions, classes, imports)
#[derive(Debug, Args)]
pub struct StructureArgs {
    /// Directory to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Programming language (auto-detected if not specified)
    #[arg(long, short = 'l')]
    pub lang: Option<Language>,

    /// Maximum number of files to process (0 = unlimited)
    #[arg(long, short = 'm', default_value = "0")]
    pub max_results: usize,
}

impl StructureArgs {
    /// Run the structure command
    pub fn run(&self, format: OutputFormat, quiet: bool) -> Result<()> {
        let writer = OutputWriter::new(format, quiet);

        // Determine language (auto-detect from directory, default to Python)
        let language = self
            .lang
            .unwrap_or_else(|| Language::from_directory(&self.path).unwrap_or(Language::Python));

        // Try daemon first for cached result
        if let Some(structure) = try_daemon_route::<CodeStructure>(
            &self.path,
            "structure",
            params_with_path_lang(&self.path, Some(language.as_str())),
        ) {
            // Output based on format
            if writer.is_text() {
                let text = format_structure_text(&structure);
                writer.write_text(&text)?;
                return Ok(());
            } else {
                writer.write(&structure)?;
                return Ok(());
            }
        }

        // Fallback to direct compute
        writer.progress(&format!(
            "Extracting structure from {} ({:?})...",
            self.path.display(),
            language
        ));

        // Get code structure
        let structure = get_code_structure(
            &self.path,
            language,
            self.max_results,
            Some(&IgnoreSpec::default()),
        )?;

        // Output based on format
        if writer.is_text() {
            let text = format_structure_text(&structure);
            writer.write_text(&text)?;
        } else {
            writer.write(&structure)?;
        }

        Ok(())
    }
}
